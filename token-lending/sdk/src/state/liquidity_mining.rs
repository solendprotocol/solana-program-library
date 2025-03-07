use crate::{
    error::LendingError,
    math::{Decimal, TryAdd, TryDiv, TryMul, TrySub},
    state::unpack_decimal,
};
use arrayref::{array_mut_ref, array_ref, array_refs, mut_array_refs};
use solana_program::program_pack::{Pack, Sealed};
use solana_program::{
    clock::Clock,
    program_error::ProgramError,
    pubkey::{Pubkey, PUBKEY_BYTES},
};

use super::pack_decimal;

/// Determines the size of [PoolRewardManager]
const MAX_REWARDS: usize = 50;

/// Cannot create a reward shorter than this.
pub const MIN_REWARD_PERIOD_SECS: u64 = 3_600;

/// Each reserve has two managers:
/// - one for deposits
/// - one for borrows
#[derive(Clone, Debug, PartialEq)]
pub struct PoolRewardManager {
    /// Is updated when we change user shares in the reserve.
    pub total_shares: u64,
    /// Monotonically increasing time taken from clock sysvar.
    pub last_update_time_secs: u64,
    /// New [PoolReward] are added to the first vacant slot.
    pub pool_rewards: [PoolRewardSlot; MAX_REWARDS],
}

/// Each pool reward gets an ID which is monotonically increasing with each
/// new reward added to the pool at the particular slot.
///
/// This helps us distinguish between two distinct rewards in the same array
/// index across time.
///
/// # Wrapping
/// There are two strategies to handle wrapping:
/// 1. Consider the associated slot locked forever
/// 2. Go back to 0.
///
/// Given that one reward lasts at [MIN_REWARD_PERIOD_SECS] we've got at least
/// half a million years before we need to worry about wrapping in a single slot.
/// I'd call that someone else's problem.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PoolRewardId(pub u32);

/// # (Un)Packing
/// This is unpacked representation.
/// When packing we use the [PoolReward] `reward_mint` to determine whether the
/// reward is vacant or not to save space.
///
/// If the pubkey is eq to default pubkey then slot is vacant.
#[derive(Clone, Debug, PartialEq)]
pub enum PoolRewardSlot {
    /// New reward can be added to this slot.
    Vacant {
        /// Increment this ID when adding new [PoolReward].
        last_pool_reward_id: PoolRewardId,
    },
    /// Reward has not been closed yet.
    Occupied(PoolReward),
}

/// Tracks rewards in a specific mint over some period of time.
///
/// # Reward cancellation
/// In Suilend we also store the amount of rewards that have been made available
/// to users already.
/// We keep adding `(total_rewards * time_passed) / (total_time)` every
/// time someone interacts with the manager.
/// This value is used to transfer the unallocated rewards to the admin.
/// However, this can be calculated dynamically which avoids storing extra
/// [Decimal] on each [PoolReward].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PoolReward {
    /// Unique ID for this slot that has never been used before, and will never
    /// be used again.
    pub id: PoolRewardId,
    /// # (Un)Packing
    /// When we pack the reward we set this to default pubkey for vacant slots.
    pub vault: Pubkey,
    /// Monotonically increasing time taken from clock sysvar.
    pub start_time_secs: u64,
    /// For how long (since start time) will this reward be releasing tokens.
    pub duration_secs: u32,
    /// Total token amount to distribute.
    /// The token account that holds the rewards holds at least this much in
    /// the beginning.
    pub total_rewards: u64,
    /// How many users are still tracking this reward.
    /// Once this reaches zero we can close this reward.
    /// There's a permission-less ix with which user rewards can be distributed
    /// that's used for cranking remaining rewards.
    pub num_user_reward_managers: u64,
    /// We keep adding `(unlocked_rewards) / (total_shares)` every time
    /// someone interacts with the manager ([update_pool_reward_manager])
    /// where
    /// `unlocked_rewards = (total_rewards * time_passed) / (total_time)`
    ///
    /// # (Un)Packing
    /// We only store 16 most significant digits.
    pub cumulative_rewards_per_share: Decimal,
}

/// Tracks user's LM rewards for a specific pool (reserve.)
pub struct UserRewardManager {
    /// User cannot both borrow and deposit in the same reserve.
    /// This manager is unique for this reserve within the [Obligation].
    ///
    /// We know whether to use [crate::state::Reserve]'s
    /// `deposits_pool_reward_manager` or `borrows_pool_reward_manager` based on
    /// this field.
    ///
    /// One optimization we could make is to link the [UserRewardManager] via
    /// index which would save 32 bytes per [UserRewardManager].
    /// However, that does make the program logic more error prone.
    pub reserve: Pubkey,
    /// For deposits, this is the amount of collateral token user has in
    /// their obligation deposit.
    ///
    /// For borrows, this is (borrow_amount / cumulative_borrow_rate) user
    /// has in their obligation borrow.
    pub share: u64,
    /// Monotonically increasing time taken from clock sysvar.
    pub last_update_time_secs: u64,
    /// The index of each reward is important.
    /// It will match the index in the [PoolRewardManager] of the reserve.
    pub rewards: Vec<Option<UserReward>>,
}

/// Track user rewards for a specific [PoolReward].
pub struct UserReward {
    /// Each pool reward gets an ID which is monotonically increasing with each
    /// new reward added to the pool.
    pub pool_reward_id: PoolRewardId,
    /// Before [UserReward.cumulative_rewards_per_share] is copied we find
    /// time difference between current global rewards and last user update
    /// rewards:
    /// [PoolReward.cumulative_rewards_per_share] - [UserReward.cumulative_rewards_per_share]
    ///
    /// Then, we multiply that difference by [UserRewardManager.share] and
    /// add the result to this counter.
    pub earned_rewards: Decimal,
    /// copied from [PoolReward.cumulative_rewards_per_share] at the time of the last update
    pub cumulative_rewards_per_share: Decimal,
}

impl PoolRewardManager {
    /// Should be updated before any interaction with rewards.
    fn update(&mut self, clock: &Clock) -> Result<(), ProgramError> {
        let curr_unix_timestamp_secs = clock.unix_timestamp as u64;

        if self.last_update_time_secs >= curr_unix_timestamp_secs {
            return Ok(());
        }

        if self.total_shares == 0 {
            self.last_update_time_secs = curr_unix_timestamp_secs;
            return Ok(());
        }

        let last_update_time_secs = self.last_update_time_secs;

        // get rewards that started already and did not finish yet
        let running_rewards = self
            .pool_rewards
            .iter_mut()
            .filter_map(|r| match r {
                PoolRewardSlot::Occupied(reward) => Some(reward),
                _ => None,
            })
            .filter(|r| curr_unix_timestamp_secs > r.start_time_secs)
            .filter(|r| last_update_time_secs < (r.start_time_secs + r.duration_secs as u64));

        for reward in running_rewards {
            let end_time_secs = reward.start_time_secs + reward.duration_secs as u64;
            let time_passed_secs = curr_unix_timestamp_secs
                .min(end_time_secs)
                .checked_sub(reward.start_time_secs.max(last_update_time_secs))
                .ok_or(LendingError::MathOverflow)?;

            // When adding a reward we assert that a reward lasts for at least [MIN_REWARD_PERIOD_SECS].
            // Hence this won't error on overflow nor on division by zero.
            let unlocked_rewards = Decimal::from(reward.total_rewards)
                .try_mul(Decimal::from(time_passed_secs))?
                .try_div(Decimal::from(end_time_secs - reward.start_time_secs))?;

            reward.cumulative_rewards_per_share = reward
                .cumulative_rewards_per_share
                .try_add(unlocked_rewards.try_div(Decimal::from(self.total_shares))?)?;
        }

        self.last_update_time_secs = curr_unix_timestamp_secs;

        Ok(())
    }
}

enum CreatingNewUserRewardManager {
    /// If we are creating a [UserRewardManager] then we want to populate it.
    Yes,
    No,
}

impl UserRewardManager {
    /// Should be updated before any interaction with rewards.
    ///
    /// # Assumption
    /// Invoker has checked that this [PoolRewardManager] matches the
    /// [UserRewardManager].
    fn update(
        &mut self,
        pool_reward_manager: &mut PoolRewardManager,
        clock: &Clock,
        creating_new_reward_manager: CreatingNewUserRewardManager,
    ) -> Result<(), ProgramError> {
        pool_reward_manager.update(clock)?;

        let curr_unix_timestamp_secs = clock.unix_timestamp as u64;

        if matches!(
            creating_new_reward_manager,
            CreatingNewUserRewardManager::No
        ) && curr_unix_timestamp_secs == self.last_update_time_secs
        {
            return Ok(());
        }

        self.rewards
            .resize_with(pool_reward_manager.pool_rewards.len(), || None);

        for (reward_index, pool_reward) in pool_reward_manager.pool_rewards.iter_mut().enumerate() {
            let PoolRewardSlot::Occupied(pool_reward) = pool_reward else {
                // no reward to track
                continue;
            };

            let end_time_secs = pool_reward.start_time_secs + pool_reward.duration_secs as u64;

            match self.rewards.get_mut(reward_index) {
                None => unreachable!("We've just resized the rewards."),
                Some(None) if self.last_update_time_secs > end_time_secs => {
                    // reward period ended, skip
                }
                Some(None) => {
                    // user did not yet start accruing rewards

                    let new_user_reward = UserReward {
                        pool_reward_id: pool_reward.id,
                        cumulative_rewards_per_share: pool_reward.cumulative_rewards_per_share,
                        earned_rewards: if self.last_update_time_secs <= pool_reward.start_time_secs
                        {
                            pool_reward
                                .cumulative_rewards_per_share
                                .try_mul(Decimal::from(self.share))?
                        } else {
                            debug_assert!(matches!(
                                creating_new_reward_manager,
                                CreatingNewUserRewardManager::Yes
                            ));
                            Decimal::zero()
                        },
                    };

                    // we resized this vector to match the pool rewards
                    self.rewards[reward_index] = Some(new_user_reward);

                    pool_reward.num_user_reward_managers += 1;
                }
                Some(Some(user_reward)) => {
                    // user is already accruing rewards, add the difference

                    let new_reward_amount = pool_reward
                        .cumulative_rewards_per_share
                        .try_sub(user_reward.cumulative_rewards_per_share)?
                        .try_mul(Decimal::from(self.share))?;

                    user_reward.earned_rewards =
                        user_reward.earned_rewards.try_add(new_reward_amount)?;

                    user_reward.cumulative_rewards_per_share =
                        pool_reward.cumulative_rewards_per_share;
                }
            }
        }

        self.last_update_time_secs = curr_unix_timestamp_secs;

        Ok(())
    }
}

impl PoolReward {
    const LEN: usize = PoolRewardId::LEN + PUBKEY_BYTES + 8 + 4 + 8 + 8 + 16;
}

impl PoolRewardId {
    const LEN: usize = std::mem::size_of::<Self>();
}

impl Default for PoolRewardManager {
    fn default() -> Self {
        Self {
            total_shares: 0,
            last_update_time_secs: 0,
            pool_rewards: std::array::from_fn(|_| PoolRewardSlot::default()),
        }
    }
}

impl Default for PoolRewardSlot {
    fn default() -> Self {
        Self::Vacant {
            last_pool_reward_id: PoolRewardId(0),
        }
    }
}

impl Sealed for PoolRewardManager {}

impl Pack for PoolRewardManager {
    /// total_shares + last_update_time_secs + pool_rewards.
    const LEN: usize = 8 + 8 + MAX_REWARDS * PoolReward::LEN;

    fn pack_into_slice(&self, output: &mut [u8]) {
        output[0..8].copy_from_slice(&self.total_shares.to_le_bytes());
        output[8..16].copy_from_slice(&self.last_update_time_secs.to_le_bytes());

        for (index, pool_reward_slot) in self.pool_rewards.iter().enumerate() {
            let offset = 16 + index * PoolReward::LEN;
            let raw_pool_reward = array_mut_ref![output, offset, PoolReward::LEN];

            let (
                dst_id,
                dst_vault,
                dst_start_time_secs,
                dst_duration_secs,
                dst_total_rewards,
                dst_num_user_reward_managers,
                dst_cumulative_rewards_per_share_wads,
            ) = mut_array_refs![
                raw_pool_reward,
                PoolRewardId::LEN,
                PUBKEY_BYTES,
                8,  // start_time_secs
                4,  // duration_secs
                8,  // total_rewards
                8,  // num_user_reward_managers
                16  // cumulative_rewards_per_share
            ];

            let (
                id,
                vault,
                start_time_secs,
                duration_secs,
                total_rewards,
                num_user_reward_managers,
                cumulative_rewards_per_share,
            ) = match pool_reward_slot {
                PoolRewardSlot::Vacant {
                    last_pool_reward_id,
                } => (
                    last_pool_reward_id,
                    Pubkey::default(),
                    0u64,
                    0u32,
                    0u64,
                    0u64,
                    Decimal::zero(),
                ),
                PoolRewardSlot::Occupied(PoolReward {
                    id,
                    vault,
                    start_time_secs,
                    duration_secs,
                    total_rewards,
                    num_user_reward_managers,
                    cumulative_rewards_per_share,
                }) => (
                    id,
                    *vault,
                    *start_time_secs,
                    *duration_secs,
                    *total_rewards,
                    *num_user_reward_managers,
                    *cumulative_rewards_per_share,
                ),
            };

            dst_id.copy_from_slice(&id.0.to_le_bytes());
            dst_vault.copy_from_slice(vault.as_ref());
            *dst_start_time_secs = start_time_secs.to_le_bytes();
            *dst_duration_secs = duration_secs.to_le_bytes();
            *dst_total_rewards = total_rewards.to_le_bytes();
            *dst_num_user_reward_managers = num_user_reward_managers.to_le_bytes();
            pack_decimal(
                cumulative_rewards_per_share,
                dst_cumulative_rewards_per_share_wads,
            );
        }
    }

    fn unpack_from_slice(input: &[u8]) -> Result<Self, ProgramError> {
        let mut pool_reward_manager = PoolRewardManager {
            total_shares: u64::from_le_bytes(*array_ref![input, 0, 8]),
            last_update_time_secs: u64::from_le_bytes(*array_ref![input, 8, 8]),
            ..Default::default()
        };

        for index in 0..MAX_REWARDS {
            let offset = 8 + 8 + index * PoolReward::LEN;
            let raw_pool_reward = array_ref![input, offset, PoolReward::LEN];

            let (
                src_id,
                src_vault,
                src_start_time_secs,
                src_duration_secs,
                src_total_rewards,
                src_num_user_reward_managers,
                src_cumulative_rewards_per_share_wads,
            ) = array_refs![
                raw_pool_reward,
                PoolRewardId::LEN,
                PUBKEY_BYTES,
                8,  // start_time_secs
                4,  // duration_secs
                8,  // total_rewards
                8,  // num_user_reward_managers
                16  // cumulative_rewards_per_share
            ];

            let vault = Pubkey::new_from_array(*src_vault);
            let pool_reward_id = PoolRewardId(u32::from_le_bytes(*src_id));

            // SAFETY: ok to assign because we know the index is less than length
            pool_reward_manager.pool_rewards[index] = if vault == Pubkey::default() {
                PoolRewardSlot::Vacant {
                    last_pool_reward_id: pool_reward_id,
                }
            } else {
                PoolRewardSlot::Occupied(PoolReward {
                    id: pool_reward_id,
                    vault,
                    start_time_secs: u64::from_le_bytes(*src_start_time_secs),
                    duration_secs: u32::from_le_bytes(*src_duration_secs),
                    total_rewards: u64::from_le_bytes(*src_total_rewards),
                    num_user_reward_managers: u64::from_le_bytes(*src_num_user_reward_managers),
                    cumulative_rewards_per_share: unpack_decimal(
                        src_cumulative_rewards_per_share_wads,
                    ),
                })
            };
        }

        Ok(pool_reward_manager)
    }
}

#[cfg(test)]
mod tests {
    //! TODO: Rewrite these tests from their Suilend counterparts.
    //! TODO: Calculate test coverage and add tests for missing branches.

    use rand::Rng;

    use super::*;
    use proptest::prelude::*;

    fn pool_reward_manager_strategy() -> impl Strategy<Value = PoolRewardManager> {
        (0..100u32).prop_perturb(|_, mut rng| PoolRewardManager::new_rand(&mut rng))
    }

    proptest! {
        #[test]
        fn it_packs_and_unpacks(pool_reward_manager in pool_reward_manager_strategy()) {
            let mut packed = vec![0u8; PoolRewardManager::LEN];
            Pack::pack_into_slice(&pool_reward_manager, &mut packed);
            let unpacked = PoolRewardManager::unpack_from_slice(&packed).unwrap();
            prop_assert_eq!(pool_reward_manager, unpacked);
        }
    }

    #[test]
    fn it_unpacks_empty_bytes_as_default() {
        let packed = vec![0u8; PoolRewardManager::LEN];
        let unpacked = PoolRewardManager::unpack_from_slice(&packed).unwrap();
        assert_eq!(unpacked, PoolRewardManager::default());
    }

    #[test]
    fn it_fits_reserve_realloc_into_single_ix() {
        const MAX_REALLOC: usize = 10 * 1024;

        let size_of_discriminant = 1;
        let required_realloc = size_of_discriminant * PoolRewardManager::LEN;
        assert!(required_realloc <= MAX_REALLOC);
    }

    #[test]
    fn it_tests_pool_reward_manager_basic() {
        // TODO: rewrite Suilend "test_pool_reward_manager_basic" test
    }

    #[test]
    fn it_tests_pool_reward_manager_multiple_rewards() {
        // TODO: rewrite Suilend "test_pool_reward_manager_multiple_rewards"
    }

    #[test]
    fn it_tests_pool_reward_zero_share() {
        // TODO: rewrite Suilend "test_pool_reward_manager_zero_share"
    }

    #[test]
    fn it_tests_pool_reward_manager_auto_farm() {
        // TODO: rewrite Suilend "test_pool_reward_manager_auto_farm"
    }

    #[test]
    fn it_tests_add_too_many_pool_rewards() {
        // TODO: rewrite Suilend "test_add_too_many_pool_rewards"
    }

    #[test]
    fn it_tests_pool_reward_manager_cancel_and_close_regression() {
        // TODO: rewrite Suilend "test_pool_reward_manager_cancel_and_close_regression"
    }

    impl PoolRewardManager {
        pub(crate) fn new_rand(rng: &mut impl Rng) -> Self {
            Self {
                total_shares: rng.gen(),
                last_update_time_secs: rng.gen(),
                pool_rewards: std::array::from_fn(|_| {
                    let is_vacant = rng.gen_bool(0.5);

                    if is_vacant {
                        PoolRewardSlot::Vacant {
                            last_pool_reward_id: PoolRewardId(rng.gen()),
                        }
                    } else {
                        PoolRewardSlot::Occupied(PoolReward {
                            id: PoolRewardId(rng.gen()),
                            vault: Pubkey::new_unique(),
                            start_time_secs: rng.gen(),
                            duration_secs: rng.gen(),
                            total_rewards: rng.gen(),
                            cumulative_rewards_per_share: Decimal::from_scaled_val(rng.gen()),
                            num_user_reward_managers: rng.gen(),
                        })
                    }
                }),
            }
        }
    }
}

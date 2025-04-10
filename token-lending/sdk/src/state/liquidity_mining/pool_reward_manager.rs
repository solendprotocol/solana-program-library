//! [PoolRewardManager]s are stored in [crate::state::Reserve]s.
//! They can be either borrow or deposit but the logic is the same, the only
//! difference is how shares are calculated.
//!
//! For borrow managers the shares are "liability" and for deposit
//! managers the shares are "deposited collateral".

use crate::{
    error::LendingError,
    math::{Decimal, TryAdd, TryDiv, TryMul},
    state::{pack_decimal, unpack_decimal, MAX_REWARDS, MIN_REWARD_PERIOD_SECS},
};
use arrayref::{array_mut_ref, array_ref, array_refs, mut_array_refs};
use core::convert::{TryFrom, TryInto};
use solana_program::{
    clock::Clock,
    msg,
    program_error::ProgramError,
    program_pack::{Pack, Sealed},
    pubkey::{Pubkey, PUBKEY_BYTES},
};

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
        /// An optimization to avoid writing data that has not changed.
        /// When vacating a slot we set this to true.
        /// That way the packing logic knows whether it's fine to skip the
        /// packing or not.
        has_been_just_vacated: bool,
    },
    /// Reward has not been closed yet.
    ///
    /// We box the [PoolReward] to avoid stack overflow.
    Occupied(Box<PoolReward>),
}

/// Tracks rewards in a specific mint over some period of time.
///
/// # Reward cancellation
///
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
    ///
    /// # Reward cancellation
    ///
    /// Is cut short if the reward is cancelled.
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

impl PoolRewardManager {
    /// Adds a new pool reward.
    ///
    /// Will first update itself.
    ///
    /// Start time will be set to now if it's in the past.
    /// Must last at least [MIN_REWARD_PERIOD_SECS].
    /// The amount of tokens to distribute must be greater than zero.
    ///
    /// Will return an error if no slot can be found for the new reward.
    pub fn add_pool_reward(
        &mut self,
        vault: Pubkey,
        start_time_secs: u64,
        end_time_secs: u64,
        reward_token_amount: u64,
        clock: &Clock,
    ) -> Result<(), ProgramError> {
        self.update(clock)?;

        let start_time_secs = start_time_secs.max(clock.unix_timestamp as u64);

        if start_time_secs >= end_time_secs {
            msg!("Pool reward must end after it starts");
            return Err(LendingError::PoolRewardPeriodTooShort.into());
        }

        let duration_secs: u32 = {
            // SAFETY: just checked that start time is strictly smaller
            let d = end_time_secs - start_time_secs;
            d.try_into().map_err(|_| {
                msg!("Pool reward duration is too long");
                LendingError::MathOverflow
            })?
        };
        if MIN_REWARD_PERIOD_SECS > duration_secs as u64 {
            msg!("Pool reward duration must be at least {MIN_REWARD_PERIOD_SECS} secs");
            return Err(LendingError::PoolRewardPeriodTooShort.into());
        }

        if reward_token_amount == 0 {
            msg!("Pool reward amount must be greater than zero");
            return Err(LendingError::InvalidAmount.into());
        }

        let eligible_slot =
            self.pool_rewards
                .iter_mut()
                .enumerate()
                .find_map(|(slot_index, slot)| match slot {
                    PoolRewardSlot::Vacant {
                        last_pool_reward_id: PoolRewardId(id),
                        ..
                    } if *id < u32::MAX => Some((slot_index, PoolRewardId(*id + 1))),
                    _ => None,
                });

        let Some((slot_index, next_id)) = eligible_slot else {
            msg!("No vacant slot found for the new pool reward");
            return Err(LendingError::NoVacantSlotForPoolReward.into());
        };

        self.pool_rewards[slot_index] = PoolRewardSlot::Occupied(Box::new(PoolReward {
            id: next_id,
            vault,
            start_time_secs,
            duration_secs,
            total_rewards: reward_token_amount,
            num_user_reward_managers: 0,
            cumulative_rewards_per_share: Decimal::zero(),
        }));

        Ok(())
    }

    /// Sets the duration of the pool reward to now.
    /// Returns the amount of unallocated rewards and the vault they are in.
    pub fn cancel_pool_reward(
        &mut self,
        pool_reward_index: usize,
        clock: &Clock,
    ) -> Result<(Pubkey, u64), ProgramError> {
        self.update(clock)?;

        let Some(PoolRewardSlot::Occupied(pool_reward)) =
            self.pool_rewards.get_mut(pool_reward_index)
        else {
            msg!("Cannot cancel a non-existent pool reward");
            return Err(ProgramError::InvalidArgument);
        };

        if pool_reward.has_ended(clock) {
            msg!("Cannot cancel a pool reward that has already ended");
            return Err(LendingError::InvalidAccountInput.into());
        }

        let since_start_secs = clock.unix_timestamp as u64 - pool_reward.start_time_secs;
        let unlocked_rewards = Decimal::from(pool_reward.total_rewards)
            .try_mul(Decimal::from(since_start_secs))?
            .try_div(Decimal::from(pool_reward.duration_secs as u64))?
            .try_floor_u64()?;
        let remaining_rewards = pool_reward.total_rewards - unlocked_rewards;

        pool_reward.duration_secs =
            u32::try_from(since_start_secs).expect("New duration to be strictly shorter");

        Ok((pool_reward.vault, remaining_rewards))
    }

    /// Closes a pool reward if it has been cancelled before.
    /// Returns the vault the rewards are in.
    pub fn close_pool_reward(&mut self, pool_reward_index: usize) -> Result<Pubkey, ProgramError> {
        let Some(PoolRewardSlot::Occupied(pool_reward)) =
            self.pool_rewards.get_mut(pool_reward_index)
        else {
            msg!("Cannot close a non-existent pool reward");
            return Err(ProgramError::InvalidArgument);
        };

        if pool_reward.num_user_reward_managers > 0 {
            msg!("Cannot close a pool reward with active user reward managers");
            return Err(LendingError::InvalidAccountInput.into());
        }

        let vault = pool_reward.vault;

        self.pool_rewards[pool_reward_index] = PoolRewardSlot::Vacant {
            last_pool_reward_id: pool_reward.id,
            has_been_just_vacated: true,
        };

        Ok(vault)
    }
}

impl PoolRewardManager {
    /// Should be updated before any interaction with rewards.
    pub(crate) fn update(&mut self, clock: &Clock) -> Result<(), ProgramError> {
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

impl PoolReward {
    const LEN: usize = Self::HEAD_LEN + Self::TAIL_LEN;

    const HEAD_LEN: usize = PoolRewardId::LEN + PUBKEY_BYTES;

    /// - `start_time_secs``
    /// - `duration_secs``
    /// - `total_rewards``
    /// - `num_user_reward_managers``
    /// - `cumulative_rewards_per_share``
    const TAIL_LEN: usize = 8 + 4 + 8 + 8 + 16;

    /// Returns whether the reward has ended.
    pub(crate) fn has_ended(&self, clock: &Clock) -> bool {
        let end_time_secs = self.start_time_secs + self.duration_secs as u64;
        clock.unix_timestamp as u64 >= end_time_secs
    }
}

impl PoolRewardId {
    pub(crate) const LEN: usize = std::mem::size_of::<Self>();
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
            // this is used for initialization of the pool reward manager so
            // it makes sense as there are 0s in the account data already
            has_been_just_vacated: false,
        }
    }
}

impl PoolRewardManager {
    #[inline(never)]
    pub(crate) fn unpack_to_box(input: &[u8]) -> Result<Box<Self>, ProgramError> {
        Ok(Box::new(PoolRewardManager::unpack_from_slice(input)?))
    }
}

impl Sealed for PoolRewardManager {}

impl Pack for PoolRewardManager {
    /// total_shares + last_update_time_secs + pool_rewards.
    const LEN: usize = 8 + 8 + MAX_REWARDS * PoolReward::LEN;

    fn pack_into_slice(&self, output: &mut [u8]) {
        output[0..8].copy_from_slice(&self.total_shares.to_le_bytes());
        output[8..16].copy_from_slice(&self.last_update_time_secs.to_le_bytes());

        let rewards_to_pack = self
            .pool_rewards
            .iter()
            .enumerate()
            .filter(|(_, s)| s.should_be_packed());

        for (index, pool_reward_slot) in rewards_to_pack {
            let offset = 16 + index * PoolReward::LEN;

            let raw_pool_reward_head = array_mut_ref![output, offset, PoolReward::HEAD_LEN];
            let (dst_id, dst_vault) =
                mut_array_refs![raw_pool_reward_head, PoolRewardId::LEN, PUBKEY_BYTES];

            match pool_reward_slot {
                PoolRewardSlot::Vacant {
                    last_pool_reward_id: PoolRewardId(id),
                    ..
                } => {
                    dst_id.copy_from_slice(&id.to_le_bytes());
                    dst_vault.copy_from_slice(Pubkey::default().as_ref());
                }
                PoolRewardSlot::Occupied(pool_reward) => {
                    dst_id.copy_from_slice(&pool_reward.id.0.to_le_bytes());
                    dst_vault.copy_from_slice(pool_reward.vault.as_ref());

                    let raw_pool_reward_tail =
                        array_mut_ref![output, offset + PoolReward::HEAD_LEN, PoolReward::TAIL_LEN];

                    let (
                        dst_start_time_secs,
                        dst_duration_secs,
                        dst_total_rewards,
                        dst_num_user_reward_managers,
                        dst_cumulative_rewards_per_share_wads,
                    ) = mut_array_refs![
                        raw_pool_reward_tail,
                        8,  // start_time_secs
                        4,  // duration_secs
                        8,  // total_rewards
                        8,  // num_user_reward_managers
                        16  // cumulative_rewards_per_share
                    ];

                    *dst_start_time_secs = pool_reward.start_time_secs.to_le_bytes();
                    *dst_duration_secs = pool_reward.duration_secs.to_le_bytes();
                    *dst_total_rewards = pool_reward.total_rewards.to_le_bytes();
                    *dst_num_user_reward_managers =
                        pool_reward.num_user_reward_managers.to_le_bytes();
                    pack_decimal(
                        pool_reward.cumulative_rewards_per_share,
                        dst_cumulative_rewards_per_share_wads,
                    );
                }
            };
        }
    }

    #[inline(never)]
    fn unpack_from_slice(input: &[u8]) -> Result<Self, ProgramError> {
        let mut pool_reward_manager = PoolRewardManager {
            total_shares: u64::from_le_bytes(*array_ref![input, 0, 8]),
            last_update_time_secs: u64::from_le_bytes(*array_ref![input, 8, 8]),
            ..Default::default()
        };

        for index in 0..MAX_REWARDS {
            let offset = 8 + 8 + index * PoolReward::LEN;
            let raw_pool_reward_head = array_ref![input, offset, PoolReward::HEAD_LEN];

            #[allow(clippy::ptr_offset_with_cast)]
            let (src_id, src_vault) =
                array_refs![raw_pool_reward_head, PoolRewardId::LEN, PUBKEY_BYTES];

            let pool_reward_id = PoolRewardId(u32::from_le_bytes(*src_id));
            let vault = Pubkey::new_from_array(*src_vault);

            // SAFETY: ok to assign because we know the index is less than length
            pool_reward_manager.pool_rewards[index] = if vault == Pubkey::default() {
                PoolRewardSlot::Vacant {
                    last_pool_reward_id: pool_reward_id,
                    // nope, has been vacant since unpack
                    has_been_just_vacated: false,
                }
            } else {
                let raw_pool_reward_tail =
                    array_ref![input, offset + PoolReward::HEAD_LEN, PoolReward::TAIL_LEN];

                let (
                    src_start_time_secs,
                    src_duration_secs,
                    src_total_rewards,
                    src_num_user_reward_managers,
                    src_cumulative_rewards_per_share_wads,
                ) = array_refs![
                    raw_pool_reward_tail,
                    8,  // start_time_secs
                    4,  // duration_secs
                    8,  // total_rewards
                    8,  // num_user_reward_managers
                    16  // cumulative_rewards_per_share
                ];

                PoolRewardSlot::Occupied(Box::new(PoolReward {
                    id: pool_reward_id,
                    vault,
                    start_time_secs: u64::from_le_bytes(*src_start_time_secs),
                    duration_secs: u32::from_le_bytes(*src_duration_secs),
                    total_rewards: u64::from_le_bytes(*src_total_rewards),
                    num_user_reward_managers: u64::from_le_bytes(*src_num_user_reward_managers),
                    cumulative_rewards_per_share: unpack_decimal(
                        src_cumulative_rewards_per_share_wads,
                    ),
                }))
            };
        }

        Ok(pool_reward_manager)
    }
}

impl PoolRewardSlot {
    /// If we know for sure that data hasn't changed then we can just skip packing.
    fn should_be_packed(&self) -> bool {
        let for_sure_has_not_changed = matches!(
            self,
            Self::Vacant {
                has_been_just_vacated: false,
                ..
            }
        );

        !for_sure_has_not_changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    impl PoolRewardManager {
        pub(crate) fn new_rand(rng: &mut impl rand::Rng) -> Self {
            Self {
                total_shares: rng.gen(),
                last_update_time_secs: rng.gen(),
                pool_rewards: std::array::from_fn(|_| {
                    let is_vacant = rng.gen_bool(0.5);

                    if is_vacant {
                        PoolRewardSlot::Vacant {
                            last_pool_reward_id: Default::default(),
                            has_been_just_vacated: false,
                        }
                    } else {
                        PoolRewardSlot::Occupied(Box::new(PoolReward {
                            id: PoolRewardId(rng.gen()),
                            vault: Pubkey::new_unique(),
                            start_time_secs: rng.gen(),
                            duration_secs: rng.gen(),
                            total_rewards: rng.gen(),
                            cumulative_rewards_per_share: Decimal::from_scaled_val(rng.gen()),
                            num_user_reward_managers: rng.gen(),
                        }))
                    }
                }),
            }
        }
    }

    #[test]
    fn it_packs_id_if_vacated_in_this_tx() {
        let mut m = PoolRewardManager::default();
        m.pool_rewards[0] = PoolRewardSlot::Vacant {
            last_pool_reward_id: PoolRewardId(69),
            has_been_just_vacated: true,
        };

        let mut packed = vec![0u8; PoolRewardManager::LEN];
        m.pack_into_slice(&mut packed);
        let unpacked = PoolRewardManager::unpack_from_slice(&packed).unwrap();

        assert_eq!(
            unpacked.pool_rewards[0],
            PoolRewardSlot::Vacant {
                last_pool_reward_id: PoolRewardId(69),
                has_been_just_vacated: false,
            }
        );
    }

    #[test]
    fn it_unpacks_empty_pool_reward_manager_bytes_as_default() {
        let packed = vec![0u8; PoolRewardManager::LEN];
        let unpacked = PoolRewardManager::unpack_from_slice(&packed).unwrap();
        assert_eq!(unpacked, PoolRewardManager::default());

        // sanity check that everything starts at 0
        let all_rewards_are_empty = unpacked.pool_rewards.iter().all(|pool_reward| {
            matches!(
                pool_reward,
                PoolRewardSlot::Vacant {
                    last_pool_reward_id: PoolRewardId(0),
                    has_been_just_vacated: false,
                }
            )
        });

        assert!(all_rewards_are_empty);
    }

    #[test]
    fn it_fits_reserve_realloc_into_single_ix() {
        const MAX_REALLOC: usize = solana_program::entrypoint::MAX_PERMITTED_DATA_INCREASE;

        let size_of_discriminant = 1;
        let required_realloc = size_of_discriminant * PoolRewardManager::LEN;
        assert!(required_realloc <= MAX_REALLOC);
    }

    fn pool_reward_manager_strategy() -> impl Strategy<Value = PoolRewardManager> {
        (0..1u32).prop_perturb(|_, mut rng| PoolRewardManager::new_rand(&mut rng))
    }

    proptest! {
        #[test]
        fn it_packs_and_unpacks_pool_reward_manager(pool_reward_manager in pool_reward_manager_strategy()) {
            let mut packed = vec![0u8; PoolRewardManager::LEN];
            Pack::pack_into_slice(&pool_reward_manager, &mut packed);
            let unpacked = PoolRewardManager::unpack_from_slice(&packed).unwrap();

            prop_assert_eq!(pool_reward_manager.last_update_time_secs, unpacked.last_update_time_secs);
            prop_assert_eq!(pool_reward_manager.total_shares, unpacked.total_shares);

            for (og, unpacked) in pool_reward_manager.pool_rewards.iter().zip(unpacked.pool_rewards.iter()) {
                prop_assert_eq!(og, unpacked);
            }
        }
    }
}

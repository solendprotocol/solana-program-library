//! [UserRewardManager]s are stored in [crate::state::Obligation]s for each
//! reserve the user has borrowed from or deposited into at the current time or
//! in the past.

use crate::{
    error::LendingError,
    math::{Decimal, TryAdd, TryMul, TrySub},
    state::{
        pack_decimal, unpack_decimal, PoolRewardId, PoolRewardManager, PoolRewardSlot,
        PositionKind, MAX_REWARDS,
    },
};
use arrayref::{array_mut_ref, array_ref, array_refs, mut_array_refs};
use core::{
    convert::TryInto,
    ops::{Deref, DerefMut},
};
use solana_program::{
    clock::Clock,
    msg,
    program_error::ProgramError,
    pubkey::{Pubkey, PUBKEY_BYTES},
};

/// Wraps over user reward managers and allows mutable access to them while
/// other obligation fields are borrowed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UserRewardManagers(Vec<UserRewardManager>);

/// Tracks user's LM rewards for a specific pool (reserve.)
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct UserRewardManager {
    /// Links this manager to a reserve.
    reserve: Pubkey,
    /// Although a user cannot both borrow and deposit in the same reserve, they
    /// can deposit, withdraw and then borrow the same reserve.
    /// Meanwhile they could've accumulated some rewards that'd be lost.
    ///
    /// Also, have an explicit distinguish between borrow and deposit doesn't
    /// suffer from a footgun of misattributing rewards.
    position_kind: PositionKind,
    /// For deposits, this is the amount of collateral token user has in
    /// their obligation deposit.
    ///
    /// For borrows, this is (borrow_amount / cumulative_borrow_rate) user
    /// has in their obligation borrow.
    share: u64,
    /// Monotonically increasing time taken from clock sysvar.
    last_update_time_secs: u64,
    /// The indices on [Self::rewards] are _not_ correlated with
    /// [PoolRewardManager::pool_rewards].
    /// Instead, this vector only tracks meaningful rewards for the user.
    /// See [UserReward::pool_reward_index].
    ///
    /// This is a diversion from the Suilend implementation.
    rewards: Vec<UserReward>,
}

/// Track user rewards for a specific [PoolReward].
#[derive(Debug, PartialEq, Eq, Default, Clone)]
struct UserReward {
    /// Which [PoolReward] within the reserve's index does this [UserReward]
    /// correspond to.
    ///
    /// # (Un)packing
    /// There are ever only going to be at most [MAX_REWARDS].
    /// We therefore pack this value into a byte.
    pool_reward_index: usize,
    /// Each pool reward gets an ID which is monotonically increasing with each
    /// new reward added to the pool.
    pool_reward_id: PoolRewardId,
    /// Before [UserReward.cumulative_rewards_per_share] is copied we find
    /// time difference between current global rewards and last user update
    /// rewards:
    /// [PoolReward.cumulative_rewards_per_share] - [UserReward.cumulative_rewards_per_share]
    ///
    /// Then, we multiply that difference by [UserRewardManager.share] and
    /// add the result to this counter.
    earned_rewards: Decimal,
    /// copied from [PoolReward.cumulative_rewards_per_share] at the time of the last update
    cumulative_rewards_per_share: Decimal,
}

/// When creating a new [UserRewardManager] we need to know whether we should
/// populate it with rewards or not.
enum CreatingNewUserRewardManager {
    /// If we are creating a [UserRewardManager] then we want to populate it.
    Yes,
    /// If we are updating an existing [UserRewardManager] then we don't want
    /// to populate it.
    No,
}

impl UserRewardManagers {
    /// Returns [UserRewardManager] for the given reserve if any
    pub fn find_mut(
        &mut self,
        reserve: Pubkey,
        position_kind: PositionKind,
    ) -> Option<&mut UserRewardManager> {
        self.0.iter_mut().find(|user_reward_manager| {
            user_reward_manager.reserve == reserve
                && user_reward_manager.position_kind == position_kind
        })
    }

    /// Updates the [UserRewardManager] for the given reserve.
    ///
    /// The caller must make sure that the provided [PoolRewardManager] is valid
    /// for the given reserve.
    ///
    /// If an associated [UserRewardManager] is not found, it will be created.
    ///
    /// # Important
    ///
    /// Only call this if you're sure that the obligation should be tracking
    /// rewards for the given reserve.
    pub fn set_share(
        &mut self,
        reserve: Pubkey,
        position_kind: PositionKind,
        pool_reward_manager: &mut PoolRewardManager,
        new_share: u64,
        clock: &Clock,
    ) -> Result<(), ProgramError> {
        let user_reward_manager = if let Some(user_reward_manager) =
            self.find_mut(reserve, position_kind)
        {
            user_reward_manager.update(
                pool_reward_manager,
                clock,
                CreatingNewUserRewardManager::No,
            )?;
            user_reward_manager
        } else {
            let mut new_user_reward_manager = UserRewardManager::new(reserve, position_kind, clock);
            new_user_reward_manager.populate(pool_reward_manager, clock)?;
            self.0.push(new_user_reward_manager);
            // SAFETY: we just pushed a new item to the vector so ok to unwrap
            self.0.last_mut().unwrap()
        };

        user_reward_manager.set_share(pool_reward_manager, new_share);

        Ok(())
    }
}

impl UserRewardManager {
    /// Claims all rewards that the user has earned.
    /// Returns how many tokens should be transferred to the user.
    ///
    /// # Note
    ///
    /// Errors if there is no pool reward with this vault.
    pub fn claim_rewards(
        &mut self,
        pool_reward_manager: &mut PoolRewardManager,
        vault: Pubkey,
        clock: &Clock,
    ) -> Result<u64, ProgramError> {
        self.update(pool_reward_manager, clock, CreatingNewUserRewardManager::No)?;

        let (pool_reward_index, pool_reward) = pool_reward_manager
            .pool_rewards
            .iter_mut()
            .enumerate()
            .find_map(move |(index, slot)| match slot {
                PoolRewardSlot::Occupied(pool_reward) if pool_reward.vault == vault => {
                    Some((index, pool_reward))
                }
                _ => None,
            })
            .ok_or(LendingError::NoPoolRewardMatches)?;

        let Some((user_reward_index, user_reward)) =
            self.rewards
                .iter_mut()
                .enumerate()
                .find(|(_, user_reward)| {
                    user_reward.pool_reward_index == pool_reward_index
                        && user_reward.pool_reward_id == pool_reward.id
                })
        else {
            // User is not tracking this reward, nothing to claim.
            // Let's be graceful and make this a no-op.
            // Prevents failures when multiple parties crank rewards.
            return Ok(0);
        };

        let to_claim = user_reward.withdraw_earned_rewards()?;

        if pool_reward.has_ended(clock) && user_reward.earned_rewards.try_floor_u64()? == 0 {
            // This reward won't be used anymore as it ended and the user
            // claimed all there was to claim.
            // We can clean up this user reward.
            // We're fine with swap remove bcs `user_reward_index` is meaningless.
            // SAFETY: We got the index from enumeration, so must exist.
            self.rewards.swap_remove(user_reward_index);
            pool_reward.num_user_reward_managers -= 1;
        }

        Ok(to_claim)
    }
}

impl UserRewardManager {
    /// [Self] is dynamically sized based on how many [PoolReward]s are there
    /// for the given [Self::reserve].
    ///
    /// This is the maximum length a manager can have.
    pub(crate) const MAX_LEN: usize = Self::HEAD_LEN + MAX_REWARDS * UserReward::LEN;

    /// Length of data before [Self::rewards] tail.
    ///
    /// - [Self::reserve]
    /// - [Self::position_kind]
    /// - [Self::share]
    /// - [Self::last_update_time_secs]
    /// - [Self::rewards] vector length as u8
    const HEAD_LEN: usize = PUBKEY_BYTES + 1 + 8 + 8 + 1;

    /// Creates a new empty [UserRewardManager] for the given reserve.
    pub(crate) fn new(reserve: Pubkey, position_kind: PositionKind, clock: &Clock) -> Self {
        Self {
            reserve,
            last_update_time_secs: clock.unix_timestamp as _,
            position_kind,
            share: 0,
            rewards: Vec::new(),
        }
    }

    /// Sets new share value for this manager.
    pub(crate) fn set_share(
        &mut self,
        pool_reward_manager: &mut PoolRewardManager,
        new_share: u64,
    ) {
        msg!(
            "For reserve {} there are {} total shares. \
            User's previous position was at {} and new is at {}",
            self.reserve,
            pool_reward_manager.total_shares,
            self.share,
            new_share
        );

        // This works even for migrations.
        // User's old share is 0 although it shouldn't be bcs they have borrowed
        // or deposited.
        // We only now attribute the share to the user which is fine, it's as if
        // they just now borrowed/deposited.
        pool_reward_manager.total_shares =
            pool_reward_manager.total_shares - self.share + new_share;

        self.share = new_share;
    }

    /// When user borrows/deposits for a new reserve this function copies all
    /// reserve rewards from the pool manager to the user manager and starts
    /// accruing rewards.
    ///
    /// Invoker must have checked that this [PoolRewardManager] matches the
    /// [UserRewardManager].
    pub(crate) fn populate(
        &mut self,
        pool_reward_manager: &mut PoolRewardManager,
        clock: &Clock,
    ) -> Result<(), ProgramError> {
        self.update(
            pool_reward_manager,
            clock,
            CreatingNewUserRewardManager::Yes,
        )
    }

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

        for (pool_reward_index, pool_reward) in
            pool_reward_manager.pool_rewards.iter_mut().enumerate()
        {
            let PoolRewardSlot::Occupied(pool_reward) = pool_reward else {
                // no reward to track
                continue;
            };

            let maybe_user_reward = self
                .rewards
                .iter_mut()
                .enumerate()
                .find(|(_, r)| r.pool_reward_index == pool_reward_index);

            let end_time_secs = pool_reward.start_time_secs + pool_reward.duration_secs as u64;
            let has_ended_for_user = self.last_update_time_secs >= end_time_secs;

            match maybe_user_reward {
                Some((user_reward_index, user_reward))
                    if has_ended_for_user && user_reward.earned_rewards.try_floor_u64()? == 0 =>
                {
                    // Reward period ended and there's nothing to crank.
                    // We can clean up this user reward.
                    // We're fine with swap remove bcs `user_reward_index` is meaningless.
                    // SAFETY: We got the index from enumeration, so must exist.
                    self.rewards.swap_remove(user_reward_index);
                    pool_reward.num_user_reward_managers -= 1;
                }
                _ if has_ended_for_user => {
                    // reward period over & there are rewards yet to be cracked
                }
                Some((_, user_reward)) => {
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
                None if pool_reward.start_time_secs > curr_unix_timestamp_secs => {
                    // reward period has not started yet
                }
                None => {
                    // user did not yet start accruing rewards

                    let new_user_reward = UserReward {
                        pool_reward_index,
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

                    self.rewards.push(new_user_reward);
                    pool_reward.num_user_reward_managers += 1;
                }
            }
        }

        self.last_update_time_secs = curr_unix_timestamp_secs;

        Ok(())
    }

    /// How many bytes are needed to pack this [UserRewardManager].
    pub(crate) fn size_in_bytes_when_packed(&self) -> usize {
        Self::HEAD_LEN + self.rewards.len() * UserReward::LEN
    }

    /// Because [Self] is dynamically sized we don't implement [Pack] that
    /// contains a misleading const `LEN`.
    ///
    /// We return how many bytes were written.
    pub(crate) fn pack_into_slice(&self, output: &mut [u8]) {
        let raw_user_reward_manager = array_mut_ref![output, 0, UserRewardManager::HEAD_LEN];

        let (
            dst_reserve,
            dst_position_kind,
            dst_share,
            dst_last_update_time_secs,
            dst_user_rewards_len,
        ) = mut_array_refs![
            raw_user_reward_manager,
            PUBKEY_BYTES,
            1, // position_kind
            8, // share
            8, // last_update_time_secs
            1  // length of rewards array that's next to come
        ];

        dst_reserve.copy_from_slice(self.reserve.as_ref());
        dst_position_kind.copy_from_slice(&(self.position_kind as u8).to_le_bytes());
        dst_share.copy_from_slice(&self.share.to_le_bytes());
        dst_last_update_time_secs.copy_from_slice(&self.last_update_time_secs.to_le_bytes());
        dst_user_rewards_len.copy_from_slice(
            &({
                debug_assert!(MAX_REWARDS >= self.rewards.len());
                debug_assert!(u8::MAX >= MAX_REWARDS as _);
                self.rewards.len() as u8
            })
            .to_le_bytes(),
        );

        for (index, user_reward) in self.rewards.iter().enumerate() {
            let offset = Self::HEAD_LEN + index * UserReward::LEN;
            let raw_user_reward = array_mut_ref![output, offset, UserReward::LEN];

            let (
                dst_pool_reward_index,
                dst_pool_reward_id,
                dst_earned_rewards,
                dst_cumulative_rewards_per_share,
            ) = mut_array_refs![raw_user_reward, 1, PoolRewardId::LEN, 16, 16];

            dst_pool_reward_id.copy_from_slice(&user_reward.pool_reward_id.0.to_le_bytes());
            pack_decimal(user_reward.earned_rewards, dst_earned_rewards);
            pack_decimal(
                user_reward.cumulative_rewards_per_share,
                dst_cumulative_rewards_per_share,
            );
            let pool_reward_index = {
                assert!(user_reward.pool_reward_index < MAX_REWARDS);
                assert!(MAX_REWARDS < u8::MAX as _);
                // will always fit
                user_reward.pool_reward_index as u8
            };
            dst_pool_reward_index.copy_from_slice(&pool_reward_index.to_le_bytes());
        }
    }

    pub(crate) fn unpack_from_slice(input: &[u8]) -> Result<Self, ProgramError> {
        #[allow(clippy::ptr_offset_with_cast)]
        let raw_user_reward_manager_head = array_ref![input, 0, UserRewardManager::HEAD_LEN];

        #[allow(clippy::ptr_offset_with_cast)]
        let (
            src_reserve,
            src_position_kind,
            src_share,
            src_last_update_time_secs,
            src_user_rewards_len,
        ) = array_refs![
            raw_user_reward_manager_head,
            PUBKEY_BYTES,
            1, // position_kind
            8, // share
            8, // last_update_time_secs
            1  // length of rewards array that's next to come
        ];

        let reserve = Pubkey::new_from_array(*src_reserve);
        let position_kind = u8::from_le_bytes(*src_position_kind).try_into()?;
        let user_rewards_len = u8::from_le_bytes(*src_user_rewards_len) as _;
        let share = u64::from_le_bytes(*src_share);
        let last_update_time_secs = u64::from_le_bytes(*src_last_update_time_secs);

        let mut rewards = Vec::with_capacity(user_rewards_len);
        for index in 0..user_rewards_len {
            let offset = Self::HEAD_LEN + index * UserReward::LEN;
            let raw_user_reward = array_ref![input, offset, UserReward::LEN];

            #[allow(clippy::ptr_offset_with_cast)]
            let (
                src_pool_reward_index,
                src_pool_reward_id,
                src_earned_rewards,
                src_cumulative_rewards_per_share,
            ) = array_refs![raw_user_reward, 1, PoolRewardId::LEN, 16, 16];

            rewards.push(UserReward {
                pool_reward_index: u8::from_le_bytes(*src_pool_reward_index) as _,
                pool_reward_id: PoolRewardId(u32::from_le_bytes(*src_pool_reward_id)),
                earned_rewards: unpack_decimal(src_earned_rewards),
                cumulative_rewards_per_share: unpack_decimal(src_cumulative_rewards_per_share),
            });
        }

        Ok(Self {
            reserve,
            position_kind,
            share,
            last_update_time_secs,
            rewards,
        })
    }
}

impl UserReward {
    /// - [UserReward::pool_reward_index] truncated to a byte
    /// - [PoolRewardId]
    /// - packed [Decimal]
    /// - packed [Decimal]
    const LEN: usize = 1 + PoolRewardId::LEN + 16 + 16;

    /// Removes all earned rewards from [Self] and returns them.
    ///
    /// # Note
    /// Decimals are truncated to u64, dust is kept.
    fn withdraw_earned_rewards(&mut self) -> Result<u64, ProgramError> {
        let reward_amount = self.earned_rewards.try_floor_u64()?;

        if reward_amount > 0 {
            self.earned_rewards = self.earned_rewards.try_sub(reward_amount.into())?;
        }

        Ok(reward_amount)
    }
}

impl From<Vec<UserRewardManager>> for UserRewardManagers {
    fn from(user_reward_managers: Vec<UserRewardManager>) -> Self {
        Self(user_reward_managers)
    }
}

impl From<UserRewardManagers> for Vec<UserRewardManager> {
    fn from(user_reward_managers: UserRewardManagers) -> Self {
        user_reward_managers.0
    }
}

impl Deref for UserRewardManagers {
    type Target = Vec<UserRewardManager>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for UserRewardManagers {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Default for UserRewardManager {
    fn default() -> Self {
        Self {
            reserve: Pubkey::default(),
            position_kind: PositionKind::Deposit,
            share: 0,
            last_update_time_secs: 0,
            rewards: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    impl UserRewardManager {
        pub(crate) fn new_rand(rng: &mut impl rand::Rng) -> Self {
            let rewards_len = rng.gen_range(0..MAX_REWARDS);
            Self {
                reserve: Pubkey::new_unique(),
                position_kind: rng.gen_range(0..=1u8).try_into().unwrap(),
                share: rng.gen(),
                last_update_time_secs: rng.gen(),
                rewards: std::iter::from_fn(|| {
                    Some(UserReward {
                        pool_reward_index: rng.gen_range(0..MAX_REWARDS),
                        pool_reward_id: PoolRewardId(rng.gen()),
                        earned_rewards: Decimal::from_scaled_val(rng.gen()),
                        cumulative_rewards_per_share: Decimal::from_scaled_val(rng.gen()),
                    })
                })
                .take(rewards_len)
                .collect(),
            }
        }
    }

    fn user_reward_manager_strategy() -> impl Strategy<Value = UserRewardManager> {
        (0..100u32).prop_perturb(|_, mut rng| UserRewardManager::new_rand(&mut rng))
    }

    proptest! {
        #[test]
        fn it_packs_and_unpacks_user_reward_manager(user_reward_manager in user_reward_manager_strategy()) {
            let mut packed = vec![0u8; UserRewardManager::MAX_LEN];
            user_reward_manager.pack_into_slice(&mut packed);
            let unpacked = UserRewardManager::unpack_from_slice(&packed).unwrap();
            prop_assert_eq!(user_reward_manager, unpacked);
        }
    }
}

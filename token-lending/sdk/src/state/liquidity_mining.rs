use super::pack_decimal;
use crate::{
    error::LendingError,
    math::{Decimal, TryAdd, TryDiv, TryMul, TrySub},
    state::{unpack_decimal, PositionKind},
};
use arrayref::{array_mut_ref, array_ref, array_refs, mut_array_refs};
use core::{
    convert::TryInto,
    ops::{Deref, DerefMut},
};
use solana_program::msg;
use solana_program::program_pack::{Pack, Sealed};
use solana_program::{
    clock::Clock,
    program_error::ProgramError,
    pubkey::{Pubkey, PUBKEY_BYTES},
};
use std::convert::TryFrom;

/// Determines the size of [PoolRewardManager]
pub const MAX_REWARDS: usize = 50;

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

/// Wraps over user reward managers and allows mutable access to them while
/// other obligation fields are borrowed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UserRewardManagers(pub Vec<UserRewardManager>);

/// Tracks user's LM rewards for a specific pool (reserve.)
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct UserRewardManager {
    /// Links this manager to a reserve.
    pub reserve: Pubkey,
    /// Although a user cannot both borrow and deposit in the same reserve, they
    /// can deposit, withdraw and then borrow the same reserve.
    /// Meanwhile they could've accumulated some rewards that'd be lost.
    ///
    /// Also, have an explicit distinguish between borrow and deposit doesn't
    /// suffer from a footgun of misattributing rewards.
    pub position_kind: PositionKind,
    /// For deposits, this is the amount of collateral token user has in
    /// their obligation deposit.
    ///
    /// For borrows, this is (borrow_amount / cumulative_borrow_rate) user
    /// has in their obligation borrow.
    pub share: u64,
    /// Monotonically increasing time taken from clock sysvar.
    pub last_update_time_secs: u64,
    /// The indices on [Self::rewards] are _not_ correlated with
    /// [PoolRewardManager::pool_rewards].
    /// Instead, this vector only tracks meaningful rewards for the user.
    /// See [UserReward::pool_reward_index].
    ///
    /// This is a diversion from the Suilend implementation.
    pub rewards: Vec<UserReward>,
}

/// Track user rewards for a specific [PoolReward].
#[derive(Debug, PartialEq, Eq, Default, Clone)]
pub struct UserReward {
    /// Which [PoolReward] within the reserve's index does this [UserReward]
    /// correspond to.
    ///
    /// # (Un)packing
    /// There are ever only going to be at most [MAX_REWARDS].
    /// We therefore pack this value into a byte.
    pub pool_reward_index: usize,
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
            return Err(LendingError::MathOverflow.into());
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
            user_reward_manager.update(pool_reward_manager, clock)?;
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
    /// Creates a new empty [UserRewardManager] for the given reserve.
    pub fn new(reserve: Pubkey, position_kind: PositionKind, clock: &Clock) -> Self {
        Self {
            reserve,
            last_update_time_secs: clock.unix_timestamp as _,
            position_kind,
            share: 0,
            rewards: Vec::new(),
        }
    }

    /// Sets new share value for this manager.
    fn set_share(&mut self, pool_reward_manager: &mut PoolRewardManager, new_share: u64) {
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
        self.update(pool_reward_manager, clock)?;

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

    /// Should be updated before any interaction with rewards.
    ///
    /// Invoker must have checked that this [PoolRewardManager] matches the
    /// [UserRewardManager].
    pub fn update(
        &mut self,
        pool_reward_manager: &mut PoolRewardManager,
        clock: &Clock,
    ) -> Result<(), ProgramError> {
        self.update_(pool_reward_manager, clock, CreatingNewUserRewardManager::No)
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
        self.update_(
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
    fn update_(
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
    pub fn has_ended(&self, clock: &Clock) -> bool {
        let end_time_secs = self.start_time_secs + self.duration_secs as u64;
        clock.unix_timestamp as u64 >= end_time_secs
    }
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
                    // TBD: do we want to ceil?
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

impl UserReward {
    /// - [UserReward::pool_reward_index] truncated to a byte
    /// - [PoolRewardId]
    /// - packed [Decimal]
    /// - packed [Decimal]
    pub const LEN: usize = 1 + PoolRewardId::LEN + 16 + 16;

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

impl UserRewardManager {
    /// [Self] is dynamically sized based on how many [PoolReward]s are there
    /// for the given [Self::reserve].
    ///
    /// This is the maximum length a manager can have.
    pub const MAX_LEN: usize = Self::HEAD_LEN + MAX_REWARDS * UserReward::LEN;

    /// Length of data before [Self::rewards] tail.
    ///
    /// - [Self::reserve]
    /// - [Self::position_kind]
    /// - [Self::share]
    /// - [Self::last_update_time_secs]
    /// - [Self::rewards] vector length as u8
    const HEAD_LEN: usize = PUBKEY_BYTES + 1 + 8 + 8 + 1;

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
    //! TODO: Rewrite these tests from their Suilend counterparts.
    //! TODO: Calculate test coverage and add tests for missing branches.

    use super::*;
    use pretty_assertions::assert_eq;
    use proptest::prelude::*;
    use rand::Rng;

    const SECONDS_IN_A_DAY: u64 = 86_400;

    fn pool_reward_manager_strategy() -> impl Strategy<Value = PoolRewardManager> {
        (0..1u32).prop_perturb(|_, mut rng| PoolRewardManager::new_rand(&mut rng))
    }

    fn user_reward_manager_strategy() -> impl Strategy<Value = UserRewardManager> {
        (0..100u32).prop_perturb(|_, mut rng| UserRewardManager::new_rand(&mut rng))
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

        #[test]
        fn it_packs_and_unpacks_user_reward_manager(user_reward_manager in user_reward_manager_strategy()) {
            let mut packed = vec![0u8; UserRewardManager::MAX_LEN];
            user_reward_manager.pack_into_slice(&mut packed);
            let unpacked = UserRewardManager::unpack_from_slice(&packed).unwrap();
            prop_assert_eq!(user_reward_manager, unpacked);
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

    /// This tests replicates calculations from Suilend's
    /// "test_pool_reward_manager_basic" test.
    #[test]
    fn it_tests_pool_reward_manager_basic() {
        let usdc = Pubkey::new_unique(); // reserve pubkey
        let slnd_vault = Pubkey::new_unique(); // where rewards are stored

        let mut clock = Clock {
            unix_timestamp: 0,
            ..Default::default()
        };

        let mut pool_reward_manager = PoolRewardManager::default();
        {
            // setup pool reward manager with one reward

            pool_reward_manager
                .add_pool_reward(
                    slnd_vault,
                    0,
                    20 * SECONDS_IN_A_DAY,
                    100 * 1_000_000,
                    &clock,
                )
                .expect("It adds pool reward");
            assert_eq!(
                pool_reward_manager.pool_rewards[0],
                PoolRewardSlot::Occupied(Box::new(PoolReward {
                    id: PoolRewardId(1),
                    vault: slnd_vault,
                    start_time_secs: 0,
                    duration_secs: 20 * SECONDS_IN_A_DAY as u32,
                    total_rewards: 100 * 1_000_000,
                    cumulative_rewards_per_share: Decimal::zero(),
                    num_user_reward_managers: 0,
                }))
            );
        }

        let mut user_reward_manager_1 = UserRewardManager::new(usdc, PositionKind::Deposit, &clock);
        {
            // setup user reward manager with 100/100 shares

            user_reward_manager_1
                .populate(&mut pool_reward_manager, &clock)
                .expect("It populates user reward manager");
            user_reward_manager_1.set_share(&mut pool_reward_manager, 100);
        }

        {
            // 1/4 of the reward time passes
            clock.unix_timestamp = 5 * SECONDS_IN_A_DAY as i64;

            let claimed_slnd = user_reward_manager_1
                .claim_rewards(&mut pool_reward_manager, slnd_vault, &clock)
                .expect("It claims rewards");
            assert_eq!(claimed_slnd, 25 * 1_000_000);
        }

        let mut user_reward_manager_2 = UserRewardManager::new(usdc, PositionKind::Deposit, &clock);
        {
            // setup user reward manager with 400/500 shares

            user_reward_manager_2
                .populate(&mut pool_reward_manager, &clock)
                .expect("It populates user reward manager");
            user_reward_manager_2.set_share(&mut pool_reward_manager, 400);
        }

        {
            // 1/2 of the reward time passes
            clock.unix_timestamp = 10 * SECONDS_IN_A_DAY as i64;

            let claimed_slnd = user_reward_manager_1
                .claim_rewards(&mut pool_reward_manager, slnd_vault, &clock)
                .expect("It claims rewards");
            assert_eq!(claimed_slnd, 5 * 1_000_000);

            let claimed_slnd = user_reward_manager_2
                .claim_rewards(&mut pool_reward_manager, slnd_vault, &clock)
                .expect("It claims rewards");
            assert_eq!(claimed_slnd, 20 * 1_000_000);
        }

        {
            // set both user reward managers to 250/500 shares
            user_reward_manager_1.set_share(&mut pool_reward_manager, 250);
            user_reward_manager_2.set_share(&mut pool_reward_manager, 250);
        }

        {
            // the reward is finished
            clock.unix_timestamp = 20 * SECONDS_IN_A_DAY as i64;

            let claimed_slnd = user_reward_manager_1
                .claim_rewards(&mut pool_reward_manager, slnd_vault, &clock)
                .expect("It claims rewards");
            assert_eq!(claimed_slnd, 25 * 1_000_000);

            let claimed_slnd = user_reward_manager_2
                .claim_rewards(&mut pool_reward_manager, slnd_vault, &clock)
                .expect("It claims rewards");
            assert_eq!(claimed_slnd, 25 * 1_000_000);
        }
    }

    /// This tests replicates calculations from Suilend's
    /// "test_pool_reward_manager_multiple_rewards" test.
    #[test]
    fn it_tests_pool_reward_manager_multiple_rewards() {
        let usdc = Pubkey::new_unique(); // reserve pubkey
        let slnd_vault1 = Pubkey::new_unique(); // where rewards are stored
        let slnd_vault2 = Pubkey::new_unique(); // where rewards are stored

        let mut clock = Clock {
            unix_timestamp: 0,
            ..Default::default()
        };

        let mut pool_reward_manager = PoolRewardManager::default();
        {
            // setup a reward that starts now and lasts for 20 days

            pool_reward_manager
                .add_pool_reward(
                    slnd_vault1,
                    0,
                    20 * SECONDS_IN_A_DAY,
                    100 * 1_000_000,
                    &clock,
                )
                .expect("It adds pool reward");

            // and another reward that starts in 10 days and lasts for 10 days

            pool_reward_manager
                .add_pool_reward(
                    slnd_vault2,
                    10 * SECONDS_IN_A_DAY,
                    20 * SECONDS_IN_A_DAY,
                    100 * 1_000_000,
                    &clock,
                )
                .expect("It adds pool reward");
        }

        let mut user_reward_manager_1 = UserRewardManager::new(usdc, PositionKind::Deposit, &clock);
        {
            // setup user reward manager with 100/100 shares

            user_reward_manager_1
                .populate(&mut pool_reward_manager, &clock)
                .expect("It populates user reward manager");
            user_reward_manager_1.set_share(&mut pool_reward_manager, 100);
        }

        clock.unix_timestamp = 15 * SECONDS_IN_A_DAY as i64;

        let mut user_reward_manager_2 = UserRewardManager::new(usdc, PositionKind::Deposit, &clock);
        {
            // setup user reward manager with 100/200 shares

            user_reward_manager_2
                .populate(&mut pool_reward_manager, &clock)
                .expect("It populates user reward manager");
            user_reward_manager_2.set_share(&mut pool_reward_manager, 100);
        }

        {
            clock.unix_timestamp = 30 * SECONDS_IN_A_DAY as i64;

            let claimed_slnd = user_reward_manager_1
                .claim_rewards(&mut pool_reward_manager, slnd_vault1, &clock)
                .expect("It claims rewards");
            assert_eq!(claimed_slnd, 87_500_000);

            let claimed_slnd = user_reward_manager_1
                .claim_rewards(&mut pool_reward_manager, slnd_vault2, &clock)
                .expect("It claims rewards");
            assert_eq!(claimed_slnd, 75 * 1_000_000);

            let claimed_slnd = user_reward_manager_2
                .claim_rewards(&mut pool_reward_manager, slnd_vault1, &clock)
                .expect("It claims rewards");
            assert_eq!(claimed_slnd, 12_500_000);

            let claimed_slnd = user_reward_manager_2
                .claim_rewards(&mut pool_reward_manager, slnd_vault2, &clock)
                .expect("It claims rewards");
            assert_eq!(claimed_slnd, 25 * 1_000_000);
        }
    }

    /// This tests replicates calculations from Suilend's
    /// "test_pool_reward_manager_zero_share" test.
    #[test]
    fn it_tests_pool_reward_zero_share() {
        let usdc = Pubkey::new_unique(); // reserve pubkey
        let slnd_vault = Pubkey::new_unique(); // where rewards are stored

        let mut clock = Clock {
            unix_timestamp: 0,
            ..Default::default()
        };

        let mut pool_reward_manager = PoolRewardManager::default();
        {
            // setup pool reward manager with one reward

            pool_reward_manager
                .add_pool_reward(
                    slnd_vault,
                    0,
                    20 * SECONDS_IN_A_DAY,
                    100 * 1_000_000,
                    &clock,
                )
                .expect("It adds pool reward");
        }

        clock.unix_timestamp = 10 * SECONDS_IN_A_DAY as i64;
        let mut user_reward_manager_1 = UserRewardManager::new(usdc, PositionKind::Deposit, &clock);
        user_reward_manager_1
            .populate(&mut pool_reward_manager, &clock)
            .expect("It populates user reward manager");
        user_reward_manager_1.set_share(&mut pool_reward_manager, 1);

        clock.unix_timestamp = 20 * SECONDS_IN_A_DAY as i64;
        let claimed_slnd = user_reward_manager_1
            .claim_rewards(&mut pool_reward_manager, slnd_vault, &clock)
            .expect("It claims rewards");
        // 50 usdc is unallocated since there was zero share from 0-10 seconds
        assert_eq!(claimed_slnd, 50 * 1_000_000);
    }

    /// This tests replicates calculations from Suilend's
    /// "test_pool_reward_manager_auto_farm" test.
    #[test]
    fn it_tests_pool_reward_manager_auto_farm() {
        let usdc = Pubkey::new_unique(); // reserve pubkey
        let slnd_vault = Pubkey::new_unique(); // where rewards are stored

        let mut clock = Clock {
            unix_timestamp: 0,
            ..Default::default()
        };

        let mut pool_reward_manager = PoolRewardManager::default();

        let mut user_reward_manager_1 = UserRewardManager::new(usdc, PositionKind::Deposit, &clock);
        user_reward_manager_1
            .populate(&mut pool_reward_manager, &clock)
            .expect("It populates user reward manager");
        user_reward_manager_1.set_share(&mut pool_reward_manager, 1);

        pool_reward_manager
            .add_pool_reward(
                slnd_vault,
                0,
                20 * SECONDS_IN_A_DAY,
                100 * 1_000_000,
                &clock,
            )
            .expect("It adds pool reward");

        clock.unix_timestamp = 10 * SECONDS_IN_A_DAY as i64;

        let mut user_reward_manager_2 = UserRewardManager::new(usdc, PositionKind::Deposit, &clock);
        user_reward_manager_2
            .populate(&mut pool_reward_manager, &clock)
            .expect("It populates user reward manager");
        user_reward_manager_2.set_share(&mut pool_reward_manager, 1);

        {
            clock.unix_timestamp = 20 * SECONDS_IN_A_DAY as i64;

            let claimed_slnd = user_reward_manager_1
                .claim_rewards(&mut pool_reward_manager, slnd_vault, &clock)
                .expect("It claims rewards");
            assert_eq!(claimed_slnd, 75 * 1_000_000);

            user_reward_manager_2.set_share(&mut pool_reward_manager, 1);
            let claimed_slnd = user_reward_manager_2
                .claim_rewards(&mut pool_reward_manager, slnd_vault, &clock)
                .expect("It claims rewards");
            assert_eq!(claimed_slnd, 25 * 1_000_000);
        }
    }

    /// This tests replicates Suilend's "test_add_too_many_pool_rewards" test.
    #[test]
    fn it_tests_add_too_many_pool_rewards() {
        let clock = Clock::default();

        let mut pool_reward_manager = PoolRewardManager::default();

        for _ in 0..MAX_REWARDS {
            let slnd_vault = Pubkey::new_unique(); // where rewards are stored
            pool_reward_manager
                .add_pool_reward(
                    slnd_vault,
                    0,
                    20 * SECONDS_IN_A_DAY,
                    100 * 1_000_000,
                    &clock,
                )
                .expect("It adds pool reward");
        }

        pool_reward_manager
            .add_pool_reward(
                Pubkey::new_unique(),
                0,
                20 * SECONDS_IN_A_DAY,
                100 * 1_000_000,
                &clock,
            )
            .expect_err("It fails to add pool reward");
    }

    /// This tests replicates Suilend's
    /// "test_pool_reward_manager_cancel_and_close" test.
    #[test]
    fn it_tests_pool_reward_manager_cancel_and_close() {
        let usdc = Pubkey::new_unique(); // reserve pubkey
        let slnd_vault = Pubkey::new_unique(); // where rewards are stored

        let mut clock = Clock {
            unix_timestamp: 0,
            ..Default::default()
        };

        let mut pool_reward_manager = PoolRewardManager::default();

        pool_reward_manager
            .add_pool_reward(
                slnd_vault,
                0,
                20 * SECONDS_IN_A_DAY,
                100 * 1_000_000,
                &clock,
            )
            .expect("It adds pool reward");

        let mut user_reward_manager_1 = UserRewardManager::new(usdc, PositionKind::Deposit, &clock);
        user_reward_manager_1
            .populate(&mut pool_reward_manager, &clock)
            .expect("It populates user reward manager");
        user_reward_manager_1.set_share(&mut pool_reward_manager, 100);

        {
            clock.unix_timestamp = 10 * SECONDS_IN_A_DAY as i64;

            let (from_vault, unallocated_rewards) = pool_reward_manager
                .cancel_pool_reward(0, &clock)
                .expect("It cancels pool reward");
            assert_eq!(from_vault, slnd_vault);
            assert_eq!(unallocated_rewards, 50 * 1_000_000);
        }

        {
            clock.unix_timestamp = 15 * SECONDS_IN_A_DAY as i64;

            let claimed_slnd = user_reward_manager_1
                .claim_rewards(&mut pool_reward_manager, slnd_vault, &clock)
                .expect("It claims rewards");
            assert_eq!(claimed_slnd, 50 * 1_000_000);
        }

        let from_vault = pool_reward_manager
            .close_pool_reward(0)
            .expect("It closes pool reward");
        assert_eq!(from_vault, slnd_vault);
    }

    /// This tests replicates Suilend's
    /// "test_pool_reward_manager_cancel_and_close_regression" test.
    #[test]
    fn it_tests_pool_reward_manager_cancel_and_close_regression() {
        let usdc = Pubkey::new_unique(); // reserve pubkey
        let slnd_vault1 = Pubkey::new_unique(); // where rewards are stored
        let slnd_vault2 = Pubkey::new_unique(); // where rewards are stored

        let mut clock = Clock {
            unix_timestamp: 0,
            ..Default::default()
        };

        let mut pool_reward_manager = PoolRewardManager::default();

        pool_reward_manager
            .add_pool_reward(
                slnd_vault1,
                0,
                20 * SECONDS_IN_A_DAY,
                100 * 1_000_000,
                &clock,
            )
            .expect("It adds pool reward");

        pool_reward_manager
            .add_pool_reward(
                slnd_vault2,
                20 * SECONDS_IN_A_DAY,
                30 * SECONDS_IN_A_DAY,
                100 * 1_000_000,
                &clock,
            )
            .expect("It adds pool reward");

        let mut user_reward_manager_1 = UserRewardManager::new(usdc, PositionKind::Deposit, &clock);
        user_reward_manager_1
            .populate(&mut pool_reward_manager, &clock)
            .expect("It populates user reward manager");
        user_reward_manager_1.set_share(&mut pool_reward_manager, 100);

        {
            clock.unix_timestamp = 10 * SECONDS_IN_A_DAY as i64;

            let (from_vault, unallocated_rewards) = pool_reward_manager
                .cancel_pool_reward(0, &clock)
                .expect("It cancels pool reward");
            assert_eq!(from_vault, slnd_vault1);
            assert_eq!(unallocated_rewards, 50 * 1_000_000);

            clock.unix_timestamp = 15 * SECONDS_IN_A_DAY as i64;
            let claim_slnd = user_reward_manager_1
                .claim_rewards(&mut pool_reward_manager, slnd_vault1, &clock)
                .expect("It claims rewards");
            assert_eq!(claim_slnd, 50 * 1_000_000);

            let from_vault = pool_reward_manager
                .close_pool_reward(0)
                .expect("It closes pool reward");
            assert_eq!(from_vault, slnd_vault1);
        }

        clock.unix_timestamp = 20 * SECONDS_IN_A_DAY as i64;

        let mut user_reward_manager_2 = UserRewardManager::new(usdc, PositionKind::Deposit, &clock);
        user_reward_manager_2
            .populate(&mut pool_reward_manager, &clock)
            .expect("It populates user reward manager");
        user_reward_manager_2.set_share(&mut pool_reward_manager, 100);

        {
            clock.unix_timestamp = 30 * SECONDS_IN_A_DAY as i64;

            let claimed_slnd = user_reward_manager_2
                .claim_rewards(&mut pool_reward_manager, slnd_vault2, &clock)
                .expect("It claims rewards");
            assert_eq!(claimed_slnd, 50 * 1_000_000);
        }
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

    impl UserRewardManager {
        pub(crate) fn new_rand(rng: &mut impl Rng) -> Self {
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
}

use crate::math::Decimal;
use solana_program::pubkey::Pubkey;

/// Determines the size of [PoolRewardManager]
const MAX_REWARDS: usize = 44;

/// Each reserve has two managers:
/// - one for deposits
/// - one for borrows
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PoolRewardId(pub u32);

/// # (Un)Packing
/// This is unpacked representation.
/// When packing we use the [PoolReward] `reward_mint` to determine whether the
/// reward is vacant or not to save space.
///
/// If the pubkey is eq to default pubkey then slot is vacant.
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
    /// Amount of rewards that have been made available to users.
    ///
    /// We keep adding `(total_rewards * time_passed) / (total_time)` every
    /// time someone interacts with the manager
    /// ([update_pool_reward_manager]).
    pub allocated_rewards: Decimal,
    /// We keep adding `(unlocked_rewards) / (total_shares)` every time
    /// someone interacts with the manager ([update_pool_reward_manager])
    /// where
    /// `unlocked_rewards = (total_rewards * time_passed) / (total_time)`
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_fits_reserve_realloc_into_single_ix() {
        const MAX_REALLOC: usize = 10 * 1024;

        let size_of_discriminant = 1;
        let const_size_of_pool_manager = 8 + 8;
        let required_realloc = size_of_discriminant
            + const_size_of_pool_manager
            + 2 * MAX_REWARDS * std::mem::size_of::<PoolReward>();

        println!("assert {required_realloc} <= {MAX_REALLOC}");
        assert!(required_realloc <= MAX_REALLOC);
    }
}

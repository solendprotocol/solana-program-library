//! Liquidity mining feature built analogous to Suilend's implementation.

pub mod pool_reward_manager;
pub mod user_reward_manager;

pub use pool_reward_manager::*;
pub use user_reward_manager::*;

/// Determines the size of [PoolRewardManager]
pub const MAX_REWARDS: usize = 50;

/// Cannot create a reward shorter than this.
pub const MIN_REWARD_PERIOD_SECS: u64 = 3_600;

#[cfg(test)]
mod suilend_tests {
    //! These tests were taken from the Suilend's codebase and adapted to
    //! the new codebase.
    //!
    //! TODO: Calculate test coverage and add tests for missing branches.

    use crate::{
        math::Decimal,
        state::{
            PoolReward, PoolRewardId, PoolRewardManager, PoolRewardSlot, PositionKind,
            UserRewardManager, MAX_REWARDS,
        },
    };
    use pretty_assertions::assert_eq;
    use solana_program::{clock::Clock, pubkey::Pubkey};

    const SECONDS_IN_A_DAY: u64 = 86_400;

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
}

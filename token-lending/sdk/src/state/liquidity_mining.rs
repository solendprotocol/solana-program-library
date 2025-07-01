//! Liquidity mining feature built analogous to Suilend's implementation.

pub mod pool_reward_manager;
pub mod user_reward_manager;

pub use pool_reward_manager::*;
pub use user_reward_manager::*;

/// Determines the size of [PoolRewardManager].
///
/// On Suilend this is 50.
/// However, Sui dynamic object model let's us store more data easily.
/// In Save we're storing the data on the reserve and this means packing and
/// unpacking it frequently which negatively impacts CU limits.
///
/// In Save, if we want to add new rewards we will crank old ones to make space
/// in the reserve.
pub const MAX_REWARDS: usize = 30;

/// Cannot create a reward shorter than this.
pub const MIN_REWARD_PERIOD_SECS: u32 = 3_600;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{TryDiv, TryMul};
    use crate::{
        error::LendingError,
        math::Decimal,
        state::{PoolRewardManager, PositionKind, UserRewardManager},
    };
    use pretty_assertions::assert_eq;
    use proptest::prelude::*;
    use rand::prelude::*;
    use rand_chacha::ChaCha8Rng;
    use solana_program::{clock::Clock, msg, program_error::ProgramError, pubkey::Pubkey};
    use std::convert::TryFrom;

    /// This test asserts that cancelling a reward does not change the amount of rewards that are
    /// emitted to a user.
    #[test]
    fn it_cancels_reward_without_changing_user_eligible_amount() {
        // This is an implementation that was superseded by the edit function.
        // We show that cancelling is a special case of editing.
        impl PoolRewardManager {
            fn cancel_pool_reward(
                &mut self,
                pool_reward_index: usize,
                clock: &Clock,
            ) -> Result<(Pubkey, u64), ProgramError> {
                self.update(clock)?;

                let Some(PoolRewardEntry::Occupied(pool_reward)) =
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
        }

        let usdc = Pubkey::new_unique(); // reserve pubkey
        let slnd_vault = Pubkey::new_unique(); // where rewards are stored
        let reward_period = 10 * MIN_REWARD_PERIOD_SECS as u64;
        let total_rewards = 100 * 1_000_000;

        let mut clock = Clock {
            unix_timestamp: 0,
            ..Default::default()
        };

        let mut pool_reward_manager = PoolRewardManager::default();

        let mut user_reward_manager = UserRewardManager::new(usdc, PositionKind::Deposit, &clock);
        user_reward_manager
            .populate(&mut pool_reward_manager, &clock)
            .expect("It populates user reward manager");
        user_reward_manager.set_share(&mut pool_reward_manager, 1);

        pool_reward_manager
            .add_pool_reward(slnd_vault, 0, reward_period, total_rewards, &clock)
            .expect("It adds pool reward");

        clock.unix_timestamp = reward_period as i64 / 2;

        let (_, not_cancelled_claimed_slnd) = {
            let mut pool_reward_manager = pool_reward_manager.clone();
            let mut user_reward_manager = user_reward_manager.clone();

            user_reward_manager
                .claim_rewards(&mut pool_reward_manager, slnd_vault, &clock)
                .expect("It claims rewards")
        };

        let (_, edited_claimed_slnd) = {
            let mut pool_reward_manager = pool_reward_manager.clone();
            let mut user_reward_manager = user_reward_manager.clone();

            let pool_reward_index = 0;
            let end_now = 0; // should be same as cancel really
            pool_reward_manager
                .edit_pool_reward(pool_reward_index, end_now, &clock)
                .expect("It edits pool reward");

            user_reward_manager
                .claim_rewards(&mut pool_reward_manager, slnd_vault, &clock)
                .expect("It claims rewards")
        };

        let (_, canceled_claimed_slnd) = {
            let pool_reward_index = 0;
            pool_reward_manager
                .cancel_pool_reward(pool_reward_index, &clock)
                .expect("It cancels pool reward");

            user_reward_manager
                .claim_rewards(&mut pool_reward_manager, slnd_vault, &clock)
                .expect("It claims rewards")
        };

        assert_eq!(not_cancelled_claimed_slnd, canceled_claimed_slnd);
        assert_eq!(edited_claimed_slnd, canceled_claimed_slnd);
    }

    #[test]
    fn it_extends_reward() {
        let slnd_vault = Pubkey::new_unique(); // where rewards are stored
        let reward_period = 10 * MIN_REWARD_PERIOD_SECS as u64;
        let total_rewards = 100 * 1_000_000;

        let clock = Clock {
            unix_timestamp: 0,
            ..Default::default()
        };

        let mut pool_reward_manager = PoolRewardManager::default();

        pool_reward_manager
            .add_pool_reward(slnd_vault, 0, reward_period, total_rewards, &clock)
            .expect("It adds pool reward");

        let pool_reward_index = 0;
        let (vault, amount) = pool_reward_manager
            .edit_pool_reward(pool_reward_index, reward_period * 2, &clock)
            .expect("It edits pool reward");

        assert!(amount.is_positive());
        assert_eq!(amount as u64, total_rewards);
        assert_eq!(vault, slnd_vault);

        let PoolRewardEntry::Occupied(pool_reward) =
            &pool_reward_manager.pool_rewards[pool_reward_index]
        else {
            panic!("Expected pool reward to be occupied");
        };

        assert_eq!(pool_reward.total_rewards, total_rewards * 2);
        assert_eq!(pool_reward.duration_secs, reward_period as u32 * 2);
    }

    proptest! {
        #[test]
        fn it_yields_expected_rewards_if_edited(
            rng_seed in 0..u64::MAX,
            user_count in 1..10usize,
            reward_period in MIN_REWARD_PERIOD_SECS..1000*MIN_REWARD_PERIOD_SECS,
            total_rewards in 1_000_000..10_000_000_000_000u64,
        ) {
            let mut rng = ChaCha8Rng::seed_from_u64(rng_seed);

            let usdc = Pubkey::new_unique();
            let position_kind = PositionKind::Deposit;
            let foo_vault = Pubkey::new_unique(); // this one is not edited
            let bar_vault = Pubkey::new_unique(); // we'll edit this one

            let edit_reward_after_timestamp = rng.gen_range(
                0..reward_period as i64 / 2,
            );
            let edit_bar_to_end_at_timestamp = rng.gen_range(
                0..(reward_period * 2) as i64,
            );

            let mut clock = Clock {
                unix_timestamp: 0,
                ..Default::default()
            };

            let mut total_claimed_foo = 0;
            let mut total_claimed_bar = 0;

            let mut pool_reward_manager = PoolRewardManager::default();

            // both rewards start identically

            pool_reward_manager
                .add_pool_reward(foo_vault, 0, reward_period as _, total_rewards, &clock)
                .expect("It adds pool reward");

            pool_reward_manager
                .add_pool_reward(bar_vault, 0, reward_period as _, total_rewards, &clock)
                .expect("It adds pool reward");

            // all users start tracking the rewards with their respective shares

            let mut user_reward_managers: Vec<_> = (0..user_count)
                .map(|_| {
                    let mut user_reward_manager = UserRewardManager::new(usdc, position_kind, &clock);

                    user_reward_manager
                        .populate(&mut pool_reward_manager, &clock)
                        .expect("It populates user reward manager");

                    let user_share = rng.gen_range(0..1_000);
                    user_reward_manager.set_share(&mut pool_reward_manager, user_share);
                    user_reward_manager
                })
                .collect();

            while clock.unix_timestamp < edit_reward_after_timestamp {
                clock.unix_timestamp += rng.gen_range(0..MIN_REWARD_PERIOD_SECS) as i64;

                for user_reward_manager in &mut user_reward_managers {
                    let (_, claimed_foo) = user_reward_manager
                        .claim_rewards(&mut pool_reward_manager, foo_vault, &clock)
                        .expect("It claims foo rewards");

                    let (_, claimed_bar) = user_reward_manager
                        .claim_rewards(&mut pool_reward_manager, bar_vault, &clock)
                        .expect("It claims bar rewards");

                    prop_assert_eq!(claimed_foo, claimed_bar);

                    total_claimed_foo += claimed_foo;
                    total_claimed_bar += claimed_bar;
                }
            }

            // edit the second reward

            let bar_reward_index = 1;
            let (_, change_in_bar_reward) = pool_reward_manager
                .edit_pool_reward(bar_reward_index, edit_bar_to_end_at_timestamp as _, &clock)
                .expect("It edits bar pool reward");

            // now keep claiming until both rewards end

            loop {
                clock.unix_timestamp += rng.gen_range(0..MIN_REWARD_PERIOD_SECS) as i64;

                let has_foo_ended = match &pool_reward_manager.pool_rewards[0] {
                    PoolRewardEntry::Occupied(pool_reward) => pool_reward.has_ended(&clock),
                    _ => unreachable!(),
                };

                let has_bar_ended = match &pool_reward_manager.pool_rewards[1] {
                    PoolRewardEntry::Occupied(pool_reward) => pool_reward.has_ended(&clock),
                    _ => unreachable!(),
                };

                let neither_has_ended = !has_foo_ended && !has_bar_ended;

                for user_reward_manager in &mut user_reward_managers {
                    let (_, claimed_foo) = user_reward_manager
                        .claim_rewards(&mut pool_reward_manager, foo_vault, &clock)
                        .expect("It claims foo rewards");

                    let (_, claimed_bar) = user_reward_manager
                        .claim_rewards(&mut pool_reward_manager, bar_vault, &clock)
                        .expect("It claims bar rewards");

                    total_claimed_foo += claimed_foo;
                    total_claimed_bar += claimed_bar;

                    if neither_has_ended && claimed_foo != claimed_bar {
                        // due to rounding errors we can be a little off
                        let allowed_diff = 1;
                        let allowed_min = claimed_foo.saturating_sub(allowed_diff);
                        let allowed_range = allowed_min..=(claimed_foo + allowed_diff);
                        prop_assert!(
                            allowed_range.contains(&claimed_bar),
                            "Expected foo and bar rewards to equal, but got {} and {}",
                            claimed_foo,
                            claimed_bar
                        );
                    }
                }

                if has_foo_ended && has_bar_ended {
                    break;
                }
            }

            // check that no more rewards can be claimed

            for user_reward_manager in &mut user_reward_managers {
                let (_, claimed_foo) = user_reward_manager
                    .claim_rewards(&mut pool_reward_manager, foo_vault, &clock)
                    .expect("It claims foo rewards");
                prop_assert_eq!(claimed_foo, 0);

                let (_, claimed_bar) = user_reward_manager
                    .claim_rewards(&mut pool_reward_manager, bar_vault, &clock)
                    .expect("It claims bar rewards");
                prop_assert_eq!(claimed_bar, 0);
            }

            // check that the end state is what we'd expect

            // User's claimed no more than total_rewards and not much less either.
            // Due to rounding issues we're ok with distributing one less token per user.
            let max_allowed_diff = user_count as u64;

            let foo_allowed_range = (total_rewards - max_allowed_diff)..=total_rewards;
            prop_assert!(
                foo_allowed_range.contains(&total_claimed_foo),
                "Foo claimed rewards {} not close to total rewards of {}..={}",
                total_claimed_foo,
                total_rewards - max_allowed_diff,
                total_rewards
            );

            let expected_bar_total_rewards = (total_rewards as i64 + change_in_bar_reward) as u64;
            let bar_allowed_range =
                (expected_bar_total_rewards - max_allowed_diff)..=expected_bar_total_rewards;
            prop_assert!(
                bar_allowed_range.contains(&total_claimed_bar),
                "Bar claimed rewards {} not close to total rewards of {}..={}",
                total_claimed_bar,
                expected_bar_total_rewards - max_allowed_diff,
                expected_bar_total_rewards
            );
        }
    }
}

#[cfg(test)]
mod suilend_tests {
    //! These tests were taken from the Suilend's codebase and adapted to
    //! the new codebase.

    use crate::{
        math::Decimal,
        state::{
            PoolReward, PoolRewardEntry, PoolRewardId, PoolRewardManager, PositionKind,
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
                PoolRewardEntry::Occupied(Box::new(PoolReward {
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

            let (_, claimed_slnd) = user_reward_manager_1
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

            let (_, claimed_slnd) = user_reward_manager_1
                .claim_rewards(&mut pool_reward_manager, slnd_vault, &clock)
                .expect("It claims rewards");
            assert_eq!(claimed_slnd, 5 * 1_000_000);

            let (_, claimed_slnd) = user_reward_manager_2
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

            let (_, claimed_slnd) = user_reward_manager_1
                .claim_rewards(&mut pool_reward_manager, slnd_vault, &clock)
                .expect("It claims rewards");
            assert_eq!(claimed_slnd, 25 * 1_000_000);

            let (_, claimed_slnd) = user_reward_manager_2
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

            let (_, claimed_slnd) = user_reward_manager_1
                .claim_rewards(&mut pool_reward_manager, slnd_vault1, &clock)
                .expect("It claims rewards");
            assert_eq!(claimed_slnd, 87_500_000);

            let (_, claimed_slnd) = user_reward_manager_1
                .claim_rewards(&mut pool_reward_manager, slnd_vault2, &clock)
                .expect("It claims rewards");
            assert_eq!(claimed_slnd, 75 * 1_000_000);

            let (_, claimed_slnd) = user_reward_manager_2
                .claim_rewards(&mut pool_reward_manager, slnd_vault1, &clock)
                .expect("It claims rewards");
            assert_eq!(claimed_slnd, 12_500_000);

            let (_, claimed_slnd) = user_reward_manager_2
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
        let (_, claimed_slnd) = user_reward_manager_1
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

            let (_, claimed_slnd) = user_reward_manager_1
                .claim_rewards(&mut pool_reward_manager, slnd_vault, &clock)
                .expect("It claims rewards");
            assert_eq!(claimed_slnd, 75 * 1_000_000);

            user_reward_manager_2.set_share(&mut pool_reward_manager, 1);
            let (_, claimed_slnd) = user_reward_manager_2
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

            let pool_reward_index = 0;
            let new_end_time_secs = 0; // now
            let (from_vault, unallocated_rewards) = pool_reward_manager
                .edit_pool_reward(pool_reward_index, new_end_time_secs, &clock)
                .expect("It cancels pool reward");
            assert_eq!(from_vault, slnd_vault);
            assert_eq!(unallocated_rewards, -50 * 1_000_000 + 1); // approx
        }

        {
            clock.unix_timestamp = 15 * SECONDS_IN_A_DAY as i64;

            let (_, claimed_slnd) = user_reward_manager_1
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

            let pool_reward_index = 0;
            let new_end_time_secs = 0; // now
            let (from_vault, unallocated_rewards) = pool_reward_manager
                .edit_pool_reward(pool_reward_index, new_end_time_secs, &clock)
                .expect("It cancels pool reward");
            assert_eq!(from_vault, slnd_vault1);
            assert_eq!(unallocated_rewards, -50 * 1_000_000 + 1); // approx

            clock.unix_timestamp = 15 * SECONDS_IN_A_DAY as i64;
            let (_, claim_slnd) = user_reward_manager_1
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

            let (_, claimed_slnd) = user_reward_manager_2
                .claim_rewards(&mut pool_reward_manager, slnd_vault2, &clock)
                .expect("It claims rewards");
            assert_eq!(claimed_slnd, 50 * 1_000_000);
        }
    }
}

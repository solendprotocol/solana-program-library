#![cfg(feature = "test-bpf")]

mod helpers;

use std::collections::HashSet;

use helpers::solend_program_test::{
    setup_world, BalanceChecker, LiqMiningReward, TokenAccount, TokenBalanceChange,
};
use helpers::test_reserve_config;

use pretty_assertions::assert_eq;
use solana_program_test::*;
use solana_sdk::signature::{Keypair, Signer};
use solend_program::{
    math::Decimal,
    state::{PoolRewardId, PoolRewardManager, PositionKind, Reserve},
};
use solend_sdk::state::{PoolReward, PoolRewardEntry};

#[tokio::test]
async fn test_cancel_pool_reward_for_deposit() {
    test_cancel_(PositionKind::Deposit).await;
}

#[tokio::test]
async fn test_cancel_pool_reward_for_borrow() {
    test_cancel_(PositionKind::Borrow).await;
}

#[tokio::test]
async fn test_extend_pool_reward_for_deposit() {
    test_extend_(PositionKind::Deposit).await;
}

#[tokio::test]
async fn test_extend_pool_reward_for_borrow() {
    test_extend_(PositionKind::Borrow).await;
}

async fn test_cancel_(position_kind: PositionKind) {
    let (mut test, lending_market, usdc_reserve, _, mut lending_market_owner, _) =
        setup_world(&test_reserve_config(), &test_reserve_config()).await;

    let reward_mint = test.create_mint_as_test_authority().await;
    let reward_vault = Keypair::new();
    let duration_secs = 10 * 3_600;
    let total_rewards = 1_000_000;
    let initial_time = test.get_clock().await.unix_timestamp as u64;
    let reward = LiqMiningReward {
        mint: reward_mint,
        vault: reward_vault.insecure_clone(),
    };

    lending_market
        .add_pool_reward(
            &mut test,
            &usdc_reserve,
            &mut lending_market_owner,
            &reward,
            position_kind,
            initial_time,
            initial_time + duration_secs as u64,
            total_rewards,
        )
        .await
        .expect("Should add pool reward");

    let balance_checker = BalanceChecker::start(
        &mut test,
        &[&TokenAccount(reward.vault.pubkey()), &lending_market_owner],
    )
    .await;

    let current_time = test
        .advance_clock_by_slots_and_secs(1, duration_secs as u64 / 2)
        .await;

    let pool_reward_index = 0;
    lending_market
        .edit_pool_reward(
            &mut test,
            &usdc_reserve,
            &mut lending_market_owner,
            &reward,
            position_kind,
            pool_reward_index,
            0, // cancel
        )
        .await
        .expect("Should cancel pool reward");

    let (balance_changes, _) = balance_checker.find_balance_changes(&mut test).await;

    let diff = (total_rewards as i128) / 2 - 1;
    let expected_balance_changes = HashSet::from([
        TokenBalanceChange {
            token_account: reward.vault.pubkey(),
            mint: reward.mint,
            diff: -diff,
        },
        TokenBalanceChange {
            token_account: lending_market_owner.get_account(&reward.mint).unwrap(),
            mint: reward.mint,
            diff,
        },
    ]);
    assert_eq!(balance_changes, expected_balance_changes);

    let usdc_reserve_post = test.load_account::<Reserve>(usdc_reserve.pubkey).await;

    let expected_reward_manager = Box::new(PoolRewardManager {
        total_shares: 0,
        last_update_time_secs: current_time,
        pool_rewards: {
            let mut og = PoolRewardManager::default().pool_rewards;

            og[0] = PoolRewardEntry::Occupied(Box::new(PoolReward {
                id: PoolRewardId(1),
                vault: reward_vault.pubkey(),
                start_time_secs: initial_time,
                duration_secs: duration_secs / 2,
                total_rewards: total_rewards - diff as u64,
                num_user_reward_managers: 0,
                cumulative_rewards_per_share: Decimal::zero(),
            }));

            og
        },
    });

    match position_kind {
        PositionKind::Deposit => {
            assert_eq!(
                usdc_reserve_post.account,
                Reserve {
                    deposits_pool_reward_manager: expected_reward_manager,
                    ..usdc_reserve.clone().account
                }
            );
        }
        PositionKind::Borrow => {
            assert_eq!(
                usdc_reserve_post.account,
                Reserve {
                    borrows_pool_reward_manager: expected_reward_manager,
                    ..usdc_reserve.clone().account
                }
            );
        }
    }
}

async fn test_extend_(position_kind: PositionKind) {
    let (mut test, lending_market, usdc_reserve, _, mut lending_market_owner, _) =
        setup_world(&test_reserve_config(), &test_reserve_config()).await;

    let reward_mint = test.create_mint_as_test_authority().await;
    let reward_vault = Keypair::new();
    let duration_secs = 10 * 3_600u32;
    let total_rewards = 1_000_000;
    let initial_time = test.get_clock().await.unix_timestamp as u64;
    let reward = LiqMiningReward {
        mint: reward_mint,
        vault: reward_vault.insecure_clone(),
    };

    lending_market
        .add_pool_reward(
            &mut test,
            &usdc_reserve,
            &mut lending_market_owner,
            &reward,
            position_kind,
            initial_time,
            initial_time + duration_secs as u64,
            total_rewards,
        )
        .await
        .expect("Should add pool reward");

    let lending_market_owner_reward_token_account =
        lending_market_owner.get_account(&reward.mint).unwrap();

    let current_time = test
        .advance_clock_by_slots_and_secs(1, duration_secs as u64 / 2)
        .await;

    test.mint_to(
        &reward.mint,
        &lending_market_owner_reward_token_account,
        total_rewards,
    )
    .await;

    let balance_checker = BalanceChecker::start(
        &mut test,
        &[&TokenAccount(reward.vault.pubkey()), &lending_market_owner],
    )
    .await;

    let pool_reward_index = 0;
    lending_market
        .edit_pool_reward(
            &mut test,
            &usdc_reserve,
            &mut lending_market_owner,
            &reward,
            position_kind,
            pool_reward_index,
            initial_time + duration_secs as u64 * 2, // twice as long
        )
        .await
        .expect("Should extend pool reward");

    let (balance_changes, _) = balance_checker.find_balance_changes(&mut test).await;

    let expected_balance_changes = HashSet::from([
        TokenBalanceChange {
            token_account: reward.vault.pubkey(),
            mint: reward.mint,
            diff: total_rewards as i128,
        },
        TokenBalanceChange {
            token_account: lending_market_owner_reward_token_account,
            mint: reward.mint,
            diff: -(total_rewards as i128),
        },
    ]);
    assert_eq!(balance_changes, expected_balance_changes);

    let usdc_reserve_post = test.load_account::<Reserve>(usdc_reserve.pubkey).await;

    let expected_reward_manager = Box::new(PoolRewardManager {
        total_shares: 0,
        last_update_time_secs: current_time,
        pool_rewards: {
            let mut og = PoolRewardManager::default().pool_rewards;

            og[0] = PoolRewardEntry::Occupied(Box::new(PoolReward {
                id: PoolRewardId(1),
                vault: reward_vault.pubkey(),
                start_time_secs: initial_time,
                duration_secs: duration_secs * 2,
                total_rewards: total_rewards * 2,
                num_user_reward_managers: 0,
                cumulative_rewards_per_share: Decimal::zero(),
            }));

            og
        },
    });

    match position_kind {
        PositionKind::Deposit => {
            assert_eq!(
                usdc_reserve_post.account,
                Reserve {
                    deposits_pool_reward_manager: expected_reward_manager,
                    ..usdc_reserve.clone().account
                }
            );
        }
        PositionKind::Borrow => {
            assert_eq!(
                usdc_reserve_post.account,
                Reserve {
                    borrows_pool_reward_manager: expected_reward_manager,
                    ..usdc_reserve.clone().account
                }
            );
        }
    }
}

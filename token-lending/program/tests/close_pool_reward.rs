#![cfg(feature = "test-bpf")]

mod helpers;

use std::collections::HashSet;

use helpers::solend_program_test::{
    setup_world, BalanceChecker, LiqMiningReward, TokenBalanceChange,
};
use helpers::test_reserve_config;

use pretty_assertions::assert_eq;
use solana_program_test::*;
use solana_sdk::signature::Keypair;
use solend_program::state::{PoolRewardId, PoolRewardManager, PositionKind, Reserve};
use solend_sdk::state::PoolRewardSlot;

#[tokio::test]
async fn test_close_pool_reward_for_deposit() {
    test_(PositionKind::Deposit).await;
}

#[tokio::test]
async fn test_close_pool_reward_for_borrow() {
    test_(PositionKind::Borrow).await;
}

async fn test_(position_kind: PositionKind) {
    let (mut test, lending_market, usdc_reserve, _, mut lending_market_owner, _) =
        setup_world(&test_reserve_config(), &test_reserve_config()).await;

    let reward_mint = test.create_mint_as_test_authority().await;
    let reward_vault = Keypair::new();
    let duration_secs = 3_600;
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

    let balance_checker = BalanceChecker::start(&mut test, &[&lending_market_owner]).await;

    // doesn't matter when we close as long as there are no obligations
    test.advance_clock_by_slots_and_secs(1, 1).await;

    let pool_reward_index = 0;
    lending_market
        .close_pool_reward(
            &mut test,
            &usdc_reserve,
            &mut lending_market_owner,
            &reward,
            position_kind,
            pool_reward_index,
        )
        .await
        .expect("Should close pool reward");

    let (balance_changes, _) = balance_checker.find_balance_changes(&mut test).await;

    let expected_balance_changes = HashSet::from([TokenBalanceChange {
        token_account: lending_market_owner.get_account(&reward.mint).unwrap(),
        mint: reward.mint,
        diff: total_rewards as _,
    }]);
    assert_eq!(balance_changes, expected_balance_changes);

    let usdc_reserve_post = test.load_account::<Reserve>(usdc_reserve.pubkey).await;

    let expected_reward_manager = Box::new(PoolRewardManager {
        total_shares: 0,
        last_update_time_secs: initial_time as _,
        pool_rewards: {
            let mut og = PoolRewardManager::default().pool_rewards;

            og[0] = PoolRewardSlot::Vacant {
                last_pool_reward_id: PoolRewardId(1),
                has_been_just_vacated: false,
            };

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

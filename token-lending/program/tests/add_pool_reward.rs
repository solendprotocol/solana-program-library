#![cfg(feature = "test-bpf")]

mod helpers;

use helpers::solend_program_test::{setup_world, LiqMiningReward};
use helpers::test_reserve_config;

use pretty_assertions::assert_eq;
use solana_program_test::*;
use solana_sdk::signature::{Keypair, Signer};
use solend_program::{
    math::Decimal,
    state::{PoolRewardId, PoolRewardManager, PositionKind, Reserve, UserRewardManager},
};
use solend_sdk::state::{PoolReward, PoolRewardSlot, UserReward};

#[tokio::test]
async fn test_add_pool_reward_for_deposit() {
    test_(PositionKind::Deposit).await;
}

#[tokio::test]
async fn test_add_pool_reward_for_borrow() {
    test_(PositionKind::Borrow).await;
}

async fn test_(position_kind: PositionKind) {
    let (mut test, lending_market, usdc_reserve, wsol_reserve, mut lending_market_owner, user) =
        setup_world(&test_reserve_config(), &test_reserve_config()).await;

    let reward_mint = test.create_mint_as_test_authority().await;
    let reward_vault = Keypair::new();
    let duration_secs = 3_600;
    let total_rewards = 1_000_000;
    let current_time = test.get_clock().await.unix_timestamp as u64;

    lending_market
        .add_pool_reward(
            &mut test,
            &usdc_reserve,
            &mut lending_market_owner,
            &LiqMiningReward {
                mint: reward_mint,
                vault: reward_vault.insecure_clone(),
            },
            position_kind,
            current_time,
            current_time + duration_secs as u64,
            total_rewards,
        )
        .await
        .expect("Should add pool reward");

    let obligation = lending_market
        .init_obligation(&mut test, Keypair::new(), &user)
        .await
        .expect("This should succeed");

    let usdc_reserve_post = test.load_account::<Reserve>(usdc_reserve.pubkey).await;

    let expected_reward_manager = Box::new(PoolRewardManager {
        total_shares: 0,
        last_update_time_secs: current_time as _,
        pool_rewards: {
            let mut og = usdc_reserve
                .account
                .deposits_pool_reward_manager
                .pool_rewards
                .clone();

            og[0] = PoolRewardSlot::Occupied(Box::new(PoolReward {
                id: PoolRewardId(1),
                vault: reward_vault.pubkey(),
                start_time_secs: current_time,
                duration_secs,
                total_rewards,
                num_user_reward_managers: 0,
                cumulative_rewards_per_share: Decimal::zero(),
            }));

            og
        },
    });

    let expected_share = match position_kind {
        PositionKind::Deposit => {
            assert_eq!(
                usdc_reserve_post.account,
                Reserve {
                    deposits_pool_reward_manager: expected_reward_manager,
                    ..usdc_reserve.clone().account
                }
            );

            let deposit_amount = 1_000_000;
            lending_market
                .deposit_reserve_liquidity_and_obligation_collateral(
                    &mut test,
                    &usdc_reserve,
                    &obligation,
                    &user,
                    deposit_amount,
                )
                .await
                .expect("This should succeed");

            deposit_amount
        }
        PositionKind::Borrow => {
            assert_eq!(
                usdc_reserve_post.account,
                Reserve {
                    borrows_pool_reward_manager: expected_reward_manager,
                    ..usdc_reserve.clone().account
                }
            );

            lending_market
                .deposit_reserve_liquidity_and_obligation_collateral(
                    &mut test,
                    &wsol_reserve,
                    &obligation,
                    &user,
                    420_000_000,
                )
                .await
                .expect("This should succeed");

            lending_market
                .borrow_obligation_liquidity(
                    &mut test,
                    &usdc_reserve,
                    &obligation,
                    &user,
                    None,
                    690,
                )
                .await
                .unwrap();

            690
        }
    };

    let obligation_post = test.load_obligation(obligation.pubkey).await;

    assert_eq!(
        obligation_post.account.user_reward_managers.last().unwrap(),
        &UserRewardManager {
            reserve: usdc_reserve.pubkey,
            position_kind,
            share: expected_share,
            last_update_time_secs: current_time as _,
            rewards: vec![UserReward {
                pool_reward_index: 0,
                pool_reward_id: PoolRewardId(1),
                earned_rewards: Decimal::zero(),
                cumulative_rewards_per_share: Decimal::zero(),
            }],
        }
    );
}

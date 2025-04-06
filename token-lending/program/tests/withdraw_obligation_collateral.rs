#![cfg(feature = "test-bpf")]

mod helpers;

use crate::solend_program_test::scenario_1;
use helpers::solend_program_test::{BalanceChecker, TokenBalanceChange};
use helpers::*;
use solend_sdk::math::Decimal;

use solana_program_test::*;

use pretty_assertions::assert_eq;
use solend_program::state::{LastUpdate, Obligation, ObligationCollateral, Reserve};
use solend_sdk::state::PoolRewardManager;
use std::collections::HashSet;
use std::u64;

#[tokio::test]
async fn test_success_withdraw_fixed_amount() {
    let (mut test, lending_market, usdc_reserve, wsol_reserve, user, obligation, _) =
        scenario_1(&test_reserve_config(), &test_reserve_config()).await;

    let balance_checker =
        BalanceChecker::start(&mut test, &[&usdc_reserve, &user, &wsol_reserve]).await;

    let withdraw_amount = 1_000_000;

    lending_market
        .withdraw_obligation_collateral(
            &mut test,
            &usdc_reserve,
            &obligation,
            &user,
            withdraw_amount,
        )
        .await
        .unwrap();

    let (balance_changes, mint_supply_changes) =
        balance_checker.find_balance_changes(&mut test).await;
    let expected_balance_changes = HashSet::from([
        TokenBalanceChange {
            token_account: user
                .get_account(&usdc_reserve.account.collateral.mint_pubkey)
                .unwrap(),
            mint: usdc_reserve.account.collateral.mint_pubkey,
            diff: withdraw_amount as _,
        },
        TokenBalanceChange {
            token_account: usdc_reserve.account.collateral.supply_pubkey,
            mint: usdc_reserve.account.collateral.mint_pubkey,
            diff: -(withdraw_amount as i128),
        },
    ]);
    assert_eq!(balance_changes, expected_balance_changes);
    assert_eq!(mint_supply_changes, HashSet::new());

    let usdc_reserve_post = test.load_account::<Reserve>(usdc_reserve.pubkey).await;
    assert_eq!(
        usdc_reserve_post.account,
        Reserve {
            deposits_pool_reward_manager: Box::new(PoolRewardManager {
                total_shares: usdc_reserve
                    .account
                    .deposits_pool_reward_manager
                    .total_shares
                    - withdraw_amount,
                ..*usdc_reserve.account.deposits_pool_reward_manager
            }),
            ..usdc_reserve.account
        }
    );

    let obligation_post = test.load_obligation(obligation.pubkey).await;
    let deposit_reserve = usdc_reserve.pubkey;
    assert_eq!(
        obligation_post.account,
        Obligation {
            last_update: LastUpdate {
                slot: 1000,
                stale: true
            },
            deposits: [ObligationCollateral {
                deposit_reserve,
                deposited_amount: 100_000_000_000 - withdraw_amount,
                market_value: Decimal::from(99_999u64),
                ..obligation.account.deposits[0]
            }]
            .to_vec(),
            deposited_value: Decimal::from(99_999u64),
            user_reward_managers: {
                let mut og = obligation.account.user_reward_managers.clone();

                og.iter_mut()
                    .find(|m| m.reserve == deposit_reserve)
                    .unwrap()
                    .share = usdc_reserve
                    .account
                    .deposits_pool_reward_manager
                    .total_shares
                    - withdraw_amount;

                og
            },
            ..obligation.account
        }
    );
}

#[tokio::test]
async fn test_success_withdraw_max() {
    let (mut test, lending_market, usdc_reserve, wsol_reserve, user, obligation, _) =
        scenario_1(&test_reserve_config(), &test_reserve_config()).await;

    let balance_checker =
        BalanceChecker::start(&mut test, &[&usdc_reserve, &user, &wsol_reserve]).await;

    lending_market
        .withdraw_obligation_collateral(&mut test, &usdc_reserve, &obligation, &user, u64::MAX)
        .await
        .unwrap();

    // we are borrowing 10 SOL @ $10 with an ltv of 0.5, so the debt has to be collateralized by
    // exactly 200cUSDC.
    let sol_borrowed = obligation.account.borrows[0]
        .borrowed_amount_wads
        .try_ceil_u64()
        .unwrap()
        / LAMPORTS_TO_SOL;
    let expected_remaining_collateral = sol_borrowed * 10 * 2 * FRACTIONAL_TO_USDC;

    let (balance_changes, mint_supply_changes) =
        balance_checker.find_balance_changes(&mut test).await;
    let expected_balance_changes = HashSet::from([
        TokenBalanceChange {
            token_account: user
                .get_account(&usdc_reserve.account.collateral.mint_pubkey)
                .unwrap(),
            mint: usdc_reserve.account.collateral.mint_pubkey,
            diff: (100_000 * FRACTIONAL_TO_USDC - expected_remaining_collateral) as i128,
        },
        TokenBalanceChange {
            token_account: usdc_reserve.account.collateral.supply_pubkey,
            mint: usdc_reserve.account.collateral.mint_pubkey,
            diff: -((100_000_000_000 - expected_remaining_collateral) as i128),
        },
    ]);
    assert_eq!(balance_changes, expected_balance_changes);
    assert_eq!(mint_supply_changes, HashSet::new());

    let usdc_reserve_post = test.load_account::<Reserve>(usdc_reserve.pubkey).await;
    assert_eq!(
        usdc_reserve_post.account,
        Reserve {
            deposits_pool_reward_manager: Box::new(PoolRewardManager {
                total_shares: expected_remaining_collateral,
                ..*usdc_reserve.account.deposits_pool_reward_manager
            }),
            ..usdc_reserve.account
        }
    );

    let obligation_post = test.load_obligation(obligation.pubkey).await;
    let deposit_reserve = usdc_reserve.pubkey;
    assert_eq!(
        obligation_post.account,
        Obligation {
            last_update: LastUpdate {
                slot: 1000,
                stale: true
            },
            deposits: [ObligationCollateral {
                deposit_reserve,
                deposited_amount: expected_remaining_collateral,
                market_value: Decimal::from(200u64),
                ..obligation.account.deposits[0]
            }]
            .to_vec(),
            deposited_value: Decimal::from(200u64),
            user_reward_managers: {
                let mut og = obligation.account.user_reward_managers.clone();

                og.iter_mut()
                    .find(|m| m.reserve == deposit_reserve)
                    .unwrap()
                    .share = expected_remaining_collateral;

                og
            },
            ..obligation.account
        }
    );
}

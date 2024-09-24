#![cfg(feature = "test-bpf")]
use crate::solend_program_test::custom_scenario;

use crate::solend_program_test::User;

use crate::solend_program_test::BalanceChecker;

use crate::solend_program_test::PriceArgs;
use crate::solend_program_test::ReserveArgs;
use crate::solend_program_test::TokenBalanceChange;

mod helpers;

use helpers::*;
use solana_program_test::*;

use std::collections::HashSet;

#[tokio::test]
async fn test_donate_to_reserve() {
    let (mut test, lending_market, reserves, _obligations, _users, _) = custom_scenario(
        &[ReserveArgs {
            mint: usdc_mint::id(),
            config: test_reserve_config(),
            liquidity_amount: 100_000 * FRACTIONAL_TO_USDC,
            price: PriceArgs {
                price: 10,
                conf: 0,
                expo: -1,
                ema_price: 10,
                ema_conf: 1,
            },
        }],
        &[],
    )
    .await;

    let whale = User::new_with_balances(
        &mut test,
        &[(&usdc_mint::id(), 100_000 * FRACTIONAL_TO_USDC)],
    )
    .await;

    let balance_checker = BalanceChecker::start(&mut test, &[&whale, &reserves[0]]).await;

    lending_market
        .donate_to_reserve(
            &mut test,
            &reserves[0],
            &whale,
            100_000 * FRACTIONAL_TO_USDC,
        )
        .await
        .unwrap();

    let (balance_changes, _) = balance_checker.find_balance_changes(&mut test).await;
    let expected_balance_changes = HashSet::from([
        TokenBalanceChange {
            token_account: whale.get_account(&usdc_mint::id()).unwrap(),
            mint: usdc_mint::id(),
            diff: -(100_000 * FRACTIONAL_TO_USDC as i128),
        },
        TokenBalanceChange {
            token_account: reserves[0].account.liquidity.supply_pubkey,
            mint: usdc_mint::id(),
            diff: 100_000 * FRACTIONAL_TO_USDC as i128,
        },
    ]);

    assert_eq!(balance_changes, expected_balance_changes);
}

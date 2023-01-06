#![cfg(feature = "test-bpf")]

mod helpers;

use std::collections::HashSet;

use helpers::solend_program_test::{
    BalanceChange, BalanceChecker, Info, PriceArgs, SolendProgramTest, User,
};
use helpers::*;
use solana_program_test::*;
use solend_program::state::{
    LastUpdate, LendingMarket, Reserve, ReserveCollateral, ReserveLiquidity,
};

async fn setup() -> (
    SolendProgramTest,
    Info<LendingMarket>,
    Info<Reserve>,
    User,
    User,
) {
    let mut test = SolendProgramTest::start_new().await;

    let lending_market_owner = User::new_with_balances(
        &mut test,
        &[
            (&usdc_mint::id(), 1_000_000),
            (&wsol_mint::id(), LAMPORTS_TO_SOL),
        ],
    )
    .await;

    let lending_market = test.init_lending_market(&lending_market_owner).await;

    test.advance_clock_by_slots(999).await;

    test.init_pyth_feed(&usdc_mint::id()).await;
    test.set_price(
        &usdc_mint::id(),
        PriceArgs {
            price: 1,
            conf: 0,
            expo: 0,
        },
    )
    .await;

    // add usdc reserve
    let reserve = test
        .init_reserve(
            &lending_market,
            &lending_market_owner,
            &usdc_mint::id(),
            &test_reserve_config(),
            1_000_000,
        )
        .await;

    let user = User::new_with_balances(
        &mut test,
        &[
            (&usdc_mint::id(), 1_000_000),
            (&reserve.account.collateral.mint_pubkey, 0), // cUSDC
        ],
    )
    .await;

    (test, lending_market, reserve, lending_market_owner, user)
}

#[tokio::test]
async fn test_success_new() {
    let (mut test, lending_market, usdc_reserve, _lending_market_owner, user) = setup().await;

    let balance_checker = BalanceChecker::start(&mut test, &[&usdc_reserve, &user]).await;

    // deposit
    lending_market
        .deposit(&mut test, &usdc_reserve, &user, 1_000_000)
        .await;

    // check token balances
    let balance_changes = balance_checker.find_balance_changes(&mut test).await;

    assert_eq!(
        balance_changes,
        HashSet::from([
            BalanceChange {
                token_account: user.get_account(&usdc_mint::id()).await.unwrap(),
                mint: usdc_mint::id(),
                diff: -1_000_000,
            },
            BalanceChange {
                token_account: user
                    .get_account(&usdc_reserve.account.collateral.mint_pubkey)
                    .await
                    .unwrap(),
                mint: usdc_reserve.account.collateral.mint_pubkey,
                diff: 1_000_000,
            },
            BalanceChange {
                token_account: usdc_reserve.account.liquidity.supply_pubkey,
                mint: usdc_reserve.account.liquidity.mint_pubkey,
                diff: 1_000_000,
            },
        ]),
        "{:#?}",
        balance_changes
    );

    // check program state
    let lending_market_post = test
        .load_account::<LendingMarket>(lending_market.pubkey)
        .await;
    assert_eq!(lending_market.account, lending_market_post);

    let usdc_reserve_post = test.load_account::<Reserve>(usdc_reserve.pubkey).await;
    assert_eq!(
        usdc_reserve_post,
        Reserve {
            last_update: LastUpdate {
                slot: 1000,
                stale: true
            },
            liquidity: ReserveLiquidity {
                available_amount: usdc_reserve.account.liquidity.available_amount + 1_000_000,
                ..usdc_reserve.account.liquidity
            },
            collateral: ReserveCollateral {
                mint_total_supply: usdc_reserve.account.collateral.mint_total_supply + 1_000_000,
                ..usdc_reserve.account.collateral
            },
            ..usdc_reserve.account
        }
    );
}

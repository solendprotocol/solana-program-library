#![cfg(feature = "test-bpf")]

mod helpers;

use std::collections::HashSet;

use helpers::solend_program_test::{
    setup_world, BalanceChange, BalanceChecker, Info, SolendProgramTest, User,
};
use helpers::*;
use solana_program::native_token::LAMPORTS_PER_SOL;
use solana_program_test::*;
use solana_sdk::signature::Keypair;
use solend_program::math::TryDiv;
use solend_program::state::{
    LastUpdate, ObligationLiquidity, ReserveFees, ReserveLiquidity, SLOTS_PER_YEAR,
};
use solend_program::{
    math::{Decimal, TryAdd, TryMul, TrySub},
    state::{LendingMarket, Obligation, Reserve, ReserveConfig},
};

async fn setup() -> (
    SolendProgramTest,
    Info<LendingMarket>,
    Info<Reserve>,
    Info<Reserve>,
    User,
    Info<Obligation>,
) {
    let (mut test, lending_market, usdc_reserve, wsol_reserve, lending_market_owner, user) =
        setup_world(
            &ReserveConfig {
                deposit_limit: u64::MAX,
                ..test_reserve_config()
            },
            &ReserveConfig {
                fees: ReserveFees {
                    borrow_fee_wad: 0,
                    host_fee_percentage: 0,
                    flash_loan_fee_wad: 0,
                },
                protocol_take_rate: 0,
                ..test_reserve_config()
            },
        )
        .await;

    // init obligation
    let obligation = lending_market
        .init_obligation(&mut test, Keypair::new(), &user)
        .await
        .expect("This should succeed");

    // deposit 100k USDC
    lending_market
        .deposit(&mut test, &usdc_reserve, &user, 100_000_000_000)
        .await
        .expect("This should succeed");

    let usdc_reserve = test.load_account(usdc_reserve.pubkey).await;

    // deposit 100k cUSDC
    lending_market
        .deposit_obligation_collateral(
            &mut test,
            &usdc_reserve,
            &obligation,
            &user,
            100_000_000_000,
        )
        .await
        .expect("This should succeed");

    let wsol_depositor = User::new_with_balances(
        &mut test,
        &[
            (&wsol_mint::id(), 5 * LAMPORTS_PER_SOL),
            (&wsol_reserve.account.collateral.mint_pubkey, 0),
        ],
    )
    .await;

    // deposit 5SOL. wSOL reserve now has 6 SOL.
    lending_market
        .deposit(
            &mut test,
            &wsol_reserve,
            &wsol_depositor,
            5 * LAMPORTS_PER_SOL,
        )
        .await
        .unwrap();

    // borrow 6 SOL against 100k cUSDC.
    let obligation = test.load_account::<Obligation>(obligation.pubkey).await;
    lending_market
        .borrow_obligation_liquidity(
            &mut test,
            &wsol_reserve,
            &obligation,
            &user,
            &lending_market_owner.get_account(&wsol_mint::id()).unwrap(),
            u64::MAX,
        )
        .await
        .unwrap();

    // populate market price correctly
    lending_market
        .refresh_reserve(&mut test, &wsol_reserve)
        .await
        .unwrap();

    // populate deposit value correctly.
    let obligation = test.load_account::<Obligation>(obligation.pubkey).await;
    lending_market
        .refresh_obligation(&mut test, &obligation)
        .await
        .unwrap();

    let lending_market = test.load_account(lending_market.pubkey).await;
    let usdc_reserve = test.load_account(usdc_reserve.pubkey).await;
    let wsol_reserve = test.load_account(wsol_reserve.pubkey).await;
    let obligation = test.load_account::<Obligation>(obligation.pubkey).await;

    (
        test,
        lending_market,
        usdc_reserve,
        wsol_reserve,
        user,
        obligation,
    )
}

#[tokio::test]
async fn test_success() {
    let (mut test, lending_market, usdc_reserve, wsol_reserve, user, obligation) = setup().await;

    test.advance_clock_by_slots(1).await;

    let balance_checker =
        BalanceChecker::start(&mut test, &[&usdc_reserve, &user, &wsol_reserve]).await;

    lending_market
        .repay_obligation_liquidity(
            &mut test,
            &wsol_reserve,
            &obligation,
            &user,
            6 * LAMPORTS_PER_SOL,
        )
        .await
        .unwrap();

    // check token balances
    let balance_changes = balance_checker.find_balance_changes(&mut test).await;
    let expected_balance_changes = HashSet::from([
        BalanceChange {
            token_account: user.get_account(&wsol_mint::id()).unwrap(),
            mint: wsol_mint::id(),
            diff: -(6 * LAMPORTS_PER_SOL as i128),
        },
        BalanceChange {
            token_account: wsol_reserve.account.liquidity.supply_pubkey,
            mint: wsol_mint::id(),
            diff: (6 * LAMPORTS_PER_SOL as i128),
        },
    ]);
    assert_eq!(balance_changes, expected_balance_changes);

    // check program state
    let wsol_reserve_post = test.load_account::<Reserve>(wsol_reserve.pubkey).await;

    // 1 + 0.3/SLOTS_PER_YEAR
    let new_cumulative_borrow_rate = Decimal::one()
        .try_add(
            Decimal::from_percent(wsol_reserve.account.config.max_borrow_rate)
                .try_div(Decimal::from(SLOTS_PER_YEAR))
                .unwrap(),
        )
        .unwrap();
    let new_borrowed_amount_wads = new_cumulative_borrow_rate
        .try_mul(Decimal::from(6 * LAMPORTS_PER_SOL))
        .unwrap()
        .try_sub(Decimal::from(6 * LAMPORTS_TO_SOL))
        .unwrap();

    assert_eq!(
        wsol_reserve_post.account,
        Reserve {
            last_update: LastUpdate {
                slot: 1001,
                stale: true
            },
            liquidity: ReserveLiquidity {
                available_amount: 6 * LAMPORTS_PER_SOL,
                borrowed_amount_wads: new_borrowed_amount_wads,
                cumulative_borrow_rate_wads: new_cumulative_borrow_rate,
                ..wsol_reserve.account.liquidity
            },
            ..wsol_reserve.account
        }
    );

    let obligation_post = test.load_account::<Obligation>(obligation.pubkey).await;
    assert_eq!(
        obligation_post.account,
        Obligation {
            // we don't require obligation to be refreshed for repay
            last_update: LastUpdate {
                slot: 1000,
                stale: true
            },
            borrows: [ObligationLiquidity {
                borrow_reserve: wsol_reserve.pubkey,
                cumulative_borrow_rate_wads: new_cumulative_borrow_rate,
                borrowed_amount_wads: new_borrowed_amount_wads,
                ..obligation.account.borrows[0]
            }]
            .to_vec(),
            ..obligation.account
        }
    );
}

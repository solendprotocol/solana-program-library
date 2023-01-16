#![cfg(feature = "test-bpf")]

mod helpers;

use helpers::solend_program_test::{
    setup_world, BalanceChange, BalanceChecker, Info, SolendProgramTest, User,
};
use helpers::*;
use solana_program::native_token::LAMPORTS_PER_SOL;
use solana_program_test::*;
use solana_sdk::{
    instruction::InstructionError,
    signature::{Keypair, Signer},
    transaction::{Transaction, TransactionError},
};
use solend_program::state::{
    LastUpdate, LendingMarket, Obligation, ObligationCollateral, ObligationLiquidity, Reserve,
    ReserveConfig, ReserveFees, ReserveLiquidity, SLOTS_PER_YEAR,
};
use solend_program::{
    error::LendingError,
    instruction::{refresh_obligation, withdraw_obligation_collateral},
    processor::process_instruction,
};
use std::collections::HashSet;
use std::u64;

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
async fn test_success_withdraw_fixed_amount() {
    let (mut test, lending_market, usdc_reserve, wsol_reserve, user, obligation) = setup().await;

    let balance_checker =
        BalanceChecker::start(&mut test, &[&usdc_reserve, &user, &wsol_reserve]).await;

    lending_market
        .withdraw_obligation_collateral(&mut test, &usdc_reserve, &obligation, &user, 1_000_000)
        .await
        .unwrap();

    let balance_changes = balance_checker.find_balance_changes(&mut test).await;
    let expected_balance_changes = HashSet::from([
        BalanceChange {
            token_account: user
                .get_account(&usdc_reserve.account.collateral.mint_pubkey)
                .unwrap(),
            mint: usdc_reserve.account.collateral.mint_pubkey,
            diff: 1_000_000,
        },
        BalanceChange {
            token_account: usdc_reserve.account.collateral.supply_pubkey,
            mint: usdc_reserve.account.collateral.mint_pubkey,
            diff: -1_000_000,
        },
    ]);
    assert_eq!(balance_changes, expected_balance_changes);

    let usdc_reserve_post = test.load_account::<Reserve>(usdc_reserve.pubkey).await;
    assert_eq!(usdc_reserve_post.account, usdc_reserve.account);

    let obligation_post = test.load_account::<Obligation>(obligation.pubkey).await;
    assert_eq!(
        obligation_post.account,
        Obligation {
            last_update: LastUpdate {
                slot: 1000,
                stale: true
            },
            deposits: [ObligationCollateral {
                deposit_reserve: usdc_reserve.pubkey,
                deposited_amount: 100_000_000_000 - 1_000_000,
                ..obligation.account.deposits[0]
            }]
            .to_vec(),
            ..obligation.account
        }
    );
}

#[tokio::test]
async fn test_success_withdraw_max() {
    let (mut test, lending_market, usdc_reserve, wsol_reserve, user, obligation) = setup().await;

    let balance_checker =
        BalanceChecker::start(&mut test, &[&usdc_reserve, &user, &wsol_reserve]).await;

    lending_market
        .withdraw_obligation_collateral(&mut test, &usdc_reserve, &obligation, &user, u64::MAX)
        .await
        .unwrap();

    // we are borrowing 6 SOL @ $10 with an ltv of 0.5, so the debt has to be collateralized by
    // exactly 120cUSDC.
    let balance_changes = balance_checker.find_balance_changes(&mut test).await;
    let expected_balance_changes = HashSet::from([
        BalanceChange {
            token_account: user
                .get_account(&usdc_reserve.account.collateral.mint_pubkey)
                .unwrap(),
            mint: usdc_reserve.account.collateral.mint_pubkey,
            diff: 100_000_000_000 - 120_000_000,
        },
        BalanceChange {
            token_account: usdc_reserve.account.collateral.supply_pubkey,
            mint: usdc_reserve.account.collateral.mint_pubkey,
            diff: -(100_000_000_000 - 120_000_000),
        },
    ]);
    assert_eq!(balance_changes, expected_balance_changes);

    let usdc_reserve_post = test.load_account::<Reserve>(usdc_reserve.pubkey).await;
    assert_eq!(usdc_reserve_post.account, usdc_reserve.account);

    let obligation_post = test.load_account::<Obligation>(obligation.pubkey).await;
    assert_eq!(
        obligation_post.account,
        Obligation {
            last_update: LastUpdate {
                slot: 1000,
                stale: true
            },
            deposits: [ObligationCollateral {
                deposit_reserve: usdc_reserve.pubkey,
                deposited_amount: 120_000_000,
                ..obligation.account.deposits[0]
            }]
            .to_vec(),
            ..obligation.account
        }
    );
}

#[tokio::test]
async fn test_fail_withdraw_too_much() {
    let (mut test, lending_market, usdc_reserve, _wsol_reserve, user, obligation) = setup().await;

    let res = lending_market
        .withdraw_obligation_collateral(
            &mut test,
            &usdc_reserve,
            &obligation,
            &user,
            100_000_000_000 - 120_000_000 + 1,
        )
        .await
        .err()
        .unwrap()
        .unwrap();

    assert_eq!(
        res,
        TransactionError::InstructionError(
            3,
            InstructionError::Custom(LendingError::WithdrawTooLarge as u32)
        )
    );
}


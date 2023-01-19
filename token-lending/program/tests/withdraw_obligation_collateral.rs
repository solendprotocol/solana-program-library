#![cfg(feature = "test-bpf")]

mod helpers;

use crate::solend_program_test::scenario_1;
use helpers::solend_program_test::{BalanceChange, BalanceChecker};
use helpers::*;

use solana_program_test::*;
use solana_sdk::{instruction::InstructionError, signature::Signer, transaction::TransactionError};
use solend_program::error::LendingError;
use solend_program::state::{LastUpdate, Obligation, ObligationCollateral, Reserve};
use std::collections::HashSet;
use std::u64;

#[tokio::test]
async fn test_success_withdraw_fixed_amount() {
    let (mut test, lending_market, usdc_reserve, wsol_reserve, user, obligation) =
        scenario_1().await;

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
    let (mut test, lending_market, usdc_reserve, wsol_reserve, user, obligation) =
        scenario_1().await;

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
    let (mut test, lending_market, usdc_reserve, _wsol_reserve, user, obligation) =
        scenario_1().await;

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

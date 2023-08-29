#![cfg(feature = "test-bpf")]

use crate::solend_program_test::custom_scenario;
use crate::solend_program_test::find_reserve;
use crate::solend_program_test::User;

use crate::solend_program_test::BalanceChecker;
use crate::solend_program_test::ObligationArgs;
use crate::solend_program_test::PriceArgs;
use crate::solend_program_test::ReserveArgs;
use crate::solend_program_test::TokenBalanceChange;
use solana_program::native_token::LAMPORTS_PER_SOL;
use solana_sdk::instruction::InstructionError;
use solana_sdk::signer::Signer;
use solana_sdk::transaction::TransactionError;
use solend_program::error::LendingError;

use solend_program::state::ReserveConfig;

use solend_sdk::state::ReserveFees;
mod helpers;

use helpers::*;
use solana_program_test::*;
use wrapper::processor::liquidate_without_receiving_ctokens;

use std::collections::HashSet;

#[tokio::test]
async fn test_liquidate() {
    let (mut test, lending_market, reserves, obligations, users, lending_market_owner) =
        custom_scenario(
            &[
                ReserveArgs {
                    mint: usdc_mint::id(),
                    config: test_reserve_config(),
                    liquidity_amount: 10 * FRACTIONAL_TO_USDC,
                    price: PriceArgs {
                        price: 10,
                        conf: 0,
                        expo: -1,
                        ema_price: 10,
                        ema_conf: 0,
                    },
                },
                ReserveArgs {
                    mint: wsol_mint::id(),
                    config: test_reserve_config(),
                    liquidity_amount: 100 * LAMPORTS_PER_SOL,
                    price: PriceArgs {
                        price: 10,
                        conf: 0,
                        expo: 0,
                        ema_price: 10,
                        ema_conf: 0,
                    },
                },
            ],
            &[
                ObligationArgs {
                    deposits: vec![(usdc_mint::id(), 100 * FRACTIONAL_TO_USDC)],
                    borrows: vec![(wsol_mint::id(), LAMPORTS_PER_SOL)],
                },
                ObligationArgs {
                    deposits: vec![(wsol_mint::id(), 100_000 * LAMPORTS_PER_SOL)],
                    borrows: vec![],
                },
            ],
        )
        .await;

    test.advance_clock_by_slots(1).await;

    let repay_reserve = find_reserve(&reserves, &wsol_mint::id()).unwrap();
    let withdraw_reserve = find_reserve(&reserves, &usdc_mint::id()).unwrap();

    lending_market
        .update_reserve_config(
            &mut test,
            &lending_market_owner,
            &repay_reserve,
            ReserveConfig {
                added_borrow_weight_bps: u64::MAX,
                ..repay_reserve.account.config
            },
            repay_reserve.account.rate_limiter.config,
            None,
        )
        .await
        .unwrap();

    test.advance_clock_by_slots(1).await;

    lending_market
        .borrow_obligation_liquidity(
            &mut test,
            &withdraw_reserve,
            &obligations[1],
            &users[1],
            None,
            u64::MAX,
        )
        .await
        .unwrap();

    test.advance_clock_by_slots(1).await;

    let liquidator = User::new_with_balances(
        &mut test,
        &[
            (&wsol_mint::id(), 100 * LAMPORTS_TO_SOL),
            (&withdraw_reserve.account.collateral.mint_pubkey, 0),
            (&usdc_mint::id(), 0),
        ],
    )
    .await;

    let balance_checker = BalanceChecker::start(&mut test, &[&liquidator]).await;

    let mut instructions = lending_market
        .build_refresh_instructions(&mut test, &obligations[0], None)
        .await;

    instructions.push(liquidate_without_receiving_ctokens(
        wrapper::id(),
        u64::MAX,
        solend_program::id(),
        liquidator
            .get_account(&repay_reserve.account.liquidity.mint_pubkey)
            .unwrap(),
        liquidator
            .get_account(&withdraw_reserve.account.collateral.mint_pubkey)
            .unwrap(),
        liquidator
            .get_account(&withdraw_reserve.account.liquidity.mint_pubkey)
            .unwrap(),
        repay_reserve.pubkey,
        repay_reserve.account.liquidity.supply_pubkey,
        withdraw_reserve.pubkey,
        withdraw_reserve.account.collateral.mint_pubkey,
        withdraw_reserve.account.collateral.supply_pubkey,
        withdraw_reserve.account.liquidity.supply_pubkey,
        withdraw_reserve.account.config.fee_receiver,
        obligations[0].pubkey,
        obligations[0].account.lending_market,
        liquidator.keypair.pubkey(),
    ));

    test.process_transaction(&instructions, Some(&[&liquidator.keypair]))
        .await
        .unwrap();

    let balances = balance_checker.find_balance_changes(&mut test).await;
    println!("balances changes: {:#?}", balances);
}

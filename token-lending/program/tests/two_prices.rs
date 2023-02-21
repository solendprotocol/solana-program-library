#![cfg(feature = "test-bpf")]

use crate::solend_program_test::custom_scenario;
use crate::solend_program_test::setup_world;
use crate::solend_program_test::BalanceChecker;
use crate::solend_program_test::ObligationArgs;
use crate::solend_program_test::PriceArgs;
use crate::solend_program_test::ReserveArgs;
use crate::solend_program_test::TokenBalanceChange;
use solana_program::native_token::LAMPORTS_PER_SOL;
use solana_sdk::instruction::InstructionError;
use solana_sdk::transaction::TransactionError;
use solend_program::error::LendingError;
use solend_program::state::ReserveConfig;
use solend_program::NULL_PUBKEY;
use solend_sdk::state::ReserveFees;
mod helpers;

use crate::solend_program_test::scenario_1;
use crate::solend_program_test::User;
use helpers::*;
use solana_program_test::*;
use solana_sdk::signature::Keypair;
use solend_program::math::Decimal;
use solend_program::state::Obligation;
use std::collections::HashSet;

/// the two prices feature affects a bunch of instructions. All of those instructions are tested
/// here for correctness.

#[tokio::test]
async fn test_borrow() {
    let (mut test, lending_market, reserves, obligation, user) = custom_scenario(
        &[
            ReserveArgs {
                mint: usdc_mint::id(),
                config: test_reserve_config(),
                liquidity_amount: 100_000 * FRACTIONAL_TO_USDC,
                price: PriceArgs {
                    price: 9,
                    conf: 0,
                    expo: -1,
                    ema_price: 10,
                    ema_conf: 1,
                },
            },
            ReserveArgs {
                mint: wsol_mint::id(),
                config: ReserveConfig {
                    loan_to_value_ratio: 50,
                    liquidation_threshold: 55,
                    fees: ReserveFees::default(),
                    ..test_reserve_config()
                },
                liquidity_amount: 100 * LAMPORTS_PER_SOL,
                price: PriceArgs {
                    price: 9,
                    conf: 0,
                    expo: 0,
                    ema_price: 10,
                    ema_conf: 0,
                },
            },
        ],
        &ObligationArgs {
            deposits: vec![],
            borrows: vec![],
        },
    )
    .await;
    // let (mut test, lending_market, _, wsol_reserve, user, obligation) = scenario_1(
    //     &test_reserve_config(),
    //     &ReserveConfig {
    //         loan_to_value_ratio: 50,
    //         liquidation_threshold: 55,
    //         fees: ReserveFees::default(),
    //         ..test_reserve_config()
    //     },
    // )
    // .await;

    test.set_price(
        &usdc_mint::id(),
        &PriceArgs {
            price: 9,
            conf: 0,
            expo: -1,
            ema_price: 10,
            ema_conf: 1,
        },
    )
    .await;

    // test.set_price(
    //     &wsol_mint::id(),
    //     &PriceArgs {
    //         price: 10,
    //         conf: 0,
    //         expo: 0,
    //         ema_price: 12,
    //         ema_conf: 1,
    //     },
    // )
    // .await;

    test.advance_clock_by_slots(1).await;

    // lending_market
    //     .refresh_obligation(&mut test, &obligation)
    //     .await
    //     .unwrap();
    // let obligation = test.load_account(obligation.pubkey).await;
    // println!("obligation before: {:#?}", obligation);

    // let balance_checker = BalanceChecker::start(&mut test, &[&user]).await;

    // // obligation currently has 100k USDC deposited, 10 SOL borrowed.
    // // if we try to borrow the max amount, how much SOL should we receive?
    // // allowed borrow value = 100k * 0.9 * 0.5 = $45k
    // // borrow value upper bound = 10 * 20 = $200
    // // max SOL that can be borrowed is: ($45k - $200) / 20 = 2240 SOL
    // lending_market
    //     .borrow_obligation_liquidity(
    //         &mut test,
    //         &wsol_reserve,
    //         &obligation,
    //         &user,
    //         &NULL_PUBKEY,
    //         u64::MAX,
    //     )
    //     .await
    //     .unwrap();

    // let (balance_changes, _) = balance_checker.find_balance_changes(&mut test).await;
    // println!("{:#?}", balance_changes);
}

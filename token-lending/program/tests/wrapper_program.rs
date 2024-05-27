#![cfg(feature = "test-bpf")]

use crate::solend_program_test::TokenBalanceChange;
use solana_sdk::instruction::InstructionError;
use solana_sdk::signer::keypair::Keypair;
use solana_sdk::transaction::TransactionError;
use solend_sdk::state::{ReserveFees, ReserveType};
use std::collections::HashSet;
use wrapper::processor::max_deposit;
use wrapper::processor::max_repay;
use wrapper::processor::withdraw_exact;

use crate::solend_program_test::custom_scenario;
use crate::solend_program_test::find_reserve;
use crate::solend_program_test::User;

use crate::solend_program_test::BalanceChecker;
use crate::solend_program_test::ObligationArgs;
use crate::solend_program_test::PriceArgs;
use crate::solend_program_test::ReserveArgs;

use solana_program::native_token::LAMPORTS_PER_SOL;

use solana_sdk::signer::Signer;

use solend_program::state::ReserveConfig;

mod helpers;

use helpers::*;
use solana_program_test::*;
use wrapper::processor::liquidate_without_receiving_ctokens;

#[tokio::test]
async fn test_liquidate() {
    let (mut test, lending_market, reserves, obligations, _users, lending_market_owner) =
        custom_scenario(
            &[
                ReserveArgs {
                    mint: usdc_mint::id(),
                    config: reserve_config_no_fees(),
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
                    config: reserve_config_no_fees(),
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

    let (balance_changes, _) = balance_checker.find_balance_changes(&mut test).await;
    let expected_balances_changes = HashSet::from([
        TokenBalanceChange {
            token_account: liquidator.get_account(&usdc_mint::id()).unwrap(),
            mint: usdc_mint::id(),
            diff: (10 * FRACTIONAL_TO_USDC - 1) as i128,
        },
        TokenBalanceChange {
            token_account: liquidator.get_account(&wsol_mint::id()).unwrap(),
            mint: wsol_mint::id(),
            diff: -(LAMPORTS_PER_SOL as i128),
        },
    ]);
    assert_eq!(balance_changes, expected_balances_changes);
}

#[tokio::test]
async fn test_liquidate_fail() {
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

    let err = test
        .process_transaction(&instructions, Some(&[&liquidator.keypair]))
        .await
        .err()
        .unwrap()
        .unwrap();

    assert_eq!(
        err,
        TransactionError::InstructionError(3, InstructionError::Custom(0))
    );
    let (balance_changes, _) = balance_checker.find_balance_changes(&mut test).await;
    assert!(balance_changes.is_empty());
}

#[tokio::test]
async fn test_repay() {
    let (mut test, lending_market, reserves, obligations, users, _lending_market_owner) =
        custom_scenario(
            &[
                ReserveArgs {
                    mint: usdc_mint::id(),
                    config: reserve_config_no_fees(),
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
                    config: reserve_config_no_fees(),
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
            &[ObligationArgs {
                deposits: vec![(usdc_mint::id(), 100 * FRACTIONAL_TO_USDC)],
                borrows: vec![(wsol_mint::id(), LAMPORTS_PER_SOL)],
            }],
        )
        .await;

    let instruction = max_repay(
        wrapper::id(),
        solend_program::id(),
        users[0].get_account(&wsol_mint::id()).unwrap(),
        reserves[1].account.liquidity.supply_pubkey,
        reserves[1].pubkey,
        obligations[0].pubkey,
        lending_market.pubkey,
        users[0].keypair.pubkey(),
    );

    test.process_transaction(&[instruction], Some(&[&users[0].keypair]))
        .await
        .unwrap();
}

#[tokio::test]
async fn test_deposit() {
    let (mut test, lending_market, reserves, obligations, users, _lending_market_owner) =
        custom_scenario(
            &[ReserveArgs {
                mint: usdc_mint::id(),
                config: reserve_config_no_fees(),
                liquidity_amount: 10 * FRACTIONAL_TO_USDC,
                price: PriceArgs {
                    price: 10,
                    conf: 0,
                    expo: -1,
                    ema_price: 10,
                    ema_conf: 0,
                },
            }],
            &[ObligationArgs {
                deposits: vec![(usdc_mint::id(), 100 * FRACTIONAL_TO_USDC)],
                borrows: vec![],
            }],
        )
        .await;

    test.advance_clock_by_slots(1).await;

    let new_user =
        User::new_with_balances(&mut test, &[(&usdc_mint::id(), 100 * FRACTIONAL_TO_USDC)]).await;

    new_user
        .transfer(
            &usdc_mint::id(),
            users[0].get_account(&usdc_mint::id()).unwrap(),
            100 * FRACTIONAL_TO_USDC,
            &mut test,
        )
        .await;

    test.advance_clock_by_slots(1).await;

    let instruction = max_deposit(
        wrapper::id(),
        solend_program::id(),
        users[0].get_account(&usdc_mint::id()).unwrap(),
        users[0]
            .get_account(&reserves[0].account.collateral.mint_pubkey)
            .unwrap(),
        reserves[0].pubkey,
        reserves[0].account.liquidity.supply_pubkey,
        reserves[0].account.collateral.mint_pubkey,
        lending_market.pubkey,
        reserves[0].account.collateral.supply_pubkey,
        obligations[0].pubkey,
        obligations[0].account.owner,
        reserves[0].account.liquidity.pyth_oracle_pubkey,
        reserves[0].account.liquidity.switchboard_oracle_pubkey,
        obligations[0].account.owner,
    );
    println!("hello");

    test.process_transaction(&[instruction], Some(&[&users[0].keypair]))
        .await
        .unwrap();
}

#[tokio::test]
async fn test_withdraw_exact() {
    let (mut test, lending_market, reserves, obligations, users, _lending_market_owner) =
        custom_scenario(
            &[
                ReserveArgs {
                    mint: usdt_mint::id(),
                    config: reserve_config_no_fees(),
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
                    mint: usdc_mint::id(),
                    config: reserve_config_no_fees(),
                    liquidity_amount: 10 * FRACTIONAL_TO_USDC,
                    price: PriceArgs {
                        price: 10,
                        conf: 0,
                        expo: -1,
                        ema_price: 10,
                        ema_conf: 0,
                    },
                },
            ],
            &[ObligationArgs {
                deposits: vec![
                    (usdc_mint::id(), 100 * FRACTIONAL_TO_USDC),
                    (usdt_mint::id(), 1 * FRACTIONAL_TO_USDC),
                ],
                borrows: vec![(usdt_mint::id(), 1 * FRACTIONAL_TO_USDC)],
            }],
        )
        .await;

    test.advance_clock_by_slots(1).await;

    let balance_checker = BalanceChecker::start(&mut test, &[&users[0]]).await;

    let mut instructions = lending_market
        .build_refresh_instructions(&mut test, &obligations[0], None)
        .await;

    instructions.push(withdraw_exact(
        wrapper::id(),
        solend_program::id(),
        reserves[0].account.collateral.supply_pubkey,
        // user_collateral_pubkey,
        users[0]
            .get_account(&reserves[0].account.collateral.mint_pubkey)
            .unwrap(),
        // reserve_pubkey,
        reserves[0].pubkey,
        // obligation_pubkey,
        obligations[0].pubkey,
        // lending_market_pubkey,
        lending_market.pubkey,
        // user_liquidity_pubkey,
        users[0]
            .get_account(&reserves[0].account.liquidity.mint_pubkey)
            .unwrap(),
        // reserve_collateral_mint_pubkey,
        reserves[0].account.collateral.mint_pubkey,
        // reserve_liquidity_supply_pubkey,
        reserves[0].account.liquidity.supply_pubkey,
        // obligation_owner_pubkey,
        obligations[0].account.owner,
        // user_transfer_authority_pubkey,
        users[0].keypair.pubkey(),
        obligations[0]
            .account
            .deposits
            .iter()
            .map(|d| d.deposit_reserve)
            .collect(),
        // liquidity amount
        4 * FRACTIONAL_TO_USDC,
    ));

    test.process_transaction(&instructions, Some(&[&users[0].keypair]))
        .await
        .unwrap();

    let (balance_changes, _) = balance_checker.find_balance_changes(&mut test).await;
    println!("{:?}", balance_changes);
}

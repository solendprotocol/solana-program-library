#![cfg(feature = "test-bpf")]

use crate::solend_program_test::custom_scenario;
use crate::solend_program_test::find_reserve;
use crate::solend_program_test::BalanceChecker;
use crate::solend_program_test::TokenBalanceChange;
use crate::solend_program_test::User;
use std::collections::HashSet;

use solend_sdk::math::Decimal;

use crate::solend_program_test::ObligationArgs;
use crate::solend_program_test::PriceArgs;
use crate::solend_program_test::ReserveArgs;

use solana_program::native_token::LAMPORTS_PER_SOL;
use solana_sdk::instruction::InstructionError;
use solana_sdk::transaction::TransactionError;
use solend_program::error::LendingError;

use solend_program::state::ReserveConfig;

use solend_sdk::state::*;
mod helpers;

use helpers::*;
use solana_program_test::*;

#[tokio::test]
async fn test_forgive_debt_success_easy() {
    let (mut test, lending_market, reserves, obligations, users, lending_market_owner) =
        custom_scenario(
            &[
                ReserveArgs {
                    mint: usdc_mint::id(),
                    config: ReserveConfig {
                        liquidation_bonus: 0,
                        ..test_reserve_config()
                    },
                    liquidity_amount: 100_000 * FRACTIONAL_TO_USDC,
                    price: PriceArgs {
                        price: 10,
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
                        optimal_borrow_rate: 0,
                        max_borrow_rate: 0,
                        ..test_reserve_config()
                    },
                    liquidity_amount: LAMPORTS_PER_SOL,
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
                    deposits: vec![(usdc_mint::id(), 20 * FRACTIONAL_TO_USDC)],
                    borrows: vec![(wsol_mint::id(), LAMPORTS_PER_SOL)],
                },
                ObligationArgs {
                    deposits: vec![(wsol_mint::id(), LAMPORTS_PER_SOL)],
                    borrows: vec![],
                },
            ],
        )
        .await;

    // USDC depegs to 0.1
    test.set_price(
        &usdc_mint::id(),
        &PriceArgs {
            price: 1,
            conf: 0,
            expo: -1,
            ema_price: 0,
            ema_conf: 0,
        },
    )
    .await;

    test.advance_clock_by_slots(1).await;

    let wsol_reserve = find_reserve(&reserves, &wsol_mint::id()).unwrap();
    let usdc_reserve = find_reserve(&reserves, &usdc_mint::id()).unwrap();

    // this should fail because the obligation hasn't been liquidated yet
    let err = lending_market
        .forgive_debt(
            &mut test,
            &obligations[0],
            &lending_market_owner,
            &wsol_reserve,
            u64::MAX,
        )
        .await
        .unwrap_err()
        .unwrap();

    assert_eq!(
        err,
        TransactionError::InstructionError(
            3,
            InstructionError::Custom(LendingError::InvalidAccountInput as u32)
        )
    );

    // liquidate everything first. 0.2 SOL is repaid, 2 USDC is withdrawn
    {
        let liquidator = User::new_with_balances(
            &mut test,
            &[
                (&usdc_mint::id(), 100_000 * FRACTIONAL_TO_USDC),
                (&usdc_reserve.account.collateral.mint_pubkey, 0),
                (&wsol_mint::id(), 100_000 * LAMPORTS_PER_SOL),
                (&wsol_reserve.account.collateral.mint_pubkey, 0),
            ],
        )
        .await;

        lending_market
            .liquidate_obligation_and_redeem_reserve_collateral(
                &mut test,
                &wsol_reserve,
                &usdc_reserve,
                &obligations[0],
                &liquidator,
                u64::MAX,
            )
            .await
            .unwrap();

        test.advance_clock_by_slots(1).await;
    }

    lending_market
        .forgive_debt(
            &mut test,
            &obligations[0],
            &lending_market_owner,
            &wsol_reserve,
            u64::MAX,
        )
        .await
        .unwrap();

    let obligation_post = test.load_account::<Obligation>(obligations[0].pubkey).await;
    assert_eq!(
        obligation_post.account,
        Obligation {
            last_update: LastUpdate {
                slot: 1002,
                stale: true,
            },
            deposits: vec![],
            borrows: vec![],
            deposited_value: Decimal::zero(),
            borrowed_value: Decimal::from(8u64),
            borrowed_value_upper_bound: Decimal::from(8u64),
            allowed_borrow_value: Decimal::zero(),
            unhealthy_borrow_value: Decimal::zero(),
            ..obligations[0].account
        }
    );

    let wsol_reserve_post = test.load_account::<Reserve>(wsol_reserve.pubkey).await;
    assert_eq!(
        wsol_reserve_post.account,
        Reserve {
            last_update: LastUpdate {
                slot: 1002,
                stale: true,
            },
            liquidity: ReserveLiquidity {
                borrowed_amount_wads: Decimal::zero(),
                // 0.2 SOL is repaid on liquidation
                available_amount: LAMPORTS_PER_SOL / 5
                    + wsol_reserve.account.liquidity.available_amount,
                ..wsol_reserve.account.liquidity
            },
            ..wsol_reserve.account.clone()
        }
    );

    test.advance_clock_by_slots(1).await;

    // user 2 tries to withdraw their SOL with a 40% haircut (0.8 sol is forgiven out of 2 sol)
    let balance_checker = BalanceChecker::start(&mut test, &[&users[1]]).await;

    lending_market
        .withdraw_obligation_collateral_and_redeem_reserve_collateral(
            &mut test,
            &wsol_reserve,
            &obligations[1],
            &users[1],
            u64::MAX,
        )
        .await
        .unwrap();

    let (balance_changes, _) = balance_checker.find_balance_changes(&mut test).await;
    assert_eq!(
        balance_changes,
        HashSet::from([TokenBalanceChange {
            token_account: users[1].get_account(&wsol_mint::id()).unwrap(),
            mint: wsol_mint::id(),
            diff: (LAMPORTS_PER_SOL * 6 / 10) as i128
        }])
    );
}

#[tokio::test]
async fn test_forgive_debt_fail_invalid_signer() {
    let (mut test, lending_market, reserves, obligations, users, _lending_market_owner) =
        custom_scenario(
            &[
                ReserveArgs {
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
                },
                ReserveArgs {
                    mint: wsol_mint::id(),
                    config: ReserveConfig {
                        loan_to_value_ratio: 50,
                        liquidation_threshold: 55,
                        fees: ReserveFees::default(),
                        optimal_borrow_rate: 0,
                        max_borrow_rate: 0,
                        ..test_reserve_config()
                    },
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
                deposits: vec![(usdc_mint::id(), 200 * FRACTIONAL_TO_USDC)],
                borrows: vec![(wsol_mint::id(), 10 * LAMPORTS_PER_SOL)],
            }],
        )
        .await;

    // USDC depegs to 25c
    test.set_price(
        &usdc_mint::id(),
        &PriceArgs {
            price: 25,
            conf: 0,
            expo: -2,
            ema_price: 25,
            ema_conf: 0,
        },
    )
    .await;

    test.advance_clock_by_slots(1).await;

    let wsol_reserve = find_reserve(&reserves, &wsol_mint::id()).unwrap();
    let err = lending_market
        .forgive_debt(
            &mut test,
            &obligations[0],
            &users[0], // <-- wrong signer
            &wsol_reserve,
            u64::MAX,
        )
        .await
        .unwrap_err()
        .unwrap();

    assert_eq!(
        err,
        TransactionError::InstructionError(
            3,
            InstructionError::Custom(LendingError::InvalidMarketOwner as u32)
        )
    );
}

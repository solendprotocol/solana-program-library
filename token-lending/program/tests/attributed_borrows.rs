#![cfg(feature = "test-bpf")]

use crate::solend_program_test::custom_scenario;

use solana_sdk::instruction::InstructionError;
use solana_sdk::transaction::TransactionError;
use solend_program::math::TryAdd;
use solend_program::state::LastUpdate;
use solend_program::state::Reserve;
use solend_sdk::error::LendingError;
use solend_sdk::state::ReserveLiquidity;

use crate::solend_program_test::ObligationArgs;
use crate::solend_program_test::PriceArgs;
use crate::solend_program_test::ReserveArgs;

use solana_program::native_token::LAMPORTS_PER_SOL;

use solend_sdk::math::Decimal;

use solend_program::state::{Obligation, ReserveConfig};

use solend_sdk::state::ReserveFees;
mod helpers;

use helpers::*;
use solana_program_test::*;

#[tokio::test]
async fn test_calculations() {
    let (mut test, lending_market, reserves, obligations, users, lending_market_owner) =
        custom_scenario(
            &[
                ReserveArgs {
                    mint: usdc_mint::id(),
                    config: ReserveConfig {
                        loan_to_value_ratio: 80,
                        liquidation_threshold: 81,
                        max_liquidation_threshold: 82,
                        fees: ReserveFees {
                            host_fee_percentage: 0,
                            ..ReserveFees::default()
                        },
                        optimal_borrow_rate: 0,
                        max_borrow_rate: 0,
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
                        loan_to_value_ratio: 80,
                        liquidation_threshold: 81,
                        max_liquidation_threshold: 82,
                        fees: ReserveFees {
                            host_fee_percentage: 0,
                            ..ReserveFees::default()
                        },
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
            &[
                ObligationArgs {
                    deposits: vec![
                        (usdc_mint::id(), 80 * FRACTIONAL_TO_USDC),
                        (wsol_mint::id(), 2 * LAMPORTS_PER_SOL),
                    ],
                    borrows: vec![
                        (usdc_mint::id(), 10 * FRACTIONAL_TO_USDC),
                        (wsol_mint::id(), LAMPORTS_PER_SOL),
                    ],
                },
                ObligationArgs {
                    deposits: vec![
                        (usdc_mint::id(), 400 * FRACTIONAL_TO_USDC),
                        (wsol_mint::id(), 10 * LAMPORTS_PER_SOL),
                    ],
                    borrows: vec![
                        (usdc_mint::id(), 100 * FRACTIONAL_TO_USDC),
                        (wsol_mint::id(), 2 * LAMPORTS_PER_SOL),
                    ],
                },
            ],
        )
        .await;

    // check initial borrow attribution values
    // obligation 0
    // usdc.borrow_attribution = 80 / 100 * 20 = 16
    assert_eq!(
        obligations[0].account.deposits[0].attributed_borrow_value,
        Decimal::from(16u64)
    );
    // wsol.borrow_attribution = 20 / 100 * 20 = 4
    assert_eq!(
        obligations[0].account.deposits[1].attributed_borrow_value,
        Decimal::from(4u64)
    );

    // obligation 1
    // usdc.borrow_attribution = 400 / 500 * 120 = 96
    assert_eq!(
        obligations[1].account.deposits[0].attributed_borrow_value,
        Decimal::from(96u64)
    );
    // wsol.borrow_attribution = 100 / 500 * 120 = 24
    assert_eq!(
        obligations[1].account.deposits[1].attributed_borrow_value,
        Decimal::from(24u64)
    );

    // usdc reserve: 16 + 96 = 112
    assert_eq!(
        reserves[0].account.attributed_borrow_value,
        Decimal::from(112u64)
    );
    // wsol reserve: 4 + 24 = 28
    assert_eq!(
        reserves[1].account.attributed_borrow_value,
        Decimal::from(28u64)
    );

    // change borrow attribution limit, check that it's applied
    lending_market
        .update_reserve_config(
            &mut test,
            &lending_market_owner,
            &reserves[0],
            ReserveConfig {
                attributed_borrow_limit: 113,
                ..reserves[0].account.config
            },
            reserves[0].account.rate_limiter.config,
            None,
        )
        .await
        .unwrap();

    // attempt to borrow another 10 usd from obligation 0, this should fail
    let err = lending_market
        .borrow_obligation_liquidity(
            &mut test,
            &reserves[0],
            &obligations[0],
            &users[0],
            None,
            10 * FRACTIONAL_TO_USDC,
        )
        .await
        .unwrap_err()
        .unwrap();

    assert_eq!(
        err,
        TransactionError::InstructionError(
            1,
            InstructionError::Custom(LendingError::BorrowAttributionLimitExceeded as u32)
        )
    );

    // change borrow attribution limit so that the borrow will succeed
    lending_market
        .update_reserve_config(
            &mut test,
            &lending_market_owner,
            &reserves[0],
            ReserveConfig {
                attributed_borrow_limit: 120,
                ..reserves[0].account.config
            },
            reserves[0].account.rate_limiter.config,
            None,
        )
        .await
        .unwrap();

    test.advance_clock_by_slots(1).await;

    // attempt to borrow another 10 usd from obligation 0, this should pass now
    lending_market
        .borrow_obligation_liquidity(
            &mut test,
            &reserves[0],
            &obligations[0],
            &users[0],
            None,
            10 * FRACTIONAL_TO_USDC,
        )
        .await
        .unwrap();

    // check both reserves before refresh, since the borrow attribution values should have been
    // updated
    {
        let usdc_reserve = reserves[0].account.clone();
        let usdc_reserve_post = test.load_account::<Reserve>(reserves[0].pubkey).await;
        let expected_usdc_reserve_post = Reserve {
            last_update: LastUpdate {
                slot: 1001,
                stale: true,
            },
            liquidity: ReserveLiquidity {
                available_amount: usdc_reserve.liquidity.available_amount - 10 * FRACTIONAL_TO_USDC,
                borrowed_amount_wads: usdc_reserve
                    .liquidity
                    .borrowed_amount_wads
                    .try_add(Decimal::from(10 * FRACTIONAL_TO_USDC))
                    .unwrap(),
                ..usdc_reserve.liquidity
            },
            rate_limiter: {
                let mut rate_limiter = usdc_reserve.rate_limiter;
                rate_limiter
                    .update(1000, Decimal::from(10 * FRACTIONAL_TO_USDC))
                    .unwrap();

                rate_limiter
            },
            attributed_borrow_value: Decimal::from(120u64),
            config: ReserveConfig {
                attributed_borrow_limit: 120,
                ..usdc_reserve.config
            },
            ..usdc_reserve
        };
        assert_eq!(usdc_reserve_post.account, expected_usdc_reserve_post);

        let wsol_reserve_post = test.load_account::<Reserve>(reserves[1].pubkey).await;
        assert_eq!(
            wsol_reserve_post.account.attributed_borrow_value,
            Decimal::from(30u64)
        );
    }

    lending_market
        .refresh_obligation(&mut test, &obligations[0])
        .await
        .unwrap();

    let obligation_post = test.load_account::<Obligation>(obligations[0].pubkey).await;

    // obligation 0 after borrowing 10 usd
    // usdc.borrow_attribution = 80 / 100 * 30 = 24
    assert_eq!(
        obligation_post.account.deposits[0].attributed_borrow_value,
        Decimal::from(24u64)
    );

    // wsol.borrow_attribution = 20 / 100 * 30 = 6
    assert_eq!(
        obligation_post.account.deposits[1].attributed_borrow_value,
        Decimal::from(6u64)
    );

    let usdc_reserve_post = test.load_account::<Reserve>(reserves[0].pubkey).await;
    assert_eq!(
        usdc_reserve_post.account.attributed_borrow_value,
        Decimal::from(120u64)
    );

    let wsol_reserve_post = test.load_account::<Reserve>(reserves[1].pubkey).await;
    assert_eq!(
        wsol_reserve_post.account.attributed_borrow_value,
        Decimal::from(30u64)
    );
}

// #[tokio::test]
// async fn benchmark() {
//     // setup
//     let reserve_arg = ReserveArgs {
//         mint: usdc_mint::id(),
//         config: ReserveConfig {
//             loan_to_value_ratio: 80,
//             liquidation_threshold: 81,
//             max_liquidation_threshold: 82,
//             fees: ReserveFees {
//                 host_fee_percentage: 0,
//                 ..ReserveFees::default()
//             },
//             optimal_borrow_rate: 0,
//             max_borrow_rate: 0,
//             ..test_reserve_config()
//         },
//         liquidity_amount: 100 * FRACTIONAL_TO_USDC,
//         price: PriceArgs {
//             price: 10,
//             conf: 0,
//             expo: -1,
//             ema_price: 10,
//             ema_conf: 1,
//         },
//     };

//     let reserve_args = vec![reserve_arg; 9];

//     let obligation_args = ObligationArgs {
//         deposits: vec![],
//         borrows: vec![],
//     };

//     let (mut test, lending_market, reserves, obligations, mut users, _lending_market_owner) =
//         custom_scenario(&reserve_args, &[obligation_args]).await;

//     let user = User::new_with_balances(
//         &mut test,
//         &[(&usdc_mint::id(), 100_000 * FRACTIONAL_TO_USDC)],
//     )
//     .await;

//     user.transfer(
//         &usdc_mint::id(),
//         users[0].get_account(&usdc_mint::id()).unwrap(),
//         100_000 * FRACTIONAL_TO_USDC,
//         &mut test,
//     )
//     .await;

//     test.advance_clock_by_slots(1).await;

//     for reserve in &reserves {
//         users[0]
//             .create_token_account(&reserve.account.collateral.mint_pubkey, &mut test)
//             .await;

//         lending_market
//             .deposit_reserve_liquidity_and_obligation_collateral(
//                 &mut test,
//                 reserve,
//                 &obligations[0],
//                 &users[0],
//                 10 * FRACTIONAL_TO_USDC,
//             )
//             .await
//             .unwrap();

//         test.advance_clock_by_slots(1).await;
//     }

//     lending_market
//         .borrow_obligation_liquidity(
//             &mut test,
//             &reserves[0],
//             &obligations[0],
//             &users[0],
//             None,
//             FRACTIONAL_TO_USDC,
//         )
//         .await
//         .unwrap();

//     info!("Starting benchmark");
//     // lending_market
//     //     .refresh_obligation(&mut test, &obligations[0])
//     //     .await
//     //     .unwrap();

//     // test.advance_clock_by_slots(1).await;

//     for reserve in reserves.iter().skip(1).rev() {
//         lending_market
//             .withdraw_obligation_collateral_and_redeem_reserve_collateral(
//                 &mut test,
//                 reserve,
//                 &obligations[0],
//                 &users[0],
//                 u64::MAX,
//             )
//             .await
//             .unwrap();

//         test.advance_clock_by_slots(1).await;
//     }

//     lending_market
//         .refresh_obligation(&mut test, &obligations[0])
//         .await
//         .unwrap();

//     test.advance_clock_by_slots(1).await;
// }

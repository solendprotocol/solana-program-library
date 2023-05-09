#![cfg(feature = "test-bpf")]

use crate::solend_program_test::custom_scenario;

use crate::solend_program_test::ObligationArgs;
use crate::solend_program_test::PriceArgs;
use crate::solend_program_test::ReserveArgs;

use solana_program::native_token::LAMPORTS_PER_SOL;
use solana_sdk::instruction::InstructionError;
use solana_sdk::transaction::TransactionError;
use solend_program::error::LendingError;
use solend_sdk::math::Decimal;

use solend_program::state::LastUpdate;
use solend_program::state::ReserveType;
use solend_program::state::{Obligation, ObligationLiquidity, ReserveConfig};
use solend_program::NULL_PUBKEY;
use solend_sdk::state::ReserveFees;
mod helpers;

use helpers::*;
use solana_program_test::*;

#[tokio::test]
async fn test_refresh_obligation() {
    let (mut test, lending_market, reserves, obligations, users, _) = custom_scenario(
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
                    loan_to_value_ratio: 0,
                    liquidation_threshold: 0,
                    fees: ReserveFees {
                        host_fee_percentage: 0,
                        ..ReserveFees::default()
                    },
                    optimal_borrow_rate: 0,
                    max_borrow_rate: 0,
                    reserve_type: ReserveType::Isolated,

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
            deposits: vec![(usdc_mint::id(), 100 * FRACTIONAL_TO_USDC)],
            borrows: vec![],
        }],
    )
    .await;

    lending_market
        .refresh_obligation(&mut test, &obligations[0])
        .await
        .unwrap();

    let obligation = test.load_account::<Obligation>(obligations[0].pubkey).await;
    assert!(!obligation.account.borrowing_isolated_asset);

    test.advance_clock_by_slots(1).await;

    let wsol_reserve = reserves
        .iter()
        .find(|r| r.account.liquidity.mint_pubkey == wsol_mint::id())
        .unwrap();

    // borrow isolated tier asset
    lending_market
        .borrow_obligation_liquidity(
            &mut test,
            wsol_reserve,
            &obligations[0],
            &users[0],
            &NULL_PUBKEY,
            LAMPORTS_PER_SOL,
        )
        .await
        .unwrap();

    lending_market
        .refresh_obligation(&mut test, &obligations[0])
        .await
        .unwrap();

    let obligation_post = test.load_account::<Obligation>(obligations[0].pubkey).await;

    assert_eq!(
        obligation_post.account,
        Obligation {
            last_update: LastUpdate {
                slot: 1001,
                stale: false
            },
            borrows: vec![ObligationLiquidity {
                borrow_reserve: wsol_reserve.pubkey,
                cumulative_borrow_rate_wads: Decimal::one(),
                borrowed_amount_wads: Decimal::from(LAMPORTS_PER_SOL),
                market_value: Decimal::from(10u64),
            }],
            borrowed_value: Decimal::from(10u64),
            borrowed_value_upper_bound: Decimal::from(10u64),
            borrowing_isolated_asset: true,
            ..obligations[0].account.clone()
        }
    );
}

#[tokio::test]
async fn borrow_isolated_asset() {
    let (mut test, lending_market, reserves, obligations, users, _) = custom_scenario(
        &[
            ReserveArgs {
                mint: usdc_mint::id(),
                config: test_reserve_config(),
                liquidity_amount: 100_000 * FRACTIONAL_TO_USDC,
                price: PriceArgs {
                    price: 1,
                    conf: 0,
                    expo: 0,
                    ema_price: 1,
                    ema_conf: 0,
                },
            },
            ReserveArgs {
                mint: bonk_mint::id(),
                config: ReserveConfig {
                    loan_to_value_ratio: 0,
                    liquidation_threshold: 0,
                    fees: ReserveFees::default(),
                    optimal_borrow_rate: 0,
                    max_borrow_rate: 0,
                    protocol_liquidation_fee: 0,
                    reserve_type: ReserveType::Isolated,
                    ..test_reserve_config()
                },
                liquidity_amount: 1_000_000,
                price: PriceArgs {
                    price: 1,
                    conf: 0,
                    expo: -6,
                    ema_price: 1,
                    ema_conf: 0,
                },
            },
        ],
        &[ObligationArgs {
            deposits: vec![(usdc_mint::id(), 100 * FRACTIONAL_TO_USDC)],
            borrows: vec![],
        }],
    )
    .await;

    let bonk_reserve = reserves
        .iter()
        .find(|r| r.account.liquidity.mint_pubkey == bonk_mint::id())
        .unwrap();

    lending_market
        .borrow_obligation_liquidity(
            &mut test,
            bonk_reserve,
            &obligations[0],
            &users[0],
            &NULL_PUBKEY,
            10,
        )
        .await
        .unwrap();

    // borrow again
    lending_market
        .borrow_obligation_liquidity(
            &mut test,
            bonk_reserve,
            &obligations[0],
            &users[0],
            &NULL_PUBKEY,
            u64::MAX,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn borrow_isolated_asset_invalid() {
    let (mut test, lending_market, reserves, obligations, users, _) = custom_scenario(
        &[
            ReserveArgs {
                mint: usdc_mint::id(),
                config: test_reserve_config(),
                liquidity_amount: 100_000 * FRACTIONAL_TO_USDC,
                price: PriceArgs {
                    price: 1,
                    conf: 0,
                    expo: 0,
                    ema_price: 1,
                    ema_conf: 0,
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
                    protocol_liquidation_fee: 0,
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
            ReserveArgs {
                mint: bonk_mint::id(),
                config: ReserveConfig {
                    loan_to_value_ratio: 0,
                    liquidation_threshold: 0,
                    fees: ReserveFees::default(),
                    optimal_borrow_rate: 0,
                    max_borrow_rate: 0,
                    protocol_liquidation_fee: 0,
                    reserve_type: ReserveType::Isolated,
                    ..test_reserve_config()
                },
                liquidity_amount: 1_000_000,
                price: PriceArgs {
                    price: 1,
                    conf: 0,
                    expo: -6,
                    ema_price: 1,
                    ema_conf: 0,
                },
            },
        ],
        &[ObligationArgs {
            deposits: vec![(usdc_mint::id(), 100 * FRACTIONAL_TO_USDC)],
            borrows: vec![(wsol_mint::id(), 1)],
        }],
    )
    .await;

    // try to borrow 1 unit of bonk
    let bonk_reserve = reserves
        .iter()
        .find(|r| r.account.liquidity.mint_pubkey == bonk_mint::id())
        .unwrap();

    let err = lending_market
        .borrow_obligation_liquidity(
            &mut test,
            bonk_reserve,
            &obligations[0],
            &users[0],
            &NULL_PUBKEY,
            1,
        )
        .await
        .unwrap_err()
        .unwrap();
    assert_eq!(
        err,
        TransactionError::InstructionError(
            1,
            InstructionError::Custom(LendingError::IsolatedTierAssetViolation as u32)
        )
    );
}

#[tokio::test]
async fn borrow_regular_asset_invalid() {
    let (mut test, lending_market, reserves, obligations, users, _) = custom_scenario(
        &[
            ReserveArgs {
                mint: usdc_mint::id(),
                config: test_reserve_config(),
                liquidity_amount: 100_000 * FRACTIONAL_TO_USDC,
                price: PriceArgs {
                    price: 1,
                    conf: 0,
                    expo: 0,
                    ema_price: 1,
                    ema_conf: 0,
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
                    protocol_liquidation_fee: 0,
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
            ReserveArgs {
                mint: bonk_mint::id(),
                config: ReserveConfig {
                    loan_to_value_ratio: 0,
                    liquidation_threshold: 0,
                    fees: ReserveFees::default(),
                    optimal_borrow_rate: 0,
                    max_borrow_rate: 0,
                    protocol_liquidation_fee: 0,
                    reserve_type: ReserveType::Isolated,
                    ..test_reserve_config()
                },
                liquidity_amount: 1_000_000,
                price: PriceArgs {
                    price: 1,
                    conf: 0,
                    expo: -6,
                    ema_price: 1,
                    ema_conf: 0,
                },
            },
        ],
        &[ObligationArgs {
            deposits: vec![(usdc_mint::id(), 100 * FRACTIONAL_TO_USDC)],
            borrows: vec![(bonk_mint::id(), 1)],
        }],
    )
    .await;

    // borrow LAMPORTS_PER_SOL wsol
    let wsol_reserve = reserves
        .iter()
        .find(|r| r.account.liquidity.mint_pubkey == wsol_mint::id())
        .unwrap();

    // should error bc we are already borrowing bonk, which is an isolated tier asset
    let err = lending_market
        .borrow_obligation_liquidity(
            &mut test,
            wsol_reserve,
            &obligations[0],
            &users[0],
            &NULL_PUBKEY,
            LAMPORTS_PER_SOL,
        )
        .await
        .unwrap_err()
        .unwrap();

    assert_eq!(
        err,
        TransactionError::InstructionError(
            1,
            InstructionError::Custom(LendingError::IsolatedTierAssetViolation as u32)
        )
    );
}

#[tokio::test]
async fn invalid_borrow_due_to_reserve_config_change() {
    let (mut test, lending_market, reserves, obligations, users, lending_market_owner) =
        custom_scenario(
            &[
                ReserveArgs {
                    mint: usdc_mint::id(),
                    config: test_reserve_config(),
                    liquidity_amount: 100_000 * FRACTIONAL_TO_USDC,
                    price: PriceArgs {
                        price: 1,
                        conf: 0,
                        expo: 0,
                        ema_price: 1,
                        ema_conf: 0,
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
                        protocol_liquidation_fee: 0,
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
                ReserveArgs {
                    mint: bonk_mint::id(),
                    config: ReserveConfig {
                        loan_to_value_ratio: 0,
                        liquidation_threshold: 0,
                        fees: ReserveFees::default(),
                        optimal_borrow_rate: 0,
                        max_borrow_rate: 0,
                        protocol_liquidation_fee: 0,
                        reserve_type: ReserveType::Regular, // regular for now
                        ..test_reserve_config()
                    },
                    liquidity_amount: 1_000_000,
                    price: PriceArgs {
                        price: 1,
                        conf: 0,
                        expo: -6,
                        ema_price: 1,
                        ema_conf: 0,
                    },
                },
            ],
            &[ObligationArgs {
                deposits: vec![(usdc_mint::id(), 100 * FRACTIONAL_TO_USDC)],
                borrows: vec![(bonk_mint::id(), 1), (wsol_mint::id(), LAMPORTS_PER_SOL)],
            }],
        )
        .await;

    // update reserve config such that the bonk reserve is now of asset type isolated
    let bonk_reserve = reserves
        .iter()
        .find(|r| r.account.liquidity.mint_pubkey == bonk_mint::id())
        .unwrap();

    lending_market
        .update_reserve_config(
            &mut test,
            &lending_market_owner,
            bonk_reserve,
            ReserveConfig {
                reserve_type: ReserveType::Isolated,
                ..bonk_reserve.account.config
            },
            bonk_reserve.account.rate_limiter.config,
            None,
        )
        .await
        .unwrap();

    // borrow 1 more unit of BONK. this should fail because the reserve is now isolated but the
    // obligation has two borrows
    let err = lending_market
        .borrow_obligation_liquidity(
            &mut test,
            bonk_reserve,
            &obligations[0],
            &users[0],
            &NULL_PUBKEY,
            1,
        )
        .await
        .unwrap_err()
        .unwrap();

    assert_eq!(
        err,
        TransactionError::InstructionError(
            1,
            InstructionError::Custom(LendingError::IsolatedTierAssetViolation as u32)
        )
    );
}

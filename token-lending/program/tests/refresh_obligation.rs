#![cfg(feature = "test-bpf")]

mod helpers;

use crate::solend_program_test::custom_scenario;
use crate::solend_program_test::ObligationArgs;
use crate::solend_program_test::PriceArgs;
use crate::solend_program_test::ReserveArgs;
use std::collections::HashSet;

use helpers::solend_program_test::{setup_world, BalanceChecker, Info, SolendProgramTest, User};
use helpers::*;
use solana_program::native_token::LAMPORTS_PER_SOL;
use solana_program_test::*;
use solana_sdk::signature::Keypair;
use solend_program::state::SLOTS_PER_YEAR;
use solend_program::state::{LastUpdate, ObligationLiquidity, ReserveFees, ReserveLiquidity};

use solend_program::{
    math::{Decimal, TryAdd, TryDiv, TryMul},
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

    test.set_price(
        &usdc_mint::id(),
        &PriceArgs {
            price: 10,
            conf: 1,
            expo: -1,
            ema_price: 9,
            ema_conf: 1,
        },
    )
    .await;

    test.set_price(
        &wsol_mint::id(),
        &PriceArgs {
            price: 10,
            conf: 1,
            expo: 0,
            ema_price: 11,
            ema_conf: 1,
        },
    )
    .await;

    test.advance_clock_by_slots(1).await;

    let balance_checker =
        BalanceChecker::start(&mut test, &[&usdc_reserve, &user, &wsol_reserve]).await;

    lending_market
        .refresh_obligation(&mut test, &obligation)
        .await
        .unwrap();

    // check token balances
    let (balance_changes, mint_supply_changes) =
        balance_checker.find_balance_changes(&mut test).await;
    assert_eq!(balance_changes, HashSet::new());
    assert_eq!(mint_supply_changes, HashSet::new());

    // check program state
    let lending_market_post = test
        .load_account::<LendingMarket>(lending_market.pubkey)
        .await;
    assert_eq!(lending_market_post, lending_market);

    let usdc_reserve_post = test.load_account::<Reserve>(usdc_reserve.pubkey).await;
    assert_eq!(
        usdc_reserve_post.account,
        Reserve {
            last_update: LastUpdate {
                slot: 1001,
                stale: false
            },
            liquidity: ReserveLiquidity {
                smoothed_market_price: Decimal::from_percent(90),
                ..usdc_reserve.account.liquidity
            },
            ..usdc_reserve.account
        }
    );

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
        .unwrap();

    assert_eq!(
        wsol_reserve_post.account,
        Reserve {
            last_update: LastUpdate {
                slot: 1001,
                stale: true
            },
            liquidity: ReserveLiquidity {
                available_amount: 0,
                borrowed_amount_wads: new_borrowed_amount_wads,
                cumulative_borrow_rate_wads: new_cumulative_borrow_rate,
                smoothed_market_price: Decimal::from(11u64),
                ..wsol_reserve.account.liquidity
            },
            ..wsol_reserve.account
        }
    );

    let obligation_post = test.load_account::<Obligation>(obligation.pubkey).await;
    let new_borrow_value = new_borrowed_amount_wads
        .try_mul(Decimal::from(10u64))
        .unwrap()
        .try_div(Decimal::from(LAMPORTS_PER_SOL))
        .unwrap();

    assert_eq!(
        obligation_post.account,
        Obligation {
            last_update: LastUpdate {
                slot: 1001,
                stale: false
            },
            borrows: [ObligationLiquidity {
                borrow_reserve: wsol_reserve.pubkey,
                cumulative_borrow_rate_wads: new_cumulative_borrow_rate,
                borrowed_amount_wads: new_borrowed_amount_wads,
                market_value: new_borrow_value
            }]
            .to_vec(),

            borrowed_value: new_borrowed_amount_wads
                .try_mul(Decimal::from(10u64))
                .unwrap()
                .try_div(Decimal::from(LAMPORTS_PER_SOL))
                .unwrap(),

            // uses max(10, 11) = 11 for sol price
            borrowed_value_upper_bound: new_borrowed_amount_wads
                .try_mul(Decimal::from(11u64))
                .unwrap()
                .try_div(Decimal::from(LAMPORTS_PER_SOL))
                .unwrap(),

            // uses min(1, 0.9) for usdc price
            allowed_borrow_value: Decimal::from(100_000u64)
                .try_mul(Decimal::from_percent(
                    usdc_reserve.account.config.loan_to_value_ratio
                ))
                .unwrap()
                .try_mul(Decimal::from_percent(90))
                .unwrap(),

            ..obligation.account
        }
    );
}

#[tokio::test]
async fn test_obligation_liquidity_ordering() {
    let (mut test, lending_market, reserves, obligations, _users, lending_market_owner) =
        custom_scenario(
            &[
                ReserveArgs {
                    mint: usdc_mint::id(),
                    config: ReserveConfig {
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
                        loan_to_value_ratio: 0,
                        liquidation_threshold: 0,
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
            &[ObligationArgs {
                deposits: vec![(usdc_mint::id(), 100 * FRACTIONAL_TO_USDC)],
                borrows: vec![
                    (wsol_mint::id(), LAMPORTS_PER_SOL),
                    (usdc_mint::id(), FRACTIONAL_TO_USDC),
                ],
            }],
        )
        .await;

    // update usdc borrow weight to 20_000
    let usdc_reserve = reserves
        .iter()
        .find(|r| r.account.liquidity.mint_pubkey == usdc_mint::id())
        .unwrap();
    let wsol_reserve = reserves
        .iter()
        .find(|r| r.account.liquidity.mint_pubkey == wsol_mint::id())
        .unwrap();

    lending_market
        .update_reserve_config(
            &mut test,
            &lending_market_owner,
            usdc_reserve,
            ReserveConfig {
                added_borrow_weight_bps: 50_000,
                ..usdc_reserve.account.config
            },
            usdc_reserve.account.rate_limiter.config,
            None,
        )
        .await
        .unwrap();

    test.advance_clock_by_slots(1).await;

    lending_market
        .refresh_obligation(&mut test, &obligations[0])
        .await
        .unwrap();

    let obligation = test.load_account::<Obligation>(obligations[0].pubkey).await;
    assert_eq!(
        obligation.account.borrows,
        vec![
            ObligationLiquidity {
                borrow_reserve: usdc_reserve.pubkey,
                cumulative_borrow_rate_wads: Decimal::from(1u64),
                borrowed_amount_wads: Decimal::from(FRACTIONAL_TO_USDC),
                market_value: Decimal::from(1u64),
            },
            ObligationLiquidity {
                borrow_reserve: wsol_reserve.pubkey,
                cumulative_borrow_rate_wads: Decimal::from(1u64),
                borrowed_amount_wads: Decimal::from(LAMPORTS_PER_SOL),
                market_value: Decimal::from(10u64),
            },
        ]
    );

    lending_market
        .update_reserve_config(
            &mut test,
            &lending_market_owner,
            wsol_reserve,
            ReserveConfig {
                added_borrow_weight_bps: 100_000,
                ..wsol_reserve.account.config
            },
            wsol_reserve.account.rate_limiter.config,
            None,
        )
        .await
        .unwrap();
    lending_market
        .refresh_obligation(&mut test, &obligations[0])
        .await
        .unwrap();

    let obligation = test.load_account::<Obligation>(obligations[0].pubkey).await;
    assert_eq!(
        obligation.account.borrows,
        vec![
            ObligationLiquidity {
                borrow_reserve: wsol_reserve.pubkey,
                cumulative_borrow_rate_wads: Decimal::from(1u64),
                borrowed_amount_wads: Decimal::from(LAMPORTS_PER_SOL),
                market_value: Decimal::from(10u64),
            },
            ObligationLiquidity {
                borrow_reserve: usdc_reserve.pubkey,
                cumulative_borrow_rate_wads: Decimal::from(1u64),
                borrowed_amount_wads: Decimal::from(FRACTIONAL_TO_USDC),
                market_value: Decimal::from(1u64),
            },
        ]
    );
}

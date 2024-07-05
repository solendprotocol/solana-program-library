#![cfg(feature = "test-bpf")]

mod helpers;

use crate::solend_program_test::custom_scenario;
use crate::solend_program_test::setup_world;
use crate::solend_program_test::BalanceChecker;
use crate::solend_program_test::Info;
use crate::solend_program_test::Oracle;
use crate::solend_program_test::PriceArgs;
use crate::solend_program_test::ReserveArgs;
use crate::solend_program_test::SolendProgramTest;
use crate::solend_program_test::SwitchboardPriceArgs;
use crate::solend_program_test::User;
use helpers::*;
use solana_program::instruction::InstructionError;
use solana_program::native_token::LAMPORTS_PER_SOL;
use solana_program_test::*;
use solana_sdk::{signature::Keypair, transaction::TransactionError};
use solend_program::state::LastUpdate;
use solend_program::state::LendingMarket;
use solend_program::state::Obligation;
use solend_program::state::Reserve;
use solend_program::state::ReserveConfig;
use solend_program::state::ReserveFees;
use solend_program::state::ReserveLiquidity;
use solend_program::NULL_PUBKEY;
use solend_program::{
    error::LendingError,
    math::{Decimal, Rate, TryAdd, TryDiv, TryMul, TrySub},
    state::SLOTS_PER_YEAR,
};
use std::collections::HashSet;

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
                protocol_take_rate: 10,
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

    // borrow 6 SOL against 100k cUSDC. All sol is borrowed, so the borrow rate should be at max.
    let obligation = test.load_account::<Obligation>(obligation.pubkey).await;
    lending_market
        .borrow_obligation_liquidity(
            &mut test,
            &wsol_reserve,
            &obligation,
            &user,
            lending_market_owner.get_account(&wsol_mint::id()),
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
        lending_market_owner,
        obligation,
    )
}

#[tokio::test]
async fn test_success() {
    let (mut test, lending_market, _, wsol_reserve, _, _) = setup().await;

    // should be maxed out at 30%
    let borrow_rate = wsol_reserve.account.current_borrow_rate().unwrap();

    test.set_price(
        &wsol_mint::id(),
        &PriceArgs {
            price: 20,
            conf: 1,
            expo: 1,
            ema_price: 15,
            ema_conf: 1,
        },
    )
    .await;

    test.advance_clock_by_slots(1).await;
    let balance_checker = BalanceChecker::start(&mut test, &[&wsol_reserve]).await;

    lending_market
        .refresh_reserve(&mut test, &wsol_reserve)
        .await
        .unwrap();

    // check balances
    assert_eq!(
        balance_checker.find_balance_changes(&mut test).await,
        (HashSet::new(), HashSet::new())
    );

    // check program state
    let wsol_reserve_post = test.load_account::<Reserve>(wsol_reserve.pubkey).await;

    let slot_rate = borrow_rate.try_div(SLOTS_PER_YEAR).unwrap();
    let compound_rate = Rate::one().try_add(slot_rate).unwrap();
    let compound_borrow = Decimal::from(6 * LAMPORTS_PER_SOL)
        .try_mul(compound_rate)
        .unwrap();
    let net_new_debt = compound_borrow
        .try_sub(Decimal::from(6 * LAMPORTS_PER_SOL))
        .unwrap();
    let protocol_take_rate = Rate::from_percent(wsol_reserve.account.config.protocol_take_rate);
    let delta_accumulated_protocol_fees = net_new_debt.try_mul(protocol_take_rate).unwrap();

    assert_eq!(
        wsol_reserve_post.account,
        Reserve {
            last_update: LastUpdate {
                slot: 1001,
                stale: false
            },
            liquidity: ReserveLiquidity {
                borrowed_amount_wads: compound_borrow,
                cumulative_borrow_rate_wads: compound_rate.into(),
                accumulated_protocol_fees_wads: delta_accumulated_protocol_fees,
                market_price: Decimal::from(200u64),
                smoothed_market_price: Decimal::from(150u64),
                ..wsol_reserve.account.liquidity
            },
            ..wsol_reserve.account
        }
    );
}

#[tokio::test]
async fn test_fail_pyth_price_stale() {
    let (mut test, lending_market, _usdc_reserve, wsol_reserve, _user, _obligation) = setup().await;

    test.advance_clock_by_slots(241).await;

    let res = lending_market
        .refresh_reserve(&mut test, &wsol_reserve)
        .await
        .unwrap_err()
        .unwrap();
    println!("{:?}", res);

    assert_eq!(
        res,
        TransactionError::InstructionError(
            1,
            InstructionError::Custom(LendingError::InvalidOracleConfig as u32),
        ),
    );
}

#[tokio::test]
async fn test_success_pyth_price_stale_switchboard_valid() {
    let (mut test, lending_market, _, wsol_reserve, lending_market_owner, _) = setup().await;

    test.set_price(
        &wsol_mint::id(),
        &PriceArgs {
            price: 9,
            conf: 0,
            expo: 0,
            ema_price: 11,
            ema_conf: 0,
        },
    )
    .await;
    test.advance_clock_by_slots(1).await;

    lending_market
        .refresh_reserve(&mut test, &wsol_reserve)
        .await
        .unwrap();

    test.advance_clock_by_slots(241).await;

    test.init_switchboard_feed(&wsol_mint::id()).await;
    test.set_switchboard_price(&wsol_mint::id(), SwitchboardPriceArgs { price: 8, expo: 0 })
        .await;

    // update reserve so the switchboard feed is not NULL_PUBKEY
    lending_market
        .update_reserve_config(
            &mut test,
            &lending_market_owner,
            &wsol_reserve,
            wsol_reserve.account.config,
            wsol_reserve.account.rate_limiter.config,
            None,
        )
        .await
        .unwrap();

    let wsol_reserve = test.load_account::<Reserve>(wsol_reserve.pubkey).await;
    lending_market
        .refresh_reserve(&mut test, &wsol_reserve)
        .await
        .unwrap();

    let wsol_reserve_post = test.load_account::<Reserve>(wsol_reserve.pubkey).await;

    // overwrite liquidity market price with the switchboard price but keep the pyth ema price
    assert_eq!(
        wsol_reserve_post.account.liquidity.market_price,
        Decimal::from(8u64)
    );
    assert_eq!(
        wsol_reserve_post.account.liquidity.smoothed_market_price,
        Decimal::from(11u64)
    );

    test.advance_clock_by_slots(241).await;
    let err = lending_market
        .refresh_reserve(&mut test, &wsol_reserve)
        .await
        .unwrap_err()
        .unwrap();
    assert_eq!(
        err,
        TransactionError::InstructionError(
            1,
            InstructionError::Custom(LendingError::InvalidOracleConfig as u32)
        )
    );
}

#[tokio::test]
async fn test_success_only_switchboard_reserve() {
    let (mut test, lending_market, _, wsol_reserve, lending_market_owner, _) = setup().await;

    test.set_price(
        &wsol_mint::id(),
        &PriceArgs {
            price: 10,
            conf: 0,
            expo: 0,
            ema_price: 11,
            ema_conf: 0,
        },
    )
    .await;

    test.advance_clock_by_slots(1).await;

    let feed = test.init_switchboard_feed(&wsol_mint::id()).await;
    test.set_switchboard_price(&wsol_mint::id(), SwitchboardPriceArgs { price: 8, expo: 0 })
        .await;

    test.advance_clock_by_slots(1).await;

    lending_market
        .update_reserve_config(
            &mut test,
            &lending_market_owner,
            &wsol_reserve,
            wsol_reserve.account.config,
            wsol_reserve.account.rate_limiter.config,
            Some(&Oracle {
                pyth_price_pubkey: NULL_PUBKEY,
                pyth_product_pubkey: NULL_PUBKEY,
                switchboard_feed_pubkey: Some(feed),
            }),
        )
        .await
        .unwrap();

    test.advance_clock_by_slots(1).await;

    let wsol_reserve = test.load_account::<Reserve>(wsol_reserve.pubkey).await;
    lending_market
        .refresh_reserve(&mut test, &wsol_reserve)
        .await
        .unwrap();

    let wsol_reserve_post = test.load_account::<Reserve>(wsol_reserve.pubkey).await;

    // when pyth is null and only switchboard exists, both price fields get overwritten
    assert_eq!(
        wsol_reserve_post.account.liquidity.market_price,
        Decimal::from(8u64)
    );
    assert_eq!(
        wsol_reserve_post.account.liquidity.smoothed_market_price,
        Decimal::from(8u64)
    );
}

#[tokio::test]
async fn test_use_price_weight() {
    let (mut test, lending_market, reserves, _obligations, _users, lending_market_owner) =
        custom_scenario(
            &[ReserveArgs {
                mint: wsol_mint::id(),
                config: ReserveConfig {
                    scaled_price_offset_bps: 2_000,
                    ..test_reserve_config()
                },
                liquidity_amount: 100_000 * FRACTIONAL_TO_USDC,
                price: PriceArgs {
                    price: 10,
                    conf: 0,
                    expo: 0,
                    ema_price: 10,
                    ema_conf: 0,
                },
            }],
            &[],
        )
        .await;

    test.set_price(
        &wsol_mint::id(),
        &PriceArgs {
            price: 10,
            conf: 0,
            expo: 0,
            ema_price: 20,
            ema_conf: 0,
        },
    )
    .await;

    test.advance_clock_by_slots(1).await;

    lending_market
        .refresh_reserve(&mut test, &reserves[0])
        .await
        .unwrap();

    let wsol_reserve = test.load_account::<Reserve>(reserves[0].pubkey).await;
    assert_eq!(
        wsol_reserve.account.liquidity.market_price,
        Decimal::from(12u64)
    );
    assert_eq!(
        wsol_reserve.account.liquidity.smoothed_market_price,
        Decimal::from(24u64)
    );

    test.advance_clock_by_slots(1).await;

    lending_market
        .update_reserve_config(
            &mut test,
            &lending_market_owner,
            &wsol_reserve,
            ReserveConfig {
                scaled_price_offset_bps: -2_000,
                ..wsol_reserve.account.config
            },
            wsol_reserve.account.rate_limiter.config,
            None,
        )
        .await
        .unwrap();

    test.advance_clock_by_slots(1).await;

    lending_market
        .refresh_reserve(&mut test, &reserves[0])
        .await
        .unwrap();

    let wsol_reserve = test.load_account::<Reserve>(reserves[0].pubkey).await;
    assert_eq!(
        wsol_reserve.account.liquidity.market_price,
        Decimal::from(8u64)
    );
    assert_eq!(
        wsol_reserve.account.liquidity.smoothed_market_price,
        Decimal::from(16u64)
    );

    // check we do same thing for switchboard
    let switchboard_feed_pubkey = Some(test.init_switchboard_feed(&wsol_mint::id()).await);
    test.set_switchboard_price(
        &wsol_mint::id(),
        SwitchboardPriceArgs { price: 30, expo: 0 },
    )
    .await;

    // update reserve so the switchboard feed is not NULL_PUBKEY
    lending_market
        .update_reserve_config(
            &mut test,
            &lending_market_owner,
            &wsol_reserve,
            wsol_reserve.account.config,
            wsol_reserve.account.rate_limiter.config,
            Some(&Oracle {
                pyth_price_pubkey: NULL_PUBKEY,
                pyth_product_pubkey: NULL_PUBKEY,
                switchboard_feed_pubkey,
            }),
        )
        .await
        .unwrap();

    test.advance_clock_by_slots(1).await;

    let wsol_reserve = test.load_account::<Reserve>(reserves[0].pubkey).await;
    lending_market
        .refresh_reserve(&mut test, &wsol_reserve)
        .await
        .unwrap();

    let wsol_reserve = test.load_account::<Reserve>(reserves[0].pubkey).await;
    assert_eq!(
        wsol_reserve.account.liquidity.market_price,
        Decimal::from(24u64)
    );

    assert_eq!(
        wsol_reserve.account.liquidity.smoothed_market_price,
        Decimal::from(24u64)
    );
}

#[tokio::test]
async fn test_use_extra_oracle() {
    let (mut test, lending_market, reserves, _obligations, _users, lending_market_owner) =
        custom_scenario(
            &[ReserveArgs {
                mint: msol_mint::id(),
                config: test_reserve_config(),
                liquidity_amount: 1000,
                price: PriceArgs {
                    price: 10,
                    conf: 0,
                    expo: 0,
                    ema_price: 10,
                    ema_conf: 0,
                },
            }],
            &[],
        )
        .await;

    let msol_reserve = &reserves[0];

    // add extra pyth oracle
    let wsol_pyth_feed = test.init_pyth_feed(&wsol_mint::id()).await;
    test.set_price(
        &wsol_mint::id(),
        &PriceArgs {
            price: 5,
            conf: 0,
            expo: 0,
            ema_price: 5,
            ema_conf: 0,
        },
    )
    .await;

    lending_market
        .update_reserve_config(
            &mut test,
            &lending_market_owner,
            msol_reserve,
            ReserveConfig {
                extra_oracle_pubkey: Some(wsol_pyth_feed),
                scaled_price_offset_bps: 2_000,
                ..msol_reserve.account.config
            },
            msol_reserve.account.rate_limiter.config,
            None,
        )
        .await
        .unwrap();

    test.advance_clock_by_slots(1).await;

    let msol_reserve = test.load_account::<Reserve>(reserves[0].pubkey).await;
    lending_market
        .refresh_reserve(&mut test, &msol_reserve)
        .await
        .unwrap();

    let msol_reserve_post = test.load_account::<Reserve>(reserves[0].pubkey).await;
    assert_eq!(
        msol_reserve_post.account,
        Reserve {
            last_update: LastUpdate {
                slot: 1001,
                stale: false
            },
            liquidity: ReserveLiquidity {
                market_price: Decimal::from(12u64),
                smoothed_market_price: Decimal::from(12u64),
                extra_market_price: Some(Decimal::from(5u64)),
                ..msol_reserve.account.liquidity
            },
            ..msol_reserve.account
        }
    );

    // add extra switchboard oracle
    let msol_reserve = test.load_account::<Reserve>(reserves[0].pubkey).await;
    let wsol_switchboard_feed = test.init_switchboard_feed(&wsol_mint::id()).await;
    test.set_switchboard_price(&wsol_mint::id(), SwitchboardPriceArgs { price: 2, expo: 0 })
        .await;

    lending_market
        .update_reserve_config(
            &mut test,
            &lending_market_owner,
            &msol_reserve,
            ReserveConfig {
                extra_oracle_pubkey: Some(wsol_switchboard_feed),
                ..msol_reserve.account.config
            },
            msol_reserve.account.rate_limiter.config,
            None,
        )
        .await
        .unwrap();

    test.advance_clock_by_slots(1).await;

    let msol_reserve = test.load_account::<Reserve>(reserves[0].pubkey).await;
    lending_market
        .refresh_reserve(&mut test, &msol_reserve)
        .await
        .unwrap();

    let msol_reserve_post = test.load_account::<Reserve>(reserves[0].pubkey).await;
    assert_eq!(
        msol_reserve_post.account,
        Reserve {
            last_update: LastUpdate {
                slot: 1002,
                stale: false
            },
            liquidity: ReserveLiquidity {
                extra_market_price: Some(Decimal::from(2u64)),
                ..msol_reserve.account.liquidity
            },
            ..msol_reserve.account
        }
    );

    test.advance_clock_by_slots(1).await;

    // remove extra oracle, make sure extra price is None now
    let msol_reserve = test.load_account::<Reserve>(reserves[0].pubkey).await;
    lending_market
        .update_reserve_config(
            &mut test,
            &lending_market_owner,
            &msol_reserve,
            ReserveConfig {
                extra_oracle_pubkey: None,
                ..msol_reserve.account.config
            },
            msol_reserve.account.rate_limiter.config,
            None,
        )
        .await
        .unwrap();
    let msol_reserve = test.load_account::<Reserve>(reserves[0].pubkey).await;

    lending_market
        .refresh_reserve(&mut test, &msol_reserve)
        .await
        .unwrap();

    let msol_reserve_post = test.load_account::<Reserve>(reserves[0].pubkey).await;
    assert_eq!(
        msol_reserve_post.account,
        Reserve {
            last_update: LastUpdate {
                slot: 1003,
                stale: false
            },
            liquidity: ReserveLiquidity {
                extra_market_price: None,
                ..msol_reserve.account.liquidity
            },
            ..msol_reserve.account
        }
    );
}

#[tokio::test]
async fn test_use_extra_oracle_bad_cases() {
    let (mut test, lending_market, reserves, _obligations, _users, lending_market_owner) =
        custom_scenario(
            &[ReserveArgs {
                mint: msol_mint::id(),
                config: test_reserve_config(),
                liquidity_amount: 1000,
                price: PriceArgs {
                    price: 10,
                    conf: 0,
                    expo: 0,
                    ema_price: 10,
                    ema_conf: 0,
                },
            }],
            &[],
        )
        .await;

    let msol_reserve = &reserves[0];

    // add extra pyth oracle
    let wsol_pyth_feed = test.init_pyth_feed(&wsol_mint::id()).await;
    test.set_price(
        &wsol_mint::id(),
        &PriceArgs {
            price: 5,
            conf: 0,
            expo: 0,
            ema_price: 5,
            ema_conf: 0,
        },
    )
    .await;

    lending_market
        .update_reserve_config(
            &mut test,
            &lending_market_owner,
            msol_reserve,
            ReserveConfig {
                extra_oracle_pubkey: Some(wsol_pyth_feed),
                scaled_price_offset_bps: 2_000,
                ..msol_reserve.account.config
            },
            msol_reserve.account.rate_limiter.config,
            None,
        )
        .await
        .unwrap();

    test.advance_clock_by_slots(1000).await;

    test.set_price(
        &msol_mint::id(),
        &PriceArgs {
            price: 10,
            conf: 0,
            expo: 0,
            ema_price: 10,
            ema_conf: 0,
        },
    )
    .await;

    let mut msol_reserve = test.load_account::<Reserve>(reserves[0].pubkey).await;

    // this no longer fails because the extra oracle is not checked for staleness/variance
    lending_market
        .refresh_reserve(&mut test, &msol_reserve)
        .await
        .unwrap();

    msol_reserve.account.config.extra_oracle_pubkey =
        Some(msol_reserve.account.liquidity.pyth_oracle_pubkey);
    // this should fail because the extra oracle passed in doesn't match
    let err = lending_market
        .refresh_reserve(&mut test, &msol_reserve)
        .await
        .unwrap_err()
        .unwrap();
    assert_eq!(
        err,
        TransactionError::InstructionError(
            1,
            InstructionError::Custom(LendingError::InvalidAccountInput as u32)
        )
    );

    msol_reserve.account.config.extra_oracle_pubkey = None;
    // this should fail because there's no extra oracle passed in
    let err = lending_market
        .refresh_reserve(&mut test, &msol_reserve)
        .await
        .unwrap_err()
        .unwrap();
    assert_eq!(
        err,
        TransactionError::InstructionError(
            1,
            InstructionError::Custom(LendingError::InvalidAccountInput as u32)
        )
    );
}

#[tokio::test]
async fn test_pyth_pull_oracle() {
    let (mut test, lending_market, _, wsol_reserve, lending_market_owner, _) = setup().await;

    test.set_price(
        &wsol_mint::id(),
        &PriceArgs {
            price: 10,
            conf: 0,
            expo: 0,
            ema_price: 11,
            ema_conf: 0,
        },
    )
    .await;

    test.advance_clock_by_slots(1).await;

    let feed = test.init_pyth_pull_feed(&wsol_mint::id()).await;

    test.advance_clock_by_slots(1).await;

    lending_market
        .update_reserve_config(
            &mut test,
            &lending_market_owner,
            &wsol_reserve,
            wsol_reserve.account.config,
            wsol_reserve.account.rate_limiter.config,
            Some(&Oracle {
                pyth_price_pubkey: feed,
                pyth_product_pubkey: NULL_PUBKEY,
                switchboard_feed_pubkey: None,
            }),
        )
        .await
        .unwrap();

    test.advance_clock_by_slots(1).await;

    test.set_pyth_pull_price(
        &wsol_mint::id(),
        &PriceArgs {
            price: 8,
            conf: 0,
            expo: 0,
            ema_price: 9,
            ema_conf: 0,
        },
    )
    .await;


    let wsol_reserve = test.load_account::<Reserve>(wsol_reserve.pubkey).await;
    lending_market
        .refresh_reserve(&mut test, &wsol_reserve)
        .await
        .unwrap();

    let wsol_reserve_post = test.load_account::<Reserve>(wsol_reserve.pubkey).await;

    assert_eq!(
        wsol_reserve_post.account.liquidity.market_price,
        Decimal::from(8u64)
    );
    assert_eq!(
        wsol_reserve_post.account.liquidity.smoothed_market_price,
        Decimal::from(9u64)
    );

    test.advance_clock_by_slots(1).await;

    // bad confidence interval
    test.set_pyth_pull_price(
        &wsol_mint::id(),
        &PriceArgs {
            price: 8,
            conf: 10,
            expo: 0,
            ema_price: 9,
            ema_conf: 0,
        },
    )
    .await;

    let wsol_reserve = test.load_account::<Reserve>(wsol_reserve.pubkey).await;
    let res = lending_market
        .refresh_reserve(&mut test, &wsol_reserve)
        .await
        .unwrap_err()
        .unwrap();

    assert_eq!(
        res,
        TransactionError::InstructionError(
            1,
            InstructionError::Custom(LendingError::InvalidOracleConfig as u32),
        ),
    );
}

#[tokio::test]
async fn test_switchboard_pull_oracle() {
    let (mut test, lending_market, _, wsol_reserve, lending_market_owner, _) = setup().await;

    test.set_price(
        &wsol_mint::id(),
        &PriceArgs {
            price: 10,
            conf: 0,
            expo: 0,
            ema_price: 11,
            ema_conf: 0,
        },
    )
    .await;

    test.advance_clock_by_slots(1).await;

    let feed = test.init_switchboard_pull_feed(&wsol_mint::id()).await;

    test.advance_clock_by_slots(1).await;

    lending_market
        .update_reserve_config(
            &mut test,
            &lending_market_owner,
            &wsol_reserve,
            wsol_reserve.account.config,
            wsol_reserve.account.rate_limiter.config,
            Some(&Oracle {
                pyth_price_pubkey: NULL_PUBKEY,
                pyth_product_pubkey: NULL_PUBKEY,
                switchboard_feed_pubkey: Some(feed),
            }),
        )
        .await
        .unwrap();

    test.advance_clock_by_slots(1).await;

    test.set_switchboard_pull_price(&wsol_mint::id(), SwitchboardPriceArgs { price: 10, expo: 0 }).await;

    test.advance_clock_by_slots(1).await;

    let wsol_reserve = test.load_account::<Reserve>(wsol_reserve.pubkey).await;
    lending_market
        .refresh_reserve(&mut test, &wsol_reserve)
        .await
        .unwrap();

    let wsol_reserve_post = test.load_account::<Reserve>(wsol_reserve.pubkey).await;

    assert_eq!(
        wsol_reserve_post.account.liquidity.market_price,
        Decimal::from(10u64)
    );
    assert_eq!(
        wsol_reserve_post.account.liquidity.smoothed_market_price,
        Decimal::from(10u64)
    );

    test.advance_clock_by_slots(1000).await;

    // stale
    let wsol_reserve = test.load_account::<Reserve>(wsol_reserve.pubkey).await;
    let res = lending_market
        .refresh_reserve(&mut test, &wsol_reserve)
        .await
        .unwrap_err()
        .unwrap();

    assert_eq!(
        res,
        TransactionError::InstructionError(
            1,
            InstructionError::Custom(LendingError::InvalidOracleConfig as u32),
        ),
    );
}

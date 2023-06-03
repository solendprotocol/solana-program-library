#![cfg(feature = "test-bpf")]

use crate::helpers::solend_program_test::*;
use solana_program::pubkey::Pubkey;
use solana_sdk::signature::Signer;

use solend_program::math::TryDiv;
mod helpers;

use solend_program::state::{RateLimiterConfig, ReserveFees};
use std::collections::HashSet;

use helpers::*;
use solana_program::native_token::LAMPORTS_PER_SOL;
use solana_program_test::*;
use solana_sdk::{
    instruction::InstructionError, signature::Keypair, transaction::TransactionError,
};
use solend_program::state::*;
use solend_program::{error::LendingError, math::Decimal};

async fn setup(
    wsol_reserve_config: &ReserveConfig,
) -> (
    SolendProgramTest,
    Info<LendingMarket>,
    Info<Reserve>,
    Info<Reserve>,
    User,
    Info<Obligation>,
    User,
    User,
) {
    let (mut test, lending_market, usdc_reserve, wsol_reserve, lending_market_owner, user) =
        setup_world(&test_reserve_config(), wsol_reserve_config).await;

    let obligation = lending_market
        .init_obligation(&mut test, Keypair::new(), &user)
        .await
        .expect("This should succeed");

    lending_market
        .deposit(&mut test, &usdc_reserve, &user, 100_000_000)
        .await
        .expect("This should succeed");

    let usdc_reserve = test.load_account(usdc_reserve.pubkey).await;

    lending_market
        .deposit_obligation_collateral(&mut test, &usdc_reserve, &obligation, &user, 100_000_000)
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

    lending_market
        .deposit(
            &mut test,
            &wsol_reserve,
            &wsol_depositor,
            5 * LAMPORTS_PER_SOL,
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

    let host_fee_receiver = User::new_with_balances(&mut test, &[(&wsol_mint::id(), 0)]).await;
    (
        test,
        lending_market,
        usdc_reserve,
        wsol_reserve,
        user,
        obligation,
        host_fee_receiver,
        lending_market_owner,
    )
}

#[tokio::test]
async fn test_success() {
    let (
        mut test,
        lending_market,
        usdc_reserve,
        wsol_reserve,
        user,
        obligation,
        host_fee_receiver,
        _,
    ) = setup(&ReserveConfig {
        fees: ReserveFees {
            borrow_fee_wad: 100_000_000_000,
            flash_loan_fee_wad: 0,
            host_fee_percentage: 20,
        },
        ..test_reserve_config()
    })
    .await;

    let balance_checker = BalanceChecker::start(
        &mut test,
        &[&usdc_reserve, &user, &wsol_reserve, &host_fee_receiver],
    )
    .await;

    lending_market
        .borrow_obligation_liquidity(
            &mut test,
            &wsol_reserve,
            &obligation,
            &user,
            host_fee_receiver.get_account(&wsol_mint::id()),
            4 * LAMPORTS_PER_SOL,
        )
        .await
        .unwrap();

    // check token balances
    let (balance_changes, mint_supply_changes) =
        balance_checker.find_balance_changes(&mut test).await;

    let expected_balance_changes = HashSet::from([
        TokenBalanceChange {
            token_account: wsol_reserve.account.liquidity.supply_pubkey,
            mint: wsol_mint::id(),
            diff: -((4 * LAMPORTS_PER_SOL + 400) as i128),
        },
        TokenBalanceChange {
            token_account: user.get_account(&wsol_mint::id()).unwrap(),
            mint: wsol_mint::id(),
            diff: (4 * LAMPORTS_PER_SOL) as i128,
        },
        TokenBalanceChange {
            token_account: wsol_reserve.account.config.fee_receiver,
            mint: wsol_mint::id(),
            diff: 320,
        },
        TokenBalanceChange {
            token_account: host_fee_receiver.get_account(&wsol_mint::id()).unwrap(),
            mint: wsol_mint::id(),
            diff: 80,
        },
    ]);
    assert_eq!(
        balance_changes, expected_balance_changes,
        "{:#?} \n {:#?}",
        balance_changes, expected_balance_changes
    );
    assert_eq!(mint_supply_changes, HashSet::new());

    // check program state
    let lending_market_post = test
        .load_account::<LendingMarket>(lending_market.pubkey)
        .await;
    assert_eq!(
        lending_market_post.account,
        LendingMarket {
            rate_limiter: {
                let mut rate_limiter = lending_market.account.rate_limiter;
                rate_limiter
                    .update(
                        1000,
                        Decimal::from(10 * (4 * LAMPORTS_PER_SOL + 400))
                            .try_div(Decimal::from(1_000_000_000_u64))
                            .unwrap(),
                    )
                    .unwrap();
                rate_limiter
            },
            ..lending_market.account
        }
    );

    let wsol_reserve_post = test.load_account::<Reserve>(wsol_reserve.pubkey).await;
    let expected_wsol_reserve_post = Reserve {
        last_update: LastUpdate {
            slot: 1000,
            stale: true,
        },
        liquidity: ReserveLiquidity {
            available_amount: 6 * LAMPORTS_PER_SOL - (4 * LAMPORTS_PER_SOL + 400),
            borrowed_amount_wads: Decimal::from(4 * LAMPORTS_PER_SOL + 400),
            ..wsol_reserve.account.liquidity
        },
        rate_limiter: {
            let mut rate_limiter = wsol_reserve.account.rate_limiter;
            rate_limiter
                .update(1000, Decimal::from(4 * LAMPORTS_PER_SOL + 400))
                .unwrap();

            rate_limiter
        },
        ..wsol_reserve.account
    };

    assert_eq!(
        wsol_reserve_post.account, expected_wsol_reserve_post,
        "{:#?} {:#?}",
        wsol_reserve_post, expected_wsol_reserve_post
    );

    let obligation_post = test.load_account::<Obligation>(obligation.pubkey).await;
    assert_eq!(
        obligation_post.account,
        Obligation {
            last_update: LastUpdate {
                slot: 1000,
                stale: true
            },
            borrows: vec![ObligationLiquidity {
                borrow_reserve: wsol_reserve.pubkey,
                borrowed_amount_wads: Decimal::from(4 * LAMPORTS_PER_SOL + 400),
                cumulative_borrow_rate_wads: wsol_reserve
                    .account
                    .liquidity
                    .cumulative_borrow_rate_wads,
                market_value: Decimal::zero(), // we only update this retroactively on a
                                               // refresh_obligation
            }],
            deposited_value: Decimal::from(100u64),
            borrowed_value: Decimal::zero(),
            allowed_borrow_value: Decimal::from(50u64),
            unhealthy_borrow_value: Decimal::from(55u64),
            ..obligation.account
        },
        "{:#?}",
        obligation_post.account
    );
}

// FIXME this should really be a unit test
#[tokio::test]
async fn test_borrow_max() {
    let (
        mut test,
        lending_market,
        usdc_reserve,
        wsol_reserve,
        user,
        obligation,
        host_fee_receiver,
        _,
    ) = setup(&ReserveConfig {
        fees: ReserveFees {
            borrow_fee_wad: 100_000_000_000,
            flash_loan_fee_wad: 0,
            host_fee_percentage: 20,
        },
        ..test_reserve_config()
    })
    .await;

    let balance_checker = BalanceChecker::start(
        &mut test,
        &[&usdc_reserve, &user, &wsol_reserve, &host_fee_receiver],
    )
    .await;

    lending_market
        .borrow_obligation_liquidity(
            &mut test,
            &wsol_reserve,
            &obligation,
            &user,
            host_fee_receiver.get_account(&wsol_mint::id()),
            u64::MAX,
        )
        .await
        .unwrap();

    // check token balances
    let (balance_changes, mint_supply_changes) =
        balance_checker.find_balance_changes(&mut test).await;

    let expected_balance_changes = HashSet::from([
        TokenBalanceChange {
            token_account: wsol_reserve.account.liquidity.supply_pubkey,
            mint: wsol_mint::id(),
            diff: -((5 * LAMPORTS_PER_SOL) as i128),
        },
        TokenBalanceChange {
            token_account: user.get_account(&wsol_mint::id()).unwrap(),
            mint: wsol_mint::id(),
            diff: (5 * LAMPORTS_PER_SOL as i128) - 500,
        },
        TokenBalanceChange {
            token_account: wsol_reserve.account.config.fee_receiver,
            mint: wsol_mint::id(),
            diff: 400,
        },
        TokenBalanceChange {
            token_account: host_fee_receiver.get_account(&wsol_mint::id()).unwrap(),
            mint: wsol_mint::id(),
            diff: 100,
        },
    ]);

    assert_eq!(
        balance_changes, expected_balance_changes,
        "{:#?} \n {:#?}",
        balance_changes, expected_balance_changes
    );
    assert_eq!(mint_supply_changes, HashSet::new());
}

#[tokio::test]
async fn test_fail_borrow_over_reserve_borrow_limit() {
    let (mut test, lending_market, _, wsol_reserve, user, obligation, host_fee_receiver, _) =
        setup(&ReserveConfig {
            borrow_limit: LAMPORTS_PER_SOL,
            ..test_reserve_config()
        })
        .await;

    let res = lending_market
        .borrow_obligation_liquidity(
            &mut test,
            &wsol_reserve,
            &obligation,
            &user,
            host_fee_receiver.get_account(&wsol_mint::id()),
            LAMPORTS_PER_SOL + 1,
        )
        .await
        .err()
        .unwrap()
        .unwrap();

    assert_eq!(
        res,
        TransactionError::InstructionError(
            1,
            InstructionError::Custom(LendingError::InvalidAmount as u32)
        )
    );
}

#[tokio::test]
async fn test_fail_reserve_borrow_rate_limit_exceeded() {
    let (
        mut test,
        lending_market,
        _,
        wsol_reserve,
        user,
        obligation,
        host_fee_receiver,
        lending_market_owner,
    ) = setup(&ReserveConfig {
        ..test_reserve_config()
    })
    .await;

    // ie, within 10 slots, the maximum outflow is 1 SOL
    lending_market
        .update_reserve_config(
            &mut test,
            &lending_market_owner,
            &wsol_reserve,
            wsol_reserve.account.config,
            RateLimiterConfig {
                window_duration: 10,
                max_outflow: LAMPORTS_PER_SOL,
            },
            None,
        )
        .await
        .unwrap();

    // borrow maximum amount
    lending_market
        .borrow_obligation_liquidity(
            &mut test,
            &wsol_reserve,
            &obligation,
            &user,
            host_fee_receiver.get_account(&wsol_mint::id()),
            LAMPORTS_PER_SOL,
        )
        .await
        .unwrap();

    // for the next 10 slots, we shouldn't be able to borrow anything.
    let cur_slot = test.get_clock().await.slot;
    for _ in cur_slot..(cur_slot + 10) {
        let res = lending_market
            .borrow_obligation_liquidity(
                &mut test,
                &wsol_reserve,
                &obligation,
                &user,
                host_fee_receiver.get_account(&wsol_mint::id()),
                1,
            )
            .await
            .err()
            .unwrap()
            .unwrap();

        assert_eq!(
            res,
            TransactionError::InstructionError(
                1,
                InstructionError::Custom(LendingError::OutflowRateLimitExceeded as u32)
            )
        );

        test.advance_clock_by_slots(1).await;
    }

    // after 10 slots, we should be able to at borrow most 0.1 SOL
    let res = lending_market
        .borrow_obligation_liquidity(
            &mut test,
            &wsol_reserve,
            &obligation,
            &user,
            host_fee_receiver.get_account(&wsol_mint::id()),
            LAMPORTS_PER_SOL / 10 + 1,
        )
        .await
        .err()
        .unwrap()
        .unwrap();

    assert_eq!(
        res,
        TransactionError::InstructionError(
            1,
            InstructionError::Custom(LendingError::OutflowRateLimitExceeded as u32)
        )
    );

    lending_market
        .borrow_obligation_liquidity(
            &mut test,
            &wsol_reserve,
            &obligation,
            &user,
            host_fee_receiver.get_account(&wsol_mint::id()),
            LAMPORTS_PER_SOL / 10,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn test_borrow_max_rate_limiter() {
    let (mut test, lending_market, reserves, obligations, users, lending_market_owner) =
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
                deposits: vec![(usdc_mint::id(), 100 * FRACTIONAL_TO_USDC)],
                borrows: vec![],
            }],
        )
        .await;

    let wsol_reserve = &reserves[1];
    lending_market
        .update_reserve_config(
            &mut test,
            &lending_market_owner,
            wsol_reserve,
            wsol_reserve.account.config,
            RateLimiterConfig {
                window_duration: 20,
                max_outflow: LAMPORTS_PER_SOL,
            },
            None,
        )
        .await
        .unwrap();

    test.advance_clock_by_slots(1).await;

    let balance_checker = BalanceChecker::start(&mut test, &[wsol_reserve]).await;

    lending_market
        .borrow_obligation_liquidity(
            &mut test,
            wsol_reserve,
            &obligations[0],
            &users[0],
            None,
            u64::MAX,
        )
        .await
        .unwrap();

    // check token balances
    let (balance_changes, _mint_supply_changes) =
        balance_checker.find_balance_changes(&mut test).await;

    let expected_balance_changes = HashSet::from([TokenBalanceChange {
        token_account: wsol_reserve.account.liquidity.supply_pubkey,
        mint: wsol_mint::id(),
        diff: -(LAMPORTS_PER_SOL as i128),
    }]);

    assert_eq!(balance_changes, expected_balance_changes);

    test.advance_clock_by_slots(100).await;

    lending_market
        .set_lending_market_owner_and_config(
            &mut test,
            &lending_market_owner,
            &lending_market_owner.keypair.pubkey(),
            RateLimiterConfig {
                window_duration: 20,
                max_outflow: 5, // $50
            },
            Pubkey::new_unique(),
        )
        .await
        .unwrap();

    let balance_checker = BalanceChecker::start(&mut test, &[wsol_reserve]).await;

    lending_market
        .borrow_obligation_liquidity(
            &mut test,
            wsol_reserve,
            &obligations[0],
            &users[0],
            None,
            u64::MAX,
        )
        .await
        .unwrap();

    // check token balances
    let (balance_changes, _mint_supply_changes) =
        balance_checker.find_balance_changes(&mut test).await;

    let expected_balance_changes = HashSet::from([TokenBalanceChange {
        token_account: wsol_reserve.account.liquidity.supply_pubkey,
        mint: wsol_mint::id(),
        diff: -(LAMPORTS_PER_SOL as i128 / 2),
    }]);

    assert_eq!(balance_changes, expected_balance_changes);
}

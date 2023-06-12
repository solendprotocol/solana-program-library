#![cfg(feature = "test-bpf")]

use solana_program::native_token::LAMPORTS_PER_SOL;
use solana_program::pubkey::Pubkey;
use solana_sdk::signature::Signer;
use solend_program::math::TryDiv;
mod helpers;

use crate::solend_program_test::*;
use solend_sdk::math::Decimal;
use solend_sdk::state::ObligationCollateral;
use solend_sdk::state::ReserveCollateral;
use solend_sdk::state::*;
use std::collections::HashSet;

use crate::solend_program_test::scenario_1;
use crate::solend_program_test::BalanceChecker;
use crate::solend_program_test::TokenBalanceChange;
use helpers::*;

use solana_program_test::*;

use solend_sdk::state::LastUpdate;
use solend_sdk::state::Obligation;

use solend_sdk::state::Reserve;
use solend_sdk::state::ReserveLiquidity;

#[tokio::test]
async fn test_success() {
    let (mut test, lending_market, usdc_reserve, wsol_reserve, user, obligation) =
        scenario_1(&test_reserve_config(), &test_reserve_config()).await;

    let balance_checker =
        BalanceChecker::start(&mut test, &[&usdc_reserve, &user, &wsol_reserve]).await;

    lending_market
        .withdraw_obligation_collateral_and_redeem_reserve_collateral(
            &mut test,
            &usdc_reserve,
            &obligation,
            &user,
            u64::MAX,
        )
        .await
        .unwrap();

    // check token balances
    let (balance_changes, mint_supply_changes) =
        balance_checker.find_balance_changes(&mut test).await;
    // still borrowing 100usd worth of sol so we need to leave 200usd in the obligation.
    let withdraw_amount = (100_000 * FRACTIONAL_TO_USDC - 200 * FRACTIONAL_TO_USDC) as i128;

    let expected_balance_changes = HashSet::from([
        TokenBalanceChange {
            token_account: user.get_account(&usdc_mint::id()).unwrap(),
            mint: usdc_mint::id(),
            diff: withdraw_amount,
        },
        TokenBalanceChange {
            token_account: usdc_reserve.account.liquidity.supply_pubkey,
            mint: usdc_mint::id(),
            diff: -withdraw_amount,
        },
        TokenBalanceChange {
            token_account: usdc_reserve.account.collateral.supply_pubkey,
            mint: usdc_reserve.account.collateral.mint_pubkey,
            diff: -withdraw_amount,
        },
    ]);
    assert_eq!(balance_changes, expected_balance_changes);
    assert_eq!(
        mint_supply_changes,
        HashSet::from([MintSupplyChange {
            mint: usdc_reserve.account.collateral.mint_pubkey,
            diff: -withdraw_amount
        }])
    );

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
                        Decimal::from(withdraw_amount as u64)
                            .try_div(Decimal::from(1_000_000u64))
                            .unwrap(),
                    )
                    .unwrap();
                rate_limiter
            },
            ..lending_market.account
        }
    );

    let usdc_reserve_post = test.load_account::<Reserve>(usdc_reserve.pubkey).await;
    assert_eq!(
        usdc_reserve_post.account,
        Reserve {
            last_update: LastUpdate {
                slot: 1000,
                stale: true
            },
            liquidity: ReserveLiquidity {
                available_amount: usdc_reserve.account.liquidity.available_amount
                    - withdraw_amount as u64,
                ..usdc_reserve.account.liquidity
            },
            collateral: ReserveCollateral {
                mint_total_supply: usdc_reserve.account.collateral.mint_total_supply
                    - withdraw_amount as u64,
                ..usdc_reserve.account.collateral
            },
            rate_limiter: {
                let mut rate_limiter = usdc_reserve.account.rate_limiter;
                rate_limiter
                    .update(1000, Decimal::from(withdraw_amount as u64))
                    .unwrap();

                rate_limiter
            },
            ..usdc_reserve.account
        }
    );

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
                deposited_amount: 200 * FRACTIONAL_TO_USDC,
                ..obligation.account.deposits[0]
            }]
            .to_vec(),
            ..obligation.account
        }
    );
}

#[tokio::test]
async fn test_withdraw_max_rate_limiter() {
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
                deposits: vec![(wsol_mint::id(), 50 * LAMPORTS_PER_SOL)],
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
                max_outflow: 20 * LAMPORTS_PER_SOL,
            },
            None,
        )
        .await
        .unwrap();

    test.advance_clock_by_slots(1).await;

    let balance_checker = BalanceChecker::start(&mut test, &[&users[0]]).await;

    lending_market
        .withdraw_obligation_collateral_and_redeem_reserve_collateral(
            &mut test,
            wsol_reserve,
            &obligations[0],
            &users[0],
            u64::MAX,
        )
        .await
        .unwrap();

    // check token balances
    let (balance_changes, _mint_supply_changes) =
        balance_checker.find_balance_changes(&mut test).await;

    let expected_balance_changes = HashSet::from([TokenBalanceChange {
        token_account: users[0].get_account(&wsol_mint::id()).unwrap(),
        mint: wsol_mint::id(),
        diff: 20 * LAMPORTS_PER_SOL as i128,
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
                max_outflow: 50, // $50
            },
            Pubkey::new_unique(),
        )
        .await
        .unwrap();

    let balance_checker = BalanceChecker::start(&mut test, &[&users[0]]).await;

    lending_market
        .withdraw_obligation_collateral_and_redeem_reserve_collateral(
            &mut test,
            wsol_reserve,
            &obligations[0],
            &users[0],
            u64::MAX,
        )
        .await
        .unwrap();

    // // check token balances
    let (balance_changes, _mint_supply_changes) =
        balance_checker.find_balance_changes(&mut test).await;

    let expected_balance_changes = HashSet::from([TokenBalanceChange {
        token_account: users[0].get_account(&wsol_mint::id()).unwrap(),
        mint: wsol_mint::id(),
        diff: 5 * LAMPORTS_PER_SOL as i128,
    }]);

    assert_eq!(balance_changes, expected_balance_changes);
}

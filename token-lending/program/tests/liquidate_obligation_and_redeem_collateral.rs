#![cfg(feature = "test-bpf")]

use crate::solend_program_test::custom_scenario;
use crate::solend_program_test::MintSupplyChange;
use crate::solend_program_test::ObligationArgs;
use crate::solend_program_test::ReserveArgs;
use solana_sdk::instruction::InstructionError;
use solana_sdk::native_token::LAMPORTS_PER_SOL;
use solana_sdk::signer::Signer;
use solana_sdk::transaction::TransactionError;
use solend_program::error::LendingError;
use solend_program::math::TrySub;
use solend_program::state::LastUpdate;
use solend_program::state::ObligationCollateral;
use solend_program::state::ObligationLiquidity;
use solend_program::state::ReserveConfig;
use solend_program::state::ReserveFees;
use solend_sdk::NULL_PUBKEY;
mod helpers;

use crate::solend_program_test::scenario_1;
use crate::solend_program_test::BalanceChecker;
use crate::solend_program_test::PriceArgs;
use crate::solend_program_test::TokenBalanceChange;
use crate::solend_program_test::User;
use helpers::*;
use solana_program_test::*;
use solana_sdk::signature::Keypair;
use solend_program::math::Decimal;
use solend_program::state::LendingMarket;
use solend_program::state::Obligation;
use solend_program::state::Reserve;
use solend_program::state::ReserveCollateral;
use solend_program::state::ReserveLiquidity;
use solend_program::state::LIQUIDATION_CLOSE_FACTOR;

use std::collections::HashSet;

#[tokio::test]
async fn test_success_new() {
    let (mut test, lending_market, usdc_reserve, wsol_reserve, user, obligation, _) = scenario_1(
        &ReserveConfig {
            optimal_borrow_rate: 0,
            max_borrow_rate: 0,
            fees: ReserveFees::default(),
            ..test_reserve_config()
        },
        &test_reserve_config(),
    )
    .await;

    let liquidator = User::new_with_balances(
        &mut test,
        &[
            (&wsol_mint::id(), 100 * LAMPORTS_TO_SOL),
            (&usdc_reserve.account.collateral.mint_pubkey, 0),
            (&usdc_mint::id(), 0),
        ],
    )
    .await;

    let balance_checker = BalanceChecker::start(
        &mut test,
        &[
            &usdc_reserve,
            &user,
            &wsol_reserve,
            &usdc_reserve,
            &liquidator,
        ],
    )
    .await;

    // close LTV is 0.55, we've deposited 100k USDC and borrowed 10 SOL.
    // obligation gets liquidated if 100k * 0.55 = 10 SOL * sol_price => sol_price = 5.5k
    test.set_price(
        &wsol_mint::id(),
        &PriceArgs {
            price: 5500,
            conf: 0,
            expo: 0,
            ema_price: 5500,
            ema_conf: 0,
        },
    )
    .await;

    lending_market
        .liquidate_obligation_and_redeem_reserve_collateral(
            &mut test,
            &wsol_reserve,
            &usdc_reserve,
            &obligation,
            &liquidator,
            u64::MAX,
        )
        .await
        .unwrap();

    let (balance_changes, mint_supply_changes) =
        balance_checker.find_balance_changes(&mut test).await;

    // 55k * 0.2 => 11k worth of SOL gets repaid
    // => 11k worth of USDC gets withdrawn + bonus.
    // bonus is 5%:
    // - 1% protocol liquidation fee: 110
    // - 4% liquidator bonus: 440
    let bonus = (usdc_reserve.account.config.liquidation_bonus
        + usdc_reserve.account.config.protocol_liquidation_fee / 10) as u64;

    let expected_borrow_repaid = 10 * (LIQUIDATION_CLOSE_FACTOR as u64) / 100;
    let expected_usdc_withdrawn = expected_borrow_repaid * 5500 * (100 + bonus) / 100;

    let expected_protocol_liquidation_fee = 110;

    let expected_balance_changes = HashSet::from([
        // liquidator
        TokenBalanceChange {
            token_account: liquidator.get_account(&usdc_mint::id()).unwrap(),
            mint: usdc_mint::id(),
            diff: ((expected_usdc_withdrawn - expected_protocol_liquidation_fee)
                * FRACTIONAL_TO_USDC) as i128,
        },
        TokenBalanceChange {
            token_account: liquidator.get_account(&wsol_mint::id()).unwrap(),
            mint: wsol_mint::id(),
            diff: -((expected_borrow_repaid * LAMPORTS_TO_SOL) as i128),
        },
        // usdc reserve
        TokenBalanceChange {
            token_account: usdc_reserve.account.collateral.supply_pubkey,
            mint: usdc_reserve.account.collateral.mint_pubkey,
            diff: -((expected_usdc_withdrawn * FRACTIONAL_TO_USDC) as i128),
        },
        TokenBalanceChange {
            token_account: usdc_reserve.account.liquidity.supply_pubkey,
            mint: usdc_mint::id(),
            diff: -((expected_usdc_withdrawn * FRACTIONAL_TO_USDC) as i128),
        },
        TokenBalanceChange {
            token_account: usdc_reserve.account.config.fee_receiver,
            mint: usdc_mint::id(),
            diff: (expected_protocol_liquidation_fee * FRACTIONAL_TO_USDC) as i128,
        },
        // wsol reserve
        TokenBalanceChange {
            token_account: wsol_reserve.account.liquidity.supply_pubkey,
            mint: wsol_mint::id(),
            diff: (expected_borrow_repaid * LAMPORTS_TO_SOL) as i128,
        },
    ]);
    assert_eq!(balance_changes, expected_balance_changes);
    assert_eq!(
        mint_supply_changes,
        HashSet::from([MintSupplyChange {
            mint: usdc_reserve.account.collateral.mint_pubkey,
            diff: -((expected_usdc_withdrawn * FRACTIONAL_TO_USDC) as i128)
        }])
    );

    // check program state
    let lending_market_post = test
        .load_account::<LendingMarket>(lending_market.pubkey)
        .await;
    assert_eq!(lending_market_post.account, lending_market.account);

    let usdc_reserve_post = test.load_account::<Reserve>(usdc_reserve.pubkey).await;
    assert_eq!(
        usdc_reserve_post.account,
        Reserve {
            liquidity: ReserveLiquidity {
                available_amount: usdc_reserve.account.liquidity.available_amount
                    - expected_usdc_withdrawn * FRACTIONAL_TO_USDC,
                ..usdc_reserve.account.liquidity
            },
            collateral: ReserveCollateral {
                mint_total_supply: usdc_reserve.account.collateral.mint_total_supply
                    - expected_usdc_withdrawn * FRACTIONAL_TO_USDC,
                ..usdc_reserve.account.collateral
            },
            attributed_borrow_value: Decimal::from(55000u64),
            ..usdc_reserve.account
        }
    );

    let wsol_reserve_post = test.load_account::<Reserve>(wsol_reserve.pubkey).await;
    assert_eq!(
        wsol_reserve_post.account,
        Reserve {
            liquidity: ReserveLiquidity {
                available_amount: wsol_reserve.account.liquidity.available_amount
                    + expected_borrow_repaid * LAMPORTS_TO_SOL,
                borrowed_amount_wads: wsol_reserve
                    .account
                    .liquidity
                    .borrowed_amount_wads
                    .try_sub(Decimal::from(expected_borrow_repaid * LAMPORTS_TO_SOL))
                    .unwrap(),
                market_price: Decimal::from(5500u64),
                smoothed_market_price: Decimal::from(5500u64),
                ..wsol_reserve.account.liquidity
            },
            ..wsol_reserve.account
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
                deposited_amount: (100_000 - expected_usdc_withdrawn) * FRACTIONAL_TO_USDC,
                market_value: Decimal::from(100_000u64), // old value
                attributed_borrow_value: obligation_post.account.deposits[0]
                    .attributed_borrow_value, // don't care about verifying this here
            }]
            .to_vec(),
            borrows: [ObligationLiquidity {
                borrow_reserve: wsol_reserve.pubkey,
                cumulative_borrow_rate_wads: Decimal::one(),
                borrowed_amount_wads: Decimal::from(10 * LAMPORTS_TO_SOL)
                    .try_sub(Decimal::from(expected_borrow_repaid * LAMPORTS_TO_SOL))
                    .unwrap(),
                market_value: Decimal::from(55_000u64),
            }]
            .to_vec(),
            deposited_value: Decimal::from(100_000u64),
            borrowed_value: Decimal::from(55_000u64),
            borrowed_value_upper_bound: Decimal::from(55_000u64),
            allowed_borrow_value: Decimal::from(50_000u64),
            unhealthy_borrow_value: Decimal::from(55_000u64),
            ..obligation.account
        }
    );
}

#[tokio::test]
async fn test_whitelisting_liquidator() {
    let (
        mut test,
        lending_market,
        usdc_reserve,
        wsol_reserve,
        _user,
        obligation,
        lending_market_owner,
    ) = scenario_1(
        &ReserveConfig {
            protocol_liquidation_fee: 2,
            ..test_reserve_config()
        },
        &test_reserve_config(),
    )
    .await;

    let whitelisted_liquidator = User::new_with_balances(
        &mut test,
        &[
            (&wsol_mint::id(), 100 * LAMPORTS_TO_SOL),
            (&usdc_reserve.account.collateral.mint_pubkey, 0),
            (&usdc_mint::id(), 0),
        ],
    )
    .await;

    let rando_liquidator = User::new_with_balances(
        &mut test,
        &[
            (&wsol_mint::id(), 100 * LAMPORTS_TO_SOL),
            (&usdc_reserve.account.collateral.mint_pubkey, 0),
            (&usdc_mint::id(), 0),
        ],
    )
    .await;

    lending_market
        .set_lending_market_owner_and_config(
            &mut test,
            &lending_market_owner,
            &lending_market_owner.keypair.pubkey(),
            lending_market.account.rate_limiter.config,
            Some(whitelisted_liquidator.keypair.pubkey()),
            NULL_PUBKEY,
        )
        .await
        .unwrap();

    // close LTV is 0.55, we've deposited 100k USDC and borrowed 10 SOL.
    // obligation gets liquidated if 100k * 0.55 = 10 SOL * sol_price => sol_price = 5.5k
    test.set_price(
        &wsol_mint::id(),
        &PriceArgs {
            price: 5500,
            conf: 0,
            expo: 0,
            ema_price: 5500,
            ema_conf: 0,
        },
    )
    .await;

    let err = lending_market
        .liquidate_obligation_and_redeem_reserve_collateral(
            &mut test,
            &wsol_reserve,
            &usdc_reserve,
            &obligation,
            &rando_liquidator,
            u64::MAX,
        )
        .await
        .unwrap_err()
        .unwrap();

    assert_eq!(
        err,
        TransactionError::InstructionError(
            1,
            InstructionError::Custom(LendingError::NotWhitelistedLiquidator as u32)
        )
    );

    lending_market
        .liquidate_obligation_and_redeem_reserve_collateral(
            &mut test,
            &wsol_reserve,
            &usdc_reserve,
            &obligation,
            &whitelisted_liquidator,
            u64::MAX,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn test_success_insufficient_liquidity() {
    let (mut test, lending_market, usdc_reserve, wsol_reserve, user, obligation, _) = scenario_1(
        &ReserveConfig {
            optimal_borrow_rate: 0,
            max_borrow_rate: 0,
            fees: ReserveFees::default(),
            ..test_reserve_config()
        },
        &test_reserve_config(),
    )
    .await;

    // basically the same test as above, but now someone borrows a lot of USDC so the liquidatior
    // partially receives USDC and cUSDC
    {
        let usdc_borrower = User::new_with_balances(
            &mut test,
            &[
                (&usdc_mint::id(), 0),
                (&wsol_mint::id(), 20_000 * LAMPORTS_TO_SOL),
                (&wsol_reserve.account.collateral.mint_pubkey, 0),
            ],
        )
        .await;

        let obligation = lending_market
            .init_obligation(&mut test, Keypair::new(), &usdc_borrower)
            .await
            .unwrap();

        lending_market
            .deposit_reserve_liquidity_and_obligation_collateral(
                &mut test,
                &wsol_reserve,
                &obligation,
                &usdc_borrower,
                20_000 * LAMPORTS_TO_SOL,
            )
            .await
            .unwrap();

        let obligation = test.load_account::<Obligation>(obligation.pubkey).await;
        lending_market
            .borrow_obligation_liquidity(
                &mut test,
                &usdc_reserve,
                &obligation,
                &usdc_borrower,
                usdc_borrower.get_account(&usdc_mint::id()),
                u64::MAX,
            )
            .await
            .unwrap()
    }

    let liquidator = User::new_with_balances(
        &mut test,
        &[
            (&wsol_mint::id(), 100 * LAMPORTS_TO_SOL),
            (&usdc_reserve.account.collateral.mint_pubkey, 0),
            (&usdc_mint::id(), 0),
        ],
    )
    .await;

    let balance_checker = BalanceChecker::start(
        &mut test,
        &[&usdc_reserve, &user, &wsol_reserve, &liquidator],
    )
    .await;

    // close LTV is 0.55, we've deposited 100k USDC and borrowed 10 SOL.
    // obligation gets liquidated if 100k * 0.55 = 10 SOL * sol_price => sol_price == 5.5k
    test.set_price(
        &wsol_mint::id(),
        &PriceArgs {
            price: 5500,
            conf: 0,
            expo: 0,
            ema_price: 5500,
            ema_conf: 0,
        },
    )
    .await;

    let lending_market = test
        .load_account::<LendingMarket>(lending_market.pubkey)
        .await;
    let usdc_reserve = test.load_account::<Reserve>(usdc_reserve.pubkey).await;
    let wsol_reserve = test.load_account::<Reserve>(wsol_reserve.pubkey).await;

    let available_amount = usdc_reserve.account.liquidity.available_amount / FRACTIONAL_TO_USDC;

    lending_market
        .liquidate_obligation_and_redeem_reserve_collateral(
            &mut test,
            &wsol_reserve,
            &usdc_reserve,
            &obligation,
            &liquidator,
            u64::MAX,
        )
        .await
        .unwrap();

    let (balance_changes, mint_supply_changes) =
        balance_checker.find_balance_changes(&mut test).await;

    let bonus = (usdc_reserve.account.config.liquidation_bonus
        + usdc_reserve.account.config.protocol_liquidation_fee / 10) as u64;

    let expected_borrow_repaid = 10 * (LIQUIDATION_CLOSE_FACTOR as u64) / 100;
    let expected_cusdc_withdrawn =
        expected_borrow_repaid * 5500 * (100 + bonus) / 100 - available_amount;
    let expected_protocol_liquidation_fee = usdc_reserve
        .account
        .calculate_protocol_liquidation_fee(
            available_amount * FRACTIONAL_TO_USDC,
            Decimal::from_percent(105),
        )
        .unwrap();

    let expected_balance_changes = HashSet::from([
        // liquidator
        TokenBalanceChange {
            token_account: liquidator.get_account(&usdc_mint::id()).unwrap(),
            mint: usdc_mint::id(),
            diff: (available_amount * FRACTIONAL_TO_USDC - expected_protocol_liquidation_fee)
                as i128,
        },
        TokenBalanceChange {
            token_account: liquidator
                .get_account(&usdc_reserve.account.collateral.mint_pubkey)
                .unwrap(),
            mint: usdc_reserve.account.collateral.mint_pubkey,
            diff: (expected_cusdc_withdrawn * FRACTIONAL_TO_USDC) as i128,
        },
        TokenBalanceChange {
            token_account: liquidator.get_account(&wsol_mint::id()).unwrap(),
            mint: wsol_mint::id(),
            diff: -((expected_borrow_repaid * LAMPORTS_TO_SOL) as i128),
        },
        // usdc reserve
        TokenBalanceChange {
            token_account: usdc_reserve.account.collateral.supply_pubkey,
            mint: usdc_reserve.account.collateral.mint_pubkey,
            diff: -(((expected_cusdc_withdrawn + available_amount) * FRACTIONAL_TO_USDC) as i128),
        },
        TokenBalanceChange {
            token_account: usdc_reserve.account.liquidity.supply_pubkey,
            mint: usdc_mint::id(),
            diff: -((available_amount * FRACTIONAL_TO_USDC) as i128),
        },
        TokenBalanceChange {
            token_account: usdc_reserve.account.config.fee_receiver,
            mint: usdc_mint::id(),
            diff: expected_protocol_liquidation_fee as i128,
        },
        // wsol reserve
        TokenBalanceChange {
            token_account: wsol_reserve.account.liquidity.supply_pubkey,
            mint: wsol_mint::id(),
            diff: (expected_borrow_repaid * LAMPORTS_TO_SOL) as i128,
        },
    ]);
    assert_eq!(
        balance_changes, expected_balance_changes,
        "{:#?} {:#?}",
        balance_changes, expected_balance_changes
    );

    assert_eq!(
        mint_supply_changes,
        HashSet::from([MintSupplyChange {
            mint: usdc_reserve.account.collateral.mint_pubkey,
            diff: -((available_amount * FRACTIONAL_TO_USDC) as i128)
        }])
    );
}

#[tokio::test]
async fn test_liquidity_ordering() {
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

    let usdc_reserve = reserves
        .iter()
        .find(|r| r.account.liquidity.mint_pubkey == usdc_mint::id())
        .unwrap();

    let wsol_reserve = reserves
        .iter()
        .find(|r| r.account.liquidity.mint_pubkey == wsol_mint::id())
        .unwrap();

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

    // update usdc borrow weight to 20_000
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

    let liquidator = User::new_with_balances(
        &mut test,
        &[
            (&wsol_mint::id(), 100 * LAMPORTS_TO_SOL),
            (&usdc_reserve.account.collateral.mint_pubkey, 0),
            (&usdc_mint::id(), 100 * LAMPORTS_TO_SOL),
        ],
    )
    .await;

    // this fails because wsol isn't the first borrow on the obligation
    let err = lending_market
        .liquidate_obligation_and_redeem_reserve_collateral(
            &mut test,
            wsol_reserve,
            usdc_reserve,
            &obligations[0],
            &liquidator,
            u64::MAX,
        )
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

    // this should pass though
    lending_market
        .liquidate_obligation_and_redeem_reserve_collateral(
            &mut test,
            usdc_reserve,
            usdc_reserve,
            &obligations[0],
            &liquidator,
            u64::MAX,
        )
        .await
        .unwrap();
}

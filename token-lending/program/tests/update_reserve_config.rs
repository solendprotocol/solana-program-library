#![cfg(feature = "test-bpf")]

use crate::solend_program_test::SwitchboardPriceArgs;
use crate::solend_program_test::custom_scenario;
use crate::solend_program_test::Oracle;
use crate::solend_program_test::PriceArgs;
use crate::solend_program_test::ReserveArgs;
use solana_sdk::instruction::AccountMeta;
use solana_sdk::instruction::Instruction;
use solana_sdk::pubkey::PUBKEY_BYTES;
use solend_sdk::instruction::LendingInstruction;
mod helpers;

use crate::solend_program_test::setup_world;
use crate::solend_program_test::Info;
use crate::solend_program_test::SolendProgramTest;
use crate::solend_program_test::User;
use helpers::*;
use solana_program::example_mocks::solana_sdk::Pubkey;
use solana_program_test::*;
use solana_sdk::{
    instruction::InstructionError,
    signature::{Keypair, Signer},
    transaction::TransactionError,
};
use solend_program::state::RateLimiter;
use solend_program::state::RateLimiterConfig;
use solend_program::state::Reserve;
use solend_program::NULL_PUBKEY;

use solend_program::{error::LendingError, state::ReserveConfig};
use solend_sdk::state::LendingMarket;

async fn setup() -> (SolendProgramTest, Info<LendingMarket>, User) {
    let (test, lending_market, _, _, lending_market_owner, _) =
        setup_world(&test_reserve_config(), &test_reserve_config()).await;

    (test, lending_market, lending_market_owner)
}

#[tokio::test]
async fn test_update_reserve_config_owner() {
    let (mut test, lending_market, lending_market_owner) = setup().await;

    let wsol_reserve = test
        .init_reserve(
            &lending_market,
            &lending_market_owner,
            &wsol_mint::id(),
            &test_reserve_config(),
            &Keypair::new(),
            1000,
            None,
        )
        .await
        .unwrap();

    let new_reserve_config = ReserveConfig {
        fee_receiver: wsol_reserve.account.config.fee_receiver,
        ..test_reserve_config()
    };
    let new_rate_limiter_config = RateLimiterConfig {
        window_duration: 50,
        max_outflow: 100,
    };

    lending_market
        .update_reserve_config(
            &mut test,
            &lending_market_owner,
            &wsol_reserve,
            new_reserve_config,
            new_rate_limiter_config,
            None,
        )
        .await
        .unwrap();

    let wsol_reserve_post = test.load_account::<Reserve>(wsol_reserve.pubkey).await;
    assert_eq!(
        wsol_reserve_post.account,
        Reserve {
            config: new_reserve_config,
            rate_limiter: RateLimiter::new(new_rate_limiter_config, 1000),
            ..wsol_reserve.account
        }
    );
}

#[tokio::test]
async fn test_update_reserve_config_risk_authority() {
    let (mut test, lending_market, lending_market_owner) = setup().await;

    let wsol_reserve = test
        .init_reserve(
            &lending_market,
            &lending_market_owner,
            &wsol_mint::id(),
            &ReserveConfig {
                deposit_limit: 10000,
                ..test_reserve_config()
            },
            &Keypair::new(),
            1000,
            None,
        )
        .await
        .unwrap();

    let risk_authority = User::new_with_keypair(Keypair::new());
    lending_market
        .set_lending_market_owner_and_config(
            &mut test,
            &lending_market_owner,
            &lending_market_owner.keypair.pubkey(),
            lending_market.account.rate_limiter.config,
            lending_market.account.whitelisted_liquidator,
            risk_authority.keypair.pubkey(),
        )
        .await
        .unwrap();

    let new_reserve_config = ReserveConfig {
        borrow_limit: 20, // this should get updated on the reserve (safer than previous
        // value)
        deposit_limit: 10001, // this should NOT get updated on the reserve (prev limit was
        // safer)
        liquidation_threshold: 60, // this should NOT get updated (risk authority can't change
        // this)
        ..wsol_reserve.account.config
    };

    let new_rate_limiter_config = RateLimiterConfig {
        window_duration: 50,
        max_outflow: 0,
    };

    lending_market
        .update_reserve_config(
            &mut test,
            &risk_authority,
            &wsol_reserve,
            new_reserve_config,
            new_rate_limiter_config,
            None,
        )
        .await
        .unwrap();

    let wsol_reserve_post = test.load_account::<Reserve>(wsol_reserve.pubkey).await;
    assert_eq!(
        wsol_reserve_post.account,
        Reserve {
            config: ReserveConfig {
                borrow_limit: 20,
                ..wsol_reserve.account.config
            },
            rate_limiter: RateLimiter::new(new_rate_limiter_config, 1000),
            ..wsol_reserve.account
        }
    );
}

#[tokio::test]
async fn test_update_invalid_oracle_config() {
    let (mut test, lending_market, lending_market_owner) = setup().await;
    let wsol_reserve = test
        .init_reserve(
            &lending_market,
            &lending_market_owner,
            &wsol_mint::id(),
            &test_reserve_config(),
            &Keypair::new(),
            1000,
            None,
        )
        .await
        .unwrap();

    let oracle = test.mints.get(&wsol_mint::id()).unwrap().unwrap();

    let new_reserve_config = ReserveConfig {
        fee_receiver: wsol_reserve.account.config.fee_receiver,
        ..test_reserve_config()
    };
    let new_rate_limiter_config = RateLimiterConfig {
        window_duration: 50,
        max_outflow: 100,
    };

    let switchboard_pubkey = test.init_switchboard_feed(&wsol_mint::id()).await;

    // Try setting both of the oracles to null: Should fail
    let res = lending_market
        .update_reserve_config(
            &mut test,
            &lending_market_owner,
            &wsol_reserve,
            new_reserve_config,
            new_rate_limiter_config,
            Some(&Oracle {
                pyth_product_pubkey: oracle.pyth_product_pubkey,
                pyth_price_pubkey: NULL_PUBKEY,
                switchboard_feed_pubkey: Some(NULL_PUBKEY),
            }),
        )
        .await
        .unwrap_err()
        .unwrap();

    assert_eq!(
        res,
        TransactionError::InstructionError(
            1,
            InstructionError::Custom(LendingError::InvalidOracleConfig as u32)
        )
    );

    // this should be fine
    lending_market
        .update_reserve_config(
            &mut test,
            &lending_market_owner,
            &wsol_reserve,
            new_reserve_config,
            new_rate_limiter_config,
            Some(&Oracle {
                pyth_product_pubkey: oracle.pyth_product_pubkey,
                pyth_price_pubkey: oracle.pyth_price_pubkey,
                switchboard_feed_pubkey: Some(NULL_PUBKEY),
            }),
        )
        .await
        .unwrap();

    test.advance_clock_by_slots(1).await;

    lending_market
        .update_reserve_config(
            &mut test,
            &lending_market_owner,
            &wsol_reserve,
            new_reserve_config,
            new_rate_limiter_config,
            Some(&Oracle {
                pyth_product_pubkey: NULL_PUBKEY,
                pyth_price_pubkey: NULL_PUBKEY,
                switchboard_feed_pubkey: Some(switchboard_pubkey),
            }),
        )
        .await
        .unwrap();

    test.advance_clock_by_slots(1).await;

    // Try setting both of the oracles to null: Should fail
    let res = lending_market
        .update_reserve_config(
            &mut test,
            &lending_market_owner,
            &wsol_reserve,
            new_reserve_config,
            new_rate_limiter_config,
            Some(&Oracle {
                pyth_product_pubkey: oracle.pyth_product_pubkey,
                pyth_price_pubkey: NULL_PUBKEY,
                switchboard_feed_pubkey: Some(NULL_PUBKEY),
            }),
        )
        .await
        .unwrap_err()
        .unwrap();

    assert_eq!(
        res,
        TransactionError::InstructionError(
            1,
            InstructionError::Custom(LendingError::InvalidOracleConfig as u32)
        )
    );
}

#[tokio::test]
async fn test_update_reserve_config_invalid_signers() {
    let (mut test, lending_market, lending_market_owner) = setup().await;

    let wsol_reserve = test
        .init_reserve(
            &lending_market,
            &lending_market_owner,
            &wsol_mint::id(),
            &test_reserve_config(),
            &Keypair::new(),
            1000,
            None,
        )
        .await
        .unwrap();

    let risk_authority = User::new_with_keypair(Keypair::new());
    let rando = User::new_with_keypair(Keypair::new());

    lending_market
        .set_lending_market_owner_and_config(
            &mut test,
            &lending_market_owner,
            &lending_market_owner.keypair.pubkey(),
            lending_market.account.rate_limiter.config,
            lending_market.account.whitelisted_liquidator,
            risk_authority.keypair.pubkey(),
        )
        .await
        .unwrap();

    let new_reserve_config = ReserveConfig {
        borrow_limit: 20,
        liquidation_threshold: 60,
        ..wsol_reserve.account.config
    };

    let new_rate_limiter_config = RateLimiterConfig {
        window_duration: 50,
        max_outflow: 0,
    };

    // case 1: try to update with a random user
    let res = lending_market
        .update_reserve_config(
            &mut test,
            &rando,
            &wsol_reserve,
            new_reserve_config,
            new_rate_limiter_config,
            None,
        )
        .await
        .unwrap_err()
        .unwrap();

    assert_eq!(
        res,
        TransactionError::InstructionError(
            1,
            InstructionError::Custom(LendingError::InvalidSigner as u32)
        )
    );

    // case 2: try to update without signing
    let err = test
        .process_transaction(
            &[malicious_update_reserve_config(
                solend_program::id(),
                new_reserve_config,
                new_rate_limiter_config,
                wsol_reserve.pubkey,
                lending_market.pubkey,
                lending_market_owner.keypair.pubkey(),
                test.mints
                    .get(&wsol_mint::id())
                    .unwrap()
                    .unwrap()
                    .pyth_product_pubkey,
                wsol_reserve.account.liquidity.pyth_oracle_pubkey,
                wsol_reserve.account.liquidity.switchboard_oracle_pubkey,
            )],
            None,
        )
        .await
        .unwrap_err()
        .unwrap();

    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(LendingError::InvalidSigner as u32)
        )
    );
}

/// Creates an Malicious 'UpdateReserveConfig' instruction (no signer needed)
#[allow(clippy::too_many_arguments)]
pub fn malicious_update_reserve_config(
    program_id: Pubkey,
    config: ReserveConfig,
    rate_limiter_config: RateLimiterConfig,
    reserve_pubkey: Pubkey,
    lending_market_pubkey: Pubkey,
    lending_market_owner_pubkey: Pubkey,
    pyth_product_pubkey: Pubkey,
    pyth_price_pubkey: Pubkey,
    switchboard_feed_pubkey: Pubkey,
) -> Instruction {
    let (lending_market_authority_pubkey, _bump_seed) = Pubkey::find_program_address(
        &[&lending_market_pubkey.to_bytes()[..PUBKEY_BYTES]],
        &program_id,
    );
    let accounts = vec![
        AccountMeta::new(reserve_pubkey, false),
        AccountMeta::new_readonly(lending_market_pubkey, false),
        AccountMeta::new_readonly(lending_market_authority_pubkey, false),
        AccountMeta::new_readonly(lending_market_owner_pubkey, false),
        AccountMeta::new_readonly(pyth_product_pubkey, false),
        AccountMeta::new_readonly(pyth_price_pubkey, false),
        AccountMeta::new_readonly(switchboard_feed_pubkey, false),
    ];
    Instruction {
        program_id,
        accounts,
        data: LendingInstruction::UpdateReserveConfig {
            config,
            rate_limiter_config,
        }
        .pack(),
    }
}

#[tokio::test]
async fn test_add_extra_oracle() {
    let (mut test, lending_market, reserves, obligations, users, lending_market_owner) =
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
                ..msol_reserve.account.config
            },
            msol_reserve.account.rate_limiter.config,
            None,
        )
        .await
        .unwrap();

    let msol_reserve_post = test.load_account::<Reserve>(msol_reserve.pubkey).await;
    assert_eq!(
        msol_reserve_post.account.config.extra_oracle_pubkey,
        Some(wsol_pyth_feed)
    );

    lending_market
        .update_reserve_config(
            &mut test,
            &lending_market_owner,
            msol_reserve,
            ReserveConfig {
                extra_oracle_pubkey: None,
                ..msol_reserve.account.config
            },
            msol_reserve.account.rate_limiter.config,
            None,
        )
        .await
        .unwrap();

    let msol_reserve_post = test.load_account::<Reserve>(msol_reserve.pubkey).await;
    assert_eq!(
        msol_reserve_post.account.config.extra_oracle_pubkey,
        None
    );

    let wsol_switchboard_feed = test.init_switchboard_feed(&wsol_mint::id()).await;
    test.set_switchboard_price(
        &wsol_mint::id(),
        SwitchboardPriceArgs {
            price: 5,
            expo: 0,
        },
    )
    .await;

    lending_market
        .update_reserve_config(
            &mut test,
            &lending_market_owner,
            msol_reserve,
            ReserveConfig {
                extra_oracle_pubkey: Some(wsol_switchboard_feed),
                ..msol_reserve.account.config
            },
            msol_reserve.account.rate_limiter.config,
            None,
        )
        .await
        .unwrap();

    let msol_reserve_post = test.load_account::<Reserve>(msol_reserve.pubkey).await;
    assert_eq!(
        msol_reserve_post.account.config.extra_oracle_pubkey,
        Some(wsol_switchboard_feed)
    );
}

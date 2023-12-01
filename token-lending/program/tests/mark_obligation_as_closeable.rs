#![cfg(feature = "test-bpf")]

use crate::solend_program_test::custom_scenario;
use solana_program::{
    clock::Slot,
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

use crate::solend_program_test::User;

use solana_sdk::signer::keypair::Keypair;
use solana_sdk::signer::Signer;

use crate::solend_program_test::ObligationArgs;
use crate::solend_program_test::PriceArgs;
use crate::solend_program_test::ReserveArgs;

use solana_program::native_token::LAMPORTS_PER_SOL;
use solana_sdk::instruction::InstructionError;
use solana_sdk::transaction::TransactionError;
use solend_program::error::LendingError;

use solend_program::state::ReserveConfig;

use solend_sdk::{instruction::LendingInstruction, solend_mainnet, state::*};
mod helpers;

use helpers::*;
use solana_program_test::*;

#[tokio::test]
async fn test_mark_obligation_as_closeable_success() {
    let (mut test, lending_market, reserves, obligations, _users, lending_market_owner) =
        custom_scenario(
            &[
                ReserveArgs {
                    mint: usdc_mint::id(),
                    config: reserve_config_no_fees(),
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
                    config: reserve_config_no_fees(),
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
            &[ObligationArgs {
                deposits: vec![(usdc_mint::id(), 20 * FRACTIONAL_TO_USDC)],
                borrows: vec![(wsol_mint::id(), LAMPORTS_PER_SOL)],
            }],
        )
        .await;

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

    test.advance_clock_by_slots(1).await;

    let err = lending_market
        .mark_obligation_as_closable(
            &mut test,
            &obligations[0],
            &reserves[0],
            &risk_authority,
            10000,
        )
        .await
        .unwrap_err()
        .unwrap();

    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(LendingError::BorrowAttributionLimitNotExceeded as u32)
        )
    );

    test.advance_clock_by_slots(1).await;

    lending_market
        .update_reserve_config(
            &mut test,
            &lending_market_owner,
            &reserves[0],
            ReserveConfig {
                attributed_borrow_limit: 1,
                ..reserves[0].account.config
            },
            reserves[0].account.rate_limiter.config,
            None,
        )
        .await
        .unwrap();

    lending_market
        .mark_obligation_as_closable(
            &mut test,
            &obligations[0],
            &reserves[0],
            &risk_authority,
            10000,
        )
        .await
        .unwrap();

    let obligation_post = test.load_account::<Obligation>(obligations[0].pubkey).await;
    assert_eq!(
        obligation_post.account,
        Obligation {
            last_update: LastUpdate {
                slot: 1002,
                stale: false
            },
            closeable_by: 10000,
            ..obligations[0].account.clone()
        }
    );
}

#[tokio::test]
async fn invalid_signer() {
    let (mut test, lending_market, reserves, obligations, _users, lending_market_owner) =
        custom_scenario(
            &[
                ReserveArgs {
                    mint: usdc_mint::id(),
                    config: reserve_config_no_fees(),
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
                    config: reserve_config_no_fees(),
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
            &[ObligationArgs {
                deposits: vec![(usdc_mint::id(), 20 * FRACTIONAL_TO_USDC)],
                borrows: vec![(wsol_mint::id(), LAMPORTS_PER_SOL)],
            }],
        )
        .await;

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

    lending_market
        .update_reserve_config(
            &mut test,
            &lending_market_owner,
            &reserves[0],
            ReserveConfig {
                attributed_borrow_limit: 1,
                ..reserves[0].account.config
            },
            reserves[0].account.rate_limiter.config,
            None,
        )
        .await
        .unwrap();

    let rando = User::new_with_keypair(Keypair::new());
    let err = lending_market
        .mark_obligation_as_closable(&mut test, &obligations[0], &reserves[0], &rando, 10000)
        .await
        .unwrap_err()
        .unwrap();

    assert_eq!(
        err,
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(LendingError::InvalidAccountInput as u32)
        )
    );

    let err = test
        .process_transaction(
            &[malicious_mark_obligation_as_closeable(
                solend_mainnet::id(),
                obligations[0].pubkey,
                reserves[0].pubkey,
                lending_market.pubkey,
                risk_authority.keypair.pubkey(),
                10_000,
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

pub fn malicious_mark_obligation_as_closeable(
    program_id: Pubkey,
    obligation_pubkey: Pubkey,
    reserve_pubkey: Pubkey,
    lending_market_pubkey: Pubkey,
    risk_authority: Pubkey,
    closeable_by: Slot,
) -> Instruction {
    Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(obligation_pubkey, false),
            AccountMeta::new_readonly(lending_market_pubkey, false),
            AccountMeta::new_readonly(reserve_pubkey, false),
            AccountMeta::new_readonly(risk_authority, false),
        ],
        data: LendingInstruction::MarkObligationAsClosable { closeable_by }.pack(),
    }
}

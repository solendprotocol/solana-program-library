#![cfg(feature = "test-bpf")]

mod helpers;

use crate::solend_program_test::custom_scenario;
use helpers::solend_program_test::{SolendProgramTest, User};
use helpers::*;
use mock_pyth::mock_pyth_program;
use solana_program::instruction::InstructionError;
use solana_program::pubkey::Pubkey;
use solana_program::system_instruction::transfer;
use solana_program_test::*;
use solana_sdk::native_token::LAMPORTS_PER_SOL;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::Signer;
use solana_sdk::transaction::Transaction;
use solana_sdk::transaction::TransactionError;
use solend_program::error::LendingError;
use solend_program::instruction::init_lending_market;
use solend_program::state::{
    InitLendingMarketMetadataParams, LendingMarket, LendingMarketMetadata, RateLimiter,
    MARKET_DESCRIPTION_SIZE, MARKET_IMAGE_URL_SIZE, PROGRAM_VERSION,
};
use solend_sdk::state::MARKET_NAME_SIZE;

#[tokio::test]
async fn test_success() {
    let (mut test, lending_market, _reserves, _obligations, _users, lending_market_owner) =
        custom_scenario(&[], &[]).await;

    let instructions = [transfer(
        &test.context.payer.pubkey(),
        &lending_market_owner.keypair.pubkey(),
        LAMPORTS_PER_SOL,
    )];
    test.process_transaction(&instructions, None).await.unwrap();

    lending_market
        .update_metadata(
            &mut test,
            &lending_market_owner,
            InitLendingMarketMetadataParams {
                bump_seed: 0, // gets filled in automatically
                market_address: lending_market.pubkey,
                market_name: [2u8; MARKET_NAME_SIZE],
                market_description: [3u8; MARKET_DESCRIPTION_SIZE],
                market_image_url: [4u8; MARKET_IMAGE_URL_SIZE],
            },
        )
        .await
        .unwrap();

    let metadata_seeds = &[lending_market.pubkey.as_ref(), b"MetaData"];
    let (metadata_key, _bump_seed) =
        Pubkey::find_program_address(metadata_seeds, &solend_program::id());

    let lending_market_metadata = test
        .load_account::<LendingMarketMetadata>(metadata_key)
        .await;

    println!("{:#?}", lending_market_metadata);

    lending_market
        .update_metadata(
            &mut test,
            &lending_market_owner,
            InitLendingMarketMetadataParams {
                bump_seed: 0, // gets filled in automatically
                market_address: lending_market.pubkey,
                market_name: [5u8; MARKET_NAME_SIZE],
                market_description: [6u8; MARKET_DESCRIPTION_SIZE],
                market_image_url: [7u8; MARKET_IMAGE_URL_SIZE],
            },
        )
        .await
        .unwrap();

    let lending_market_metadata = test
        .load_account::<LendingMarketMetadata>(metadata_key)
        .await;

    println!("{:#?}", lending_market_metadata);
}

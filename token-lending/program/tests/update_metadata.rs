#![cfg(feature = "test-bpf")]

mod helpers;

use crate::solend_program_test::custom_scenario;
use solend_sdk::NULL_PUBKEY;

use helpers::*;

use solana_program::pubkey::Pubkey;
use solana_program::system_instruction::transfer;
use solana_program_test::*;
use solana_sdk::native_token::LAMPORTS_PER_SOL;

use solana_sdk::signer::Signer;

use solend_program::state::{
    LendingMarketMetadata, MARKET_DESCRIPTION_SIZE, MARKET_IMAGE_URL_SIZE, PADDING_SIZE,
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
            LendingMarketMetadata {
                bump_seed: 0, // gets filled in automatically
                market_name: [2u8; MARKET_NAME_SIZE],
                market_description: [3u8; MARKET_DESCRIPTION_SIZE],
                market_image_url: [4u8; MARKET_IMAGE_URL_SIZE],
                lookup_tables: [NULL_PUBKEY, NULL_PUBKEY, NULL_PUBKEY, NULL_PUBKEY],
                padding: [5u8; PADDING_SIZE],
            },
        )
        .await
        .unwrap();

    let metadata_seeds = &[lending_market.pubkey.as_ref(), b"MetaData"];
    let (metadata_key, _bump_seed) =
        Pubkey::find_program_address(metadata_seeds, &solend_program::id());

    let lending_market_metadata = test
        .load_zeroable_account::<LendingMarketMetadata>(metadata_key)
        .await;

    let (_, bump_seed) = Pubkey::find_program_address(
        &[&lending_market.pubkey.to_bytes()[..32], b"MetaData"],
        &solend_program::id(),
    );

    assert_eq!(
        lending_market_metadata.account,
        LendingMarketMetadata {
            bump_seed,
            market_name: [2u8; MARKET_NAME_SIZE],
            market_description: [3u8; MARKET_DESCRIPTION_SIZE],
            market_image_url: [4u8; MARKET_IMAGE_URL_SIZE],
            lookup_tables: [NULL_PUBKEY, NULL_PUBKEY, NULL_PUBKEY, NULL_PUBKEY,],
            padding: [5u8; PADDING_SIZE],
        }
    );

    lending_market
        .update_metadata(
            &mut test,
            &lending_market_owner,
            LendingMarketMetadata {
                bump_seed: 0, // gets filled in automatically
                market_name: [6u8; MARKET_NAME_SIZE],
                market_description: [7u8; MARKET_DESCRIPTION_SIZE],
                market_image_url: [8u8; MARKET_IMAGE_URL_SIZE],
                lookup_tables: [NULL_PUBKEY, NULL_PUBKEY, NULL_PUBKEY, NULL_PUBKEY],
                padding: [9u8; PADDING_SIZE],
            },
        )
        .await
        .unwrap();

    let lending_market_metadata = test
        .load_zeroable_account::<LendingMarketMetadata>(metadata_key)
        .await;

    let (_, bump_seed) = Pubkey::find_program_address(
        &[&lending_market.pubkey.to_bytes()[..32], b"MetaData"],
        &solend_program::id(),
    );

    assert_eq!(
        lending_market_metadata.account,
        LendingMarketMetadata {
            bump_seed,
            market_name: [6u8; MARKET_NAME_SIZE],
            market_description: [7u8; MARKET_DESCRIPTION_SIZE],
            market_image_url: [8u8; MARKET_IMAGE_URL_SIZE],
            lookup_tables: [NULL_PUBKEY, NULL_PUBKEY, NULL_PUBKEY, NULL_PUBKEY],
            padding: [9u8; PADDING_SIZE],
        }
    );
}

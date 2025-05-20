//! Adds a pool reward to a reserve.
//!
//! The signer must be the owner of the lending market, and there must be a free slot in the reserve.

use std::{borrow::Borrow, str::FromStr};

use solana_program::program_pack::Pack;
use solana_sdk::{pubkey::Pubkey, signature::Keypair, signer::Signer, system_instruction};
use solend_sdk::{
    instruction::{add_pool_reward, find_reward_vault_authority},
    state::{LendingMarket, PoolRewardEntry, PositionKind, Reserve},
};

use crate::{send_transaction, CommandResult, Config};

pub(crate) fn command(
    config: &mut Config,
    reserve_pubkey: Pubkey,
    position_kind: PositionKind,
    source_reward_token_account_pubkey: Pubkey,
    start_time_secs: u64,
    duration_secs: u32,
    token_amount: u64,
) -> CommandResult {
    let reserve_info = config.rpc_client.get_account(&reserve_pubkey)?;
    let reserve = Reserve::unpack_from_slice(reserve_info.data.borrow())?;
    let lending_market_info = config.rpc_client.get_account(&reserve.lending_market)?;
    let lending_market = LendingMarket::unpack(lending_market_info.data.borrow())?;

    if config.fee_payer.pubkey() != lending_market.owner {
        return Err(format!(
            "The fee payer must be the owner of the lending market '{}'",
            reserve.lending_market
        )
        .into());
    }

    let has_free_slot = reserve
        .pool_reward_manager(position_kind)
        .pool_rewards
        .iter()
        .any(|pr| matches!(pr, PoolRewardEntry::Vacant { .. }));

    if !has_free_slot {
        return Err(
            "There are no vacant slots to add the pool reward. Please crank it first".into(),
        );
    }

    let Some(source_reward_token_account) = config
        .rpc_client
        .get_token_account(&source_reward_token_account_pubkey)?
    else {
        return Err(format!(
            "Failed to fetch source token account '{}'",
            source_reward_token_account_pubkey
        )
        .into());
    };

    let reward_mint = Pubkey::from_str(&source_reward_token_account.mint)?;

    let reward_vault_keypair = Keypair::new();

    let create_account_ix = system_instruction::create_account(
        &config.fee_payer.pubkey(),
        &reward_vault_keypair.pubkey(),
        config
            .rpc_client
            .get_minimum_balance_for_rent_exemption(spl_token::state::Account::LEN)?,
        spl_token::state::Account::LEN as _,
        &spl_token::id(),
    );

    let (reward_vault_authority, reward_authority_bump) = find_reward_vault_authority(
        &config.lending_program_id,
        &reserve.lending_market,
        &reward_vault_keypair.pubkey(),
    );

    let add_reward_ix = add_pool_reward(
        config.lending_program_id,
        reward_authority_bump,
        position_kind,
        start_time_secs,
        start_time_secs + duration_secs as u64,
        token_amount,
        reserve_pubkey,
        reward_mint,
        source_reward_token_account_pubkey,
        reward_vault_authority,
        reward_vault_keypair.pubkey(),
        reserve.lending_market,
        lending_market.owner,
    );

    let recent_blockhash = config.rpc_client.get_latest_blockhash()?;

    let message = solana_sdk::message::Message::new_with_blockhash(
        &[create_account_ix, add_reward_ix],
        Some(&config.fee_payer.pubkey()),
        &recent_blockhash,
    );

    let transaction = solana_sdk::transaction::Transaction::new(
        &vec![config.fee_payer.as_ref()],
        message,
        recent_blockhash,
    );

    send_transaction(config, transaction)?;

    Ok(())
}

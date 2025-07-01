use std::{borrow::Borrow, str::FromStr};

use solana_program::program_pack::Pack;
use solana_sdk::pubkey::Pubkey;
use solend_sdk::{
    instruction::{edit_pool_reward, find_reward_vault_authority},
    state::{LendingMarket, PoolRewardEntry, PositionKind, Reserve},
};

use crate::{send_transaction, CommandResult, Config};

pub(crate) fn command(
    config: &mut Config,
    reserve_pubkey: Pubkey,
    position_kind: PositionKind,
    pool_reward_index: usize,
    new_end_time_secs: u64,
    reward_token_account_pubkey: Pubkey,
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

    let PoolRewardEntry::Occupied(pool_reward) = reserve
        .pool_reward_manager(position_kind)
        .pool_rewards
        .get(pool_reward_index)
        .ok_or_else(|| {
            format!(
                "Pool reward index {} does not exist for position kind {:?}",
                pool_reward_index, position_kind
            )
        })?
    else {
        return Err("Pool reward index is not occupied".into());
    };

    let Some(reward_vault_token_account) =
        config.rpc_client.get_token_account(&pool_reward.vault)?
    else {
        return Err(format!(
            "Failed to fetch pool reward vault '{}'",
            pool_reward.vault
        ))?;
    };

    let reward_mint = Pubkey::from_str(&reward_vault_token_account.mint)?;

    let (reward_vault_authority, reward_authority_bump) = find_reward_vault_authority(
        &config.lending_program_id,
        &reserve.lending_market,
        &pool_reward.vault,
    );

    let edit_reward_ix = edit_pool_reward(
        config.lending_program_id,
        reward_authority_bump,
        position_kind,
        pool_reward_index as _,
        new_end_time_secs,
        reserve_pubkey,
        reward_mint,
        reward_token_account_pubkey,
        reward_vault_authority,
        pool_reward.vault,
        reserve.lending_market,
        lending_market.owner,
    );

    let recent_blockhash = config.rpc_client.get_latest_blockhash()?;

    let message = solana_sdk::message::Message::new_with_blockhash(
        &[edit_reward_ix],
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

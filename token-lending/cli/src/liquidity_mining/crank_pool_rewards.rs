//! Each reserve has a limited number of slots that are used to declare pool rewards.
//! When all slots are occupied, the admin can no longer start new pool rewards.
//!
//! This is where cranking comes in.
//! Given a reserve, this command estimates the cheapest pool reward to crank out.
//! It loads each obligation and checks if it's tracking the pool reward.
//! Then it performs a claim on behalf of those obligation.

use indicatif::ProgressIterator;
use solana_client::{
    rpc_config::RpcProgramAccountsConfig,
    rpc_filter::{Memcmp, RpcFilterType},
};
use solana_program::program_pack::Pack;
use solana_sdk::pubkey::Pubkey;
use solend_sdk::{
    instruction::{claim_pool_reward, find_reward_vault_authority},
    state::{
        discriminator::AccountDiscriminator, Obligation, PoolRewardEntry, PositionKind, Reserve,
    },
};
use spl_associated_token_account::{
    get_associated_token_address, instruction::create_associated_token_account_idempotent,
};
use std::{borrow::Borrow, str::FromStr, time::SystemTime};

use crate::{send_transaction, CommandResult, Config};

/// How many claim ixs to send in a single transaction.
///
/// Will be determined empirically.
const CLAIM_IXS_BATCH_SIZE: usize = 4;

pub(crate) fn command(
    config: &mut Config,
    reserve_pubkey: Pubkey,
    position_kind: PositionKind,
) -> CommandResult {
    let reserve_info = config.rpc_client.get_account(&reserve_pubkey)?;
    let reserve = Reserve::unpack_from_slice(reserve_info.data.borrow())?;

    // since the time onchain is approximate, pick only those pool rewards that are over for sure
    // to avoid cranking for nothing
    let now_minus_an_hour_secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_secs()
        - 3_600;

    // first find a pool reward with the least number of user reward managers

    let Some((pool_reward_index, pool_reward)) = reserve
        .pool_reward_manager(position_kind)
        .pool_rewards
        .iter()
        .enumerate()
        .filter_map(|(index, pr)| {
            let PoolRewardEntry::Occupied(pr) = pr else {
                return None;
            };
            Some((index, pr))
        })
        .filter(|(_, pr)| now_minus_an_hour_secs >= pr.start_time_secs + pr.duration_secs as u64)
        .min_by_key(|(_, pr)| pr.num_user_reward_managers)
    else {
        println!("No pool rewards found for reserve '{reserve_pubkey}' ({position_kind:?})");
        return Ok(());
    };

    // now let's find the reward mint and other info about the vault

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

    // let's get all obligations

    let filter = RpcProgramAccountsConfig {
        filters: Some(vec![RpcFilterType::Memcmp(Memcmp::new_raw_bytes(
            0,
            vec![AccountDiscriminator::Obligation as u8],
        ))]),
        with_context: Some(false),
        ..Default::default()
    };
    let all_obligations = config
        .rpc_client
        .get_program_accounts_with_config(&config.lending_program_id, filter)?;

    // and filter only those that are still tracking the pool reward

    let ixs: Vec<_> = all_obligations
        .into_iter()
        .filter_map(|(pubkey, info)| {
            // get only those that can be unpacked
            Some((
                pubkey,
                Obligation::unpack(&info.data)
                    .inspect_err(|e| {
                        eprintln!("Failed to unpack obligation account '{pubkey}': {e:?}")
                    })
                    .ok()?,
            ))
        })
        .filter(|(_, obligation)| {
            // get only those that are tracking the pool reward
            obligation
                .user_reward_managers
                .iter()
                .filter(|m| m.reserve == reserve_pubkey)
                .filter(|m| m.position_kind == position_kind)
                .any(|m| {
                    m.rewards
                        .iter()
                        .find(|r| r.pool_reward_index == pool_reward_index)
                        .map(|r| r.pool_reward_id)
                        == Some(pool_reward.id)
                })
        })
        .map(|(obligation_pubkey, obligation)| (obligation_pubkey, obligation.owner))
        .map(|(obligation_pubkey, obligation_owner)| {
            let ata = get_associated_token_address(&obligation_owner, &reward_mint);

            let create_ata_ix = create_associated_token_account_idempotent(
                &config.fee_payer.as_ref().pubkey(),
                &obligation_owner,
                &reward_mint,
                &spl_token::id(),
            );

            let claim_ix = claim_pool_reward(
                config.lending_program_id,
                reward_authority_bump,
                position_kind,
                obligation_pubkey,
                ata,
                reserve_pubkey,
                reward_mint,
                reward_vault_authority,
                pool_reward.vault,
                reserve.lending_market,
            );

            std::iter::once(create_ata_ix).chain(std::iter::once(claim_ix))
        })
        .flatten()
        .collect();

    for ixs in ixs.chunks(CLAIM_IXS_BATCH_SIZE).progress() {
        let recent_blockhash = config.rpc_client.get_latest_blockhash()?;

        let message = solana_sdk::message::Message::new_with_blockhash(
            ixs,
            Some(&config.fee_payer.pubkey()),
            &recent_blockhash,
        );

        let transaction = solana_sdk::transaction::Transaction::new(
            &vec![config.fee_payer.as_ref()],
            message,
            recent_blockhash,
        );

        send_transaction(config, transaction)?;
    }

    Ok(())
}

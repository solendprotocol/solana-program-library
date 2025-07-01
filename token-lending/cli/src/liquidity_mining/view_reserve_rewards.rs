use std::{borrow::Borrow, time::SystemTime};

use solana_program::program_pack::Pack;
use solana_sdk::pubkey::Pubkey;
use solend_sdk::state::{PoolReward, PoolRewardEntry, PoolRewardManager, Reserve, MAX_REWARDS};

use crate::{CommandResult, Config};

pub(crate) fn command(config: &mut Config, reserve_pubkey: Pubkey) -> CommandResult {
    let reserve_info = config.rpc_client.get_account(&reserve_pubkey)?;
    let reserve = Reserve::unpack_from_slice(reserve_info.data.borrow())?;

    println!();
    println!("=== Borrow Rewards ===");
    print_pool_rewards(&reserve.borrows_pool_reward_manager)?;

    println!();
    println!("=== Deposit Rewards ===");
    print_pool_rewards(&reserve.deposits_pool_reward_manager)?;

    Ok(())
}

fn print_pool_rewards(manager: &PoolRewardManager) -> CommandResult {
    // since the time onchain is approximate, pick only those pool rewards that are over for sure
    // to avoid cranking for nothing
    let now_secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_secs();

    let open_count = manager
        .pool_rewards
        .iter()
        .filter_map(|pr| {
            let PoolRewardEntry::Occupied(pr) = pr else {
                return None;
            };
            Some(pr)
        })
        .filter(|pr| now_secs < pr.start_time_secs + pr.duration_secs as u64)
        .count();

    println!("Total shares amount to {}.", manager.total_shares);
    println!("There are {open_count}/{MAX_REWARDS} pool rewards running.");
    manager
        .pool_rewards
        .iter()
        .enumerate()
        .filter_map(|(index, pr)| {
            let PoolRewardEntry::Occupied(pr) = pr else {
                return None;
            };
            Some((index, *pr.clone()))
        })
        .for_each(
            |(
                index,
                PoolReward {
                    id,
                    vault,
                    start_time_secs,
                    duration_secs,
                    total_rewards,
                    cumulative_rewards_per_share,
                    num_user_reward_managers,
                },
            )| {
                println!("{index}) Pool reward {id:?}:");
                println!("  Vault: {vault}");
                println!("  Start time: {start_time_secs}");
                println!("  Duration: {duration_secs}");
                let ends_in =
                    duration_secs.saturating_sub(now_secs.saturating_sub(start_time_secs) as _);
                if ends_in > 0 {
                    println!("  Ends in {ends_in}s");
                } else {
                    println!("  Ended");
                }
                println!("  Total rewards: {total_rewards}");
                println!("  Cumulative rewards per share: {cumulative_rewards_per_share}");
                println!("  Number of user reward managers: {num_user_reward_managers}");
            },
        );

    Ok(())
}

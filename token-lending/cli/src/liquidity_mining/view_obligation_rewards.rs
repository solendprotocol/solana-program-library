use std::{borrow::Borrow, time::SystemTime};

use solana_program::program_pack::Pack;
use solana_sdk::pubkey::Pubkey;
use solend_sdk::state::{Obligation, Reserve};

use crate::{CommandResult, Config};

pub(crate) fn command(config: &mut Config, obligation_pubkey: Pubkey) -> CommandResult {
    let obligation_info = config.rpc_client.get_account(&obligation_pubkey)?;
    let obligation = Obligation::unpack_from_slice(obligation_info.data.borrow())?;

    let now_secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_secs();

    for user_manager in obligation.user_reward_managers.iter() {
        let reserve_info = config.rpc_client.get_account(&user_manager.reserve)?;
        let reserve = Reserve::unpack_from_slice(reserve_info.data.borrow())?;
        println!(
            "Rewards for reserve {} {:?} last updated {}s ago",
            user_manager.reserve,
            user_manager.position_kind,
            now_secs.saturating_sub(user_manager.last_update_time_secs)
        );

        let pool_reward_manager = reserve.pool_reward_manager(user_manager.position_kind);

        let share = user_manager.share as f64 / pool_reward_manager.total_shares as f64;
        println!(
            "  Mines {}% in {} rewards",
            share * 100.0,
            user_manager.rewards.len()
        );
    }

    Ok(())
}

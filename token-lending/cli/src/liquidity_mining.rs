//! CLI commands related to liquidity mining.

mod add_pool_reward;
mod close_pool_reward;
mod crank_pool_rewards;
mod edit_pool_reward;
mod find_obligations_to_fund;
mod migrate_all_reserves;
mod view_obligation_rewards;
mod view_reserve_rewards;

pub(crate) use add_pool_reward::command as command_add_pool_reward;
pub(crate) use close_pool_reward::command as command_close_pool_reward;
pub(crate) use crank_pool_rewards::command as command_crank_pool_rewards;
pub(crate) use edit_pool_reward::command as command_edit_pool_reward;
pub(crate) use find_obligations_to_fund::command as command_find_obligations_to_fund_for_liquidity_mining;
pub(crate) use migrate_all_reserves::command as command_migrate_all_reserves_for_liquidity_mining;
pub(crate) use view_obligation_rewards::command as command_view_obligation_rewards;
pub(crate) use view_reserve_rewards::command as command_view_reserve_rewards;

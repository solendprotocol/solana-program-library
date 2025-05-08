//! CLI commands related to liquidity mining.

mod migrate_all_reserves;

pub(crate) use migrate_all_reserves::command_upgrade_reserves_to_v2_1_0;

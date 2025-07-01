//! Temporary command that migrates all reserves to their new version.
//!
//! Delete once @v2.1.0 is fully deployed.
//!
//! Running this command before the upgrade:
//! > Found 1621 reserves to upgrade
//! > We'll spend ~54.46 $SOL on rent
//! > There are 87 reserves that were not used in the last 7 days as of 2025-05-08.

use solana_account_decoder::UiAccountEncoding;
use solana_account_decoder::UiDataSliceConfig;
use solana_client::rpc_config::RpcAccountInfoConfig;
use solana_client::rpc_config::RpcProgramAccountsConfig;
use solana_client::rpc_filter::RpcFilterType;
use solana_sdk::compute_budget::ComputeBudgetInstruction;
use solana_sdk::message::Message;
use solana_sdk::native_token::LAMPORTS_PER_SOL;
use solana_sdk::program_pack::Pack;
use solana_sdk::transaction::Transaction;
use solend_sdk::instruction::upgrade_reserve_to_v2_1_0;
use solend_sdk::state::Reserve;
use solend_sdk::state::RESERVE_LEN_V2_0_2;

use crate::send_transaction;
use crate::CommandResult;
use crate::Config;

/// How many reserves to upgrade in a single transaction.
///
/// We found the right value empirically.
const BATCH_SIZE: usize = 25;
/// How much to pay for compute units.
/// Helps lending txs.
const CU_PRICE: u64 = 3000;

/// Upgrades all reserves to the new version.
pub(crate) fn command(config: &mut Config) -> CommandResult {
    let reserve_new_rent = config
        .rpc_client
        .get_minimum_balance_for_rent_exemption(Reserve::LEN)?;

    // reserves before migration were sized to RESERVE_LEN_V2_0_2 and we're only interested in those
    let filter = RpcProgramAccountsConfig {
        filters: Some(vec![RpcFilterType::DataSize(RESERVE_LEN_V2_0_2 as _)]),
        // with_context: Some(false),
        account_config: RpcAccountInfoConfig {
            data_slice: Some(UiDataSliceConfig {
                offset: 1,
                length: 8, // we don't need the data
            }),
            encoding: Some(UiAccountEncoding::Base64),
            ..Default::default()
        },
        ..Default::default()
    };
    let reserves_to_upgrade = config
        .rpc_client
        .get_program_accounts_with_config(&config.lending_program_id, filter)?;

    println!("Found {} reserves to upgrade", reserves_to_upgrade.len());

    let missing_rent: u64 = reserves_to_upgrade
        .iter()
        .map(|(_, acc)| reserve_new_rent.saturating_sub(acc.lamports))
        .sum();

    println!(
        "We'll spend ~{:.2} $SOL on rent",
        missing_rent as f64 / LAMPORTS_PER_SOL as f64
    );

    for reserves in reserves_to_upgrade.chunks(BATCH_SIZE) {
        let mut ixs = vec![ComputeBudgetInstruction::set_compute_unit_price(CU_PRICE)];
        ixs.extend(reserves.iter().map(|(reserve_pubkey, _)| {
            upgrade_reserve_to_v2_1_0(
                config.lending_program_id,
                *reserve_pubkey,
                config.fee_payer.pubkey(),
            )
        }));

        let recent_blockhash = config.rpc_client.get_latest_blockhash()?;

        let message =
            Message::new_with_blockhash(&ixs, Some(&config.fee_payer.pubkey()), &recent_blockhash);

        let transaction =
            Transaction::new(&vec![config.fee_payer.as_ref()], message, recent_blockhash);

        send_transaction(config, transaction)?;
    }

    Ok(())
}

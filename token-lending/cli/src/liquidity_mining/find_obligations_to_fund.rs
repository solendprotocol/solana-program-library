use std::fs::File;
use std::io::Write;
use std::path::Path;

use solana_account_decoder::UiAccountEncoding;
use solana_account_decoder::UiDataSliceConfig;
use solana_client::rpc_config::RpcAccountInfoConfig;
use solana_client::rpc_config::RpcProgramAccountsConfig;
use solana_client::rpc_filter::RpcFilterType;
use solana_sdk::native_token::lamports_to_sol;
use solana_sdk::native_token::LAMPORTS_PER_SOL;
use solend_sdk::state::Obligation;

use crate::CommandResult;
use crate::Config;

pub(crate) fn command(config: &mut Config, output_csv: impl AsRef<Path>) -> CommandResult {
    let rent_for_2_0_2 = config
        .rpc_client
        .get_minimum_balance_for_rent_exemption(Obligation::MIN_LEN)?;
    let rent_for_overhead = config
        .rpc_client
        .get_minimum_balance_for_rent_exemption(1)?;
    let rent_per_reserve = config
        .rpc_client
        .get_minimum_balance_for_rent_exemption(50)?;

    // obligations before migration were sized to Obligation::MIN_LEN and we're only interested in those
    let filter = RpcProgramAccountsConfig {
        filters: Some(vec![RpcFilterType::DataSize(Obligation::MIN_LEN as _)]),
        with_context: Some(false),
        account_config: RpcAccountInfoConfig {
            data_slice: Some(UiDataSliceConfig {
                offset: 10 + 32 * 2 + 16 * 7 + 1 + 1 + 14,
                length: 2, // first byte for deposits len, second for borrows len
            }),
            encoding: Some(UiAccountEncoding::Base64),
            ..Default::default()
        },
    };
    let all_obligations = config
        .rpc_client
        .get_program_accounts_with_config(&config.lending_program_id, filter)?;

    println!("Found {} obligations in total", all_obligations.len());

    let obligations_that_need_rent: Vec<_> = all_obligations
        .into_iter()
        .filter_map(|(pubkey, account)| {
            assert_eq!(account.data.len(), 2);
            let deposits_count = account.data[0] as usize;
            let borrows_count = account.data[1] as usize;
            assert!(deposits_count + borrows_count <= 10);
            let positions_count = deposits_count + borrows_count;

            if positions_count == 0 {
                None
            } else {
                Some((pubkey, positions_count, account.lamports))
            }
        })
        .map(|(pubkey, positions_count, current_rent)| {
            let extra_rent = current_rent - rent_for_2_0_2;
            let required_extra_rent = rent_for_overhead + rent_per_reserve * positions_count as u64;

            let extra_rent_to_add = required_extra_rent.saturating_sub(extra_rent);

            (pubkey, extra_rent_to_add)
        })
        .filter(|(_, extra_rent_to_add)| *extra_rent_to_add > 0)
        .collect();

    println!(
        "Found {} obligations that need rent",
        obligations_that_need_rent.len()
    );

    let missing_rent: u64 = obligations_that_need_rent
        .iter()
        .map(|(_, extra_rent_to_add)| *extra_rent_to_add)
        .sum();

    println!(
        "We'll spend ~{:.2} $SOL on rent",
        missing_rent as f64 / LAMPORTS_PER_SOL as f64
    );
    println!(
        "Writing the amounts to CSV file at '{}'",
        output_csv.as_ref().display()
    );
    let mut file = File::create(output_csv.as_ref())?;

    // write the header used by the tokens CLI
    writeln!(file, "recipient,amount,lockup_date")?;

    for (recipient, lamports) in obligations_that_need_rent {
        writeln!(file, "{},{},", recipient, lamports_to_sol(lamports))?;
    }

    println!("Done!");
    println!("Use <https://lib.rs/crates/solana-tokens> to distribute the rent");
    println!();
    println!(
        "$ solana-tokens distribute-tokens --input-csv {} --from <KEYPAIR> --fee-payer <KEYPAIR>",
        output_csv
            .as_ref()
            .canonicalize()
            .expect("canonicalize")
            .display()
    );

    Ok(())
}

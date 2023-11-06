use solana_client::rpc_client::RpcClient;
use solana_sdk::pubkey;
use std::collections::HashMap;

use solend_sdk::{
    offchain_utils::{
        get_solend_accounts_as_map, offchain_refresh_obligation, offchain_refresh_reserve_interest,
    },
    solend_mainnet,
};

#[derive(Debug, Clone, Default)]
struct Position {
    pub deposit_balance: u64,
    pub borrow_balance: u64,
}

pub fn main() {
    let rpc_url = std::env::var("RPC_URL")
        .unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_string());
    let rpc_client = RpcClient::new(rpc_url);

    let mut accounts = get_solend_accounts_as_map(&solend_mainnet::id(), &rpc_client).unwrap();

    // update solend-specific interest variables
    let slot = rpc_client.get_slot().unwrap();
    for reserve in accounts.reserves.values_mut() {
        let _ = offchain_refresh_reserve_interest(reserve, slot);
    }

    for obligation in accounts.obligations.values_mut() {
        offchain_refresh_obligation(obligation, &accounts.reserves).unwrap();
    }

    // calculate jitosol balances per user across all pools
    let jitosol = pubkey!("J1toso1uCk3RLmjorhTtrVwY9HJ7X8V9yYac6Y7kGCPn");
    let mut user_to_position = HashMap::new();

    for obligation in accounts.obligations.values() {
        for deposit in &obligation.deposits {
            let deposit_reserve = accounts.reserves.get(&deposit.deposit_reserve).unwrap();
            if deposit_reserve.liquidity.mint_pubkey == jitosol {
                let position = user_to_position
                    .entry(obligation.owner)
                    .or_insert(Position::default());

                // convert cJitoSol to JitoSol
                let cjitosol_deposited = deposit.deposited_amount;
                let jitosol_deposited = deposit_reserve
                    .collateral_exchange_rate()
                    .unwrap()
                    .collateral_to_liquidity(cjitosol_deposited)
                    .unwrap();

                position.deposit_balance += jitosol_deposited;
            }
        }

        for borrow in &obligation.borrows {
            let borrow_reserve = accounts.reserves.get(&borrow.borrow_reserve).unwrap();
            if borrow_reserve.liquidity.mint_pubkey == jitosol {
                let position = user_to_position
                    .entry(obligation.owner)
                    .or_insert(Position::default());

                position.borrow_balance += borrow.borrowed_amount_wads.try_round_u64().unwrap();
            }
        }
    }

    println!("Done refreshing");
    println!("{:#?}", user_to_position);
}

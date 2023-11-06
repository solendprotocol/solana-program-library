#![allow(missing_docs)]

use solana_client::rpc_client::RpcClient;
use solana_program::slot_history::Slot;
// use pyth_sdk_solana;
use solana_program::program_error::ProgramError;
use std::result::Result;

use crate::{state::LastUpdate, NULL_PUBKEY};

use solana_program::{program_pack::Pack, pubkey::Pubkey};

use crate::math::{Decimal, Rate, TryAdd, TryMul};

use crate::state::{LendingMarket, Obligation, Reserve};
use std::{collections::HashMap, error::Error};

#[derive(Debug, Clone)]
pub struct SolendAccounts {
    pub lending_markets: HashMap<Pubkey, LendingMarket>,
    pub reserves: HashMap<Pubkey, Reserve>,
    pub obligations: HashMap<Pubkey, Obligation>,
}

pub fn get_solend_accounts_as_map(
    lending_program_id: &Pubkey,
    client: &RpcClient,
) -> Result<SolendAccounts, Box<dyn Error>> {
    let accounts = client.get_program_accounts(lending_program_id)?;

    let (lending_markets, reserves, obligations) = accounts.into_iter().fold(
        (HashMap::new(), HashMap::new(), HashMap::new()),
        |(mut lending_markets, mut reserves, mut obligations), (pubkey, account)| {
            match account.data.len() {
                Obligation::LEN => {
                    if let Ok(o) = Obligation::unpack(&account.data) {
                        obligations.insert(pubkey, o);
                    }
                }
                Reserve::LEN => {
                    if let Ok(r) = Reserve::unpack(&account.data) {
                        reserves.insert(pubkey, r);
                    }
                }
                LendingMarket::LEN => {
                    if let Ok(l) = LendingMarket::unpack(&account.data) {
                        lending_markets.insert(pubkey, l);
                    }
                }
                _ => (),
            };
            (lending_markets, reserves, obligations)
        },
    );

    Ok(SolendAccounts {
        lending_markets,
        reserves,
        obligations,
    })
}

pub fn offchain_refresh_reserve_interest(
    reserve: &mut Reserve,
    slot: Slot,
) -> Result<(), Box<dyn Error>> {
    reserve.accrue_interest(slot)?;
    reserve.last_update = LastUpdate { slot, stale: false };

    Ok(())
}

pub fn offchain_refresh_reserve(
    _pubkey: &Pubkey,
    reserve: &mut Reserve,
    slot: Slot,
    prices: &HashMap<Pubkey, Option<Decimal>>,
) -> Result<(), Box<dyn Error>> {
    let pyth_oracle = reserve.liquidity.pyth_oracle_pubkey;
    let switchboard_oracle = reserve.liquidity.switchboard_oracle_pubkey;

    let price = if let Some(Some(price)) = prices.get(&pyth_oracle) {
        if pyth_oracle != NULL_PUBKEY {
            Some(*price)
        } else {
            None
        }
    } else if let Some(Some(price)) = prices.get(&switchboard_oracle) {
        if switchboard_oracle != NULL_PUBKEY {
            Some(*price)
        } else {
            None
        }
    } else {
        None
    };

    if let Some(price) = price {
        reserve.liquidity.market_price = price;
    } else {
        return Err("No price".into());
    }

    reserve.accrue_interest(slot)?;
    reserve.last_update = LastUpdate { slot, stale: false };

    Ok(())
}

pub fn offchain_refresh_obligation(
    o: &mut Obligation,
    reserves: &HashMap<Pubkey, Reserve>,
) -> Result<(), Box<dyn Error>> {
    o.deposited_value = Decimal::zero();
    o.super_unhealthy_borrow_value = Decimal::zero();
    o.unhealthy_borrow_value = Decimal::zero();
    o.borrowed_value = Decimal::zero();

    for collateral in &mut o.deposits {
        let deposit_reserve = reserves
            .get(&collateral.deposit_reserve)
            .ok_or(ProgramError::Custom(35))?;

        let liquidity_amount = deposit_reserve
            .collateral_exchange_rate()?
            .decimal_collateral_to_liquidity(collateral.deposited_amount.into())?;

        let market_value = deposit_reserve.market_value(liquidity_amount)?;
        let liquidation_threshold_rate =
            Rate::from_percent(deposit_reserve.config.liquidation_threshold);
        let max_liquidation_threshold_rate =
            Rate::from_percent(deposit_reserve.config.max_liquidation_threshold);

        collateral.market_value = market_value;

        o.deposited_value = o.deposited_value.try_add(market_value)?;
        o.unhealthy_borrow_value = o
            .unhealthy_borrow_value
            .try_add(market_value.try_mul(liquidation_threshold_rate)?)?;
        o.super_unhealthy_borrow_value = o
            .super_unhealthy_borrow_value
            .try_add(market_value.try_mul(max_liquidation_threshold_rate)?)?;
    }

    let mut max_borrow_weight = None;

    for (index, liquidity) in o.borrows.iter_mut().enumerate() {
        let borrow_reserve = reserves.get(&liquidity.borrow_reserve).unwrap();
        liquidity.accrue_interest(borrow_reserve.liquidity.cumulative_borrow_rate_wads)?;

        let market_value = borrow_reserve.market_value(liquidity.borrowed_amount_wads)?;
        liquidity.market_value = market_value;

        o.borrowed_value = o
            .borrowed_value
            .try_add(market_value.try_mul(borrow_reserve.borrow_weight())?)?;

        let borrow_weight_and_pubkey = (
            borrow_reserve.config.added_borrow_weight_bps,
            borrow_reserve.liquidity.mint_pubkey,
        );

        max_borrow_weight = match max_borrow_weight {
            None => Some((borrow_weight_and_pubkey, index)),
            Some((max_borrow_weight_and_pubkey, _)) => {
                if liquidity.borrowed_amount_wads > Decimal::zero()
                    && borrow_weight_and_pubkey > max_borrow_weight_and_pubkey
                {
                    Some((borrow_weight_and_pubkey, index))
                } else {
                    max_borrow_weight
                }
            }
        };
    }

    Ok(())
}

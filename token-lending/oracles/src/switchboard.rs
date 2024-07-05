use crate::get_oracle_type;
use crate::OracleType;
use solend_sdk::math::TryDiv;
use solend_sdk::math::TryMul;

use solana_program::{
    account_info::AccountInfo, entrypoint::ProgramResult, msg, program_error::ProgramError,
    sysvar::clock::Clock,
};
use solend_sdk::{error::LendingError, math::Decimal};
use crate::{
    switchboard_on_demand_devnet, switchboard_on_demand_mainnet, switchboard_v2_devnet,
    switchboard_v2_mainnet,
};
use std::result::Result;

use switchboard_on_demand::on_demand::accounts::pull_feed::PullFeedAccountData as SbOnDemandFeed;
use switchboard_v2::AggregatorAccountData;

pub fn get_switchboard_price(
    switchboard_feed_info: &AccountInfo,
    clock: &Clock,
) -> Result<Decimal, ProgramError> {
    if *switchboard_feed_info.key == solend_sdk::NULL_PUBKEY {
        return Err(LendingError::NullOracleConfig.into());
    }
    if switchboard_feed_info.owner == &switchboard_v2_mainnet::id()
        || switchboard_feed_info.owner == &switchboard_v2_devnet::id()
    {
        return get_switchboard_price_v2(switchboard_feed_info, clock, true);
    }

    if switchboard_feed_info.owner == &switchboard_on_demand_devnet::id()
        || switchboard_feed_info.owner == &switchboard_on_demand_mainnet::id()
    {
        return get_switchboard_price_on_demand(switchboard_feed_info, clock, true);
    }
    Err(LendingError::NullOracleConfig.into())
}

pub fn get_switchboard_price_on_demand(
    switchboard_feed_info: &AccountInfo,
    clock: &Clock,
    check_staleness: bool,
) -> Result<Decimal, ProgramError> {
    const STALE_AFTER_SLOTS_ELAPSED: u64 = 240;
    let data = switchboard_feed_info.try_borrow_data()?;
    let feed = SbOnDemandFeed::parse(data).map_err(|_| ProgramError::InvalidAccountData)?;
    let slots_elapsed = clock
        .slot
        .checked_sub(feed.result.slot)
        .ok_or(LendingError::MathOverflow)?;
    if check_staleness && slots_elapsed >= STALE_AFTER_SLOTS_ELAPSED {
        msg!("Switchboard oracle price is stale");
        return Err(LendingError::InvalidOracleConfig.into());
    }
    let price_desc = feed.value().ok_or(ProgramError::InvalidAccountData)?;
    if price_desc.mantissa() < 0 {
        msg!("Switchboard oracle price is negative which is not allowed");
        return Err(LendingError::InvalidOracleConfig.into());
    }
    let price_mantissa = Decimal::from(price_desc.mantissa() as u128);
    let exp = Decimal::from((10u128).checked_pow(price_desc.scale()).unwrap());
    let price = price_mantissa.try_div(exp)?;

    let range_desc = feed.range().ok_or(ProgramError::InvalidAccountData)?;
    if range_desc.mantissa() < 0 {
        msg!("Switchboard oracle price range is negative which is not allowed");
        return Err(LendingError::InvalidOracleConfig.into());
    }
    let range_mantissa = Decimal::from(range_desc.mantissa() as u128);
    let range_exp = Decimal::from((10u128).checked_pow(range_desc.scale()).unwrap());
    let range = range_mantissa.try_div(range_exp)?;

    if range.try_mul(10_u64)? > price {
        msg!(
            "Oracle price range is too wide. price: {}, conf: {}",
            price,
            range,
        );
        return Err(LendingError::InvalidOracleConfig.into());
    }

    Ok(price)
}

pub fn get_switchboard_price_v2(
    switchboard_feed_info: &AccountInfo,
    clock: &Clock,
    check_staleness: bool,
) -> Result<Decimal, ProgramError> {
    const STALE_AFTER_SLOTS_ELAPSED: u64 = 240;
    let data = &switchboard_feed_info.try_borrow_data()?;
    let feed = AggregatorAccountData::new_from_bytes(data)?;

    let slots_elapsed = clock
        .slot
        .checked_sub(feed.latest_confirmed_round.round_open_slot)
        .ok_or(LendingError::MathOverflow)?;
    if check_staleness && slots_elapsed >= STALE_AFTER_SLOTS_ELAPSED {
        msg!("Switchboard oracle price is stale");
        return Err(LendingError::InvalidOracleConfig.into());
    }

    let price_switchboard_desc = feed.get_result()?;
    if price_switchboard_desc.mantissa < 0 {
        msg!("Switchboard oracle price is negative which is not allowed");
        return Err(LendingError::InvalidOracleConfig.into());
    }
    let price = Decimal::from(price_switchboard_desc.mantissa as u128);
    let exp = Decimal::from((10u128).checked_pow(price_switchboard_desc.scale).unwrap());
    price.try_div(exp)
}

pub fn validate_switchboard_keys(switchboard_feed_info: &AccountInfo) -> ProgramResult {
    if *switchboard_feed_info.key == solend_sdk::NULL_PUBKEY {
        return Ok(());
    }

    match get_oracle_type(switchboard_feed_info)? {
        OracleType::Switchboard => validate_switchboard_v2_keys(switchboard_feed_info),
        OracleType::SbOnDemand => validate_sb_on_demand_keys(switchboard_feed_info),
        _ => Err(LendingError::InvalidOracleConfig.into()),
    }
}

/// validates switchboard AccountInfo
fn validate_switchboard_v2_keys(switchboard_feed_info: &AccountInfo) -> ProgramResult {
    if *switchboard_feed_info.key == solend_sdk::NULL_PUBKEY {
        return Ok(());
    }
    if switchboard_feed_info.owner != &switchboard_v2_mainnet::id()
        && switchboard_feed_info.owner != &switchboard_v2_devnet::id()
    {
        msg!("Switchboard account provided is not owned by the switchboard oracle program");
        return Err(LendingError::InvalidOracleConfig.into());
    }

    let data = &switchboard_feed_info.try_borrow_data()?;
    AggregatorAccountData::new_from_bytes(data)?;

    Ok(())
}

/// validates switchboard on-demand AccountInfo
pub fn validate_sb_on_demand_keys(switchboard_feed_info: &AccountInfo) -> ProgramResult {
    if *switchboard_feed_info.key == solend_sdk::NULL_PUBKEY {
        return Ok(());
    }

    if switchboard_feed_info.owner != &switchboard_on_demand_mainnet::id()
        && switchboard_feed_info.owner != &switchboard_on_demand_devnet::id()
    {
        msg!("Switchboard account provided is not owned by the switchboard oracle program");
        return Err(LendingError::InvalidOracleConfig.into());
    }

    let data = switchboard_feed_info.try_borrow_data()?;
    SbOnDemandFeed::parse(data).map_err(|_| ProgramError::InvalidAccountData)?;

    Ok(())
}

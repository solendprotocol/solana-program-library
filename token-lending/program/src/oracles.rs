use crate::{
    self as solend_program,
    error::LendingError,
    math::{Decimal, TryDiv, TryMul},
};
use pyth_sdk_solana::{self, state::ProductAccount};
use solana_program::{
    account_info::AccountInfo,
    msg,
    program_error::{PrintProgramError, ProgramError},
    program_pack::{IsInitialized, Pack},
    pubkey::Pubkey,
    sysvar::clock::{self, Clock},
};
use std::{cmp::min, convert::TryInto, result::Result};

pub fn get_pyth_price(
    pyth_price_info: &AccountInfo,
    clock: &Clock,
) -> Result<Decimal, ProgramError> {
    const MAX_PYTH_CONFIDENCE_RATIO: u64 = 10;
    const STALE_AFTER_SLOTS_ELAPSED: u64 = 240;

    if *pyth_price_info.key == solend_program::NULL_PUBKEY {
        return Err(LendingError::NullOracleConfig.into());
    }

    let price_feed = pyth_sdk_solana::load_price_feed_from_account_info(pyth_price_info)?;
    let pyth_price = price_feed
        .get_latest_available_price_within_duration(clock.unix_timestamp, STALE_AFTER_SLOTS_ELAPSED)
        .ok_or(LendingError::InvalidOracleConfig)?;

    let price: u64 = pyth_price.price.try_into().map_err(|_| {
        msg!("Oracle price cannot be negative");
        LendingError::InvalidOracleConfig
    })?;

    // Perhaps confidence_ratio should exist as a per reserve config
    // 100/confidence_ratio = maximum size of confidence range as a percent of price
    // confidence_ratio of 10 filters out pyth prices with conf > 10% of price
    if pyth_price
        .conf
        .checked_mul(MAX_PYTH_CONFIDENCE_RATIO)
        .unwrap()
        > price
    {
        msg!(
            "Oracle price confidence is too wide. price: {}, conf: {}",
            price,
            pyth_price.conf,
        );
        return Err(LendingError::InvalidOracleConfig.into());
    }

    let market_price = if pyth_price.expo >= 0 {
        let exponent = pyth_price
            .expo
            .try_into()
            .map_err(|_| LendingError::MathOverflow)?;
        let zeros = 10u64
            .checked_pow(exponent)
            .ok_or(LendingError::MathOverflow)?;
        Decimal::from(price).try_mul(zeros)?
    } else {
        let exponent = pyth_price
            .expo
            .checked_abs()
            .ok_or(LendingError::MathOverflow)?
            .try_into()
            .map_err(|_| LendingError::MathOverflow)?;
        let decimals = 10u64
            .checked_pow(exponent)
            .ok_or(LendingError::MathOverflow)?;
        Decimal::from(price).try_div(decimals)?
    };

    Ok(market_price)
}

#![allow(missing_docs)]
use crate::{
    self as solend_program,
    error::LendingError,
    math::{Decimal, TryDiv, TryMul},
};
use pyth_sdk_solana;
use solana_program::{
    account_info::AccountInfo, msg, program_error::ProgramError, sysvar::clock::Clock,
};
use std::{convert::TryInto, result::Result};

pub fn get_pyth_price(
    pyth_price_info: &AccountInfo,
    clock: &Clock,
) -> Result<Decimal, ProgramError> {
    const MAX_PYTH_CONFIDENCE_RATIO: u64 = 10;
    const STALE_AFTER_SECONDS_ELAPSED: u64 = 120;

    if *pyth_price_info.key == solend_program::NULL_PUBKEY {
        return Err(LendingError::NullOracleConfig.into());
    }

    let price_feed = pyth_sdk_solana::load_price_feed_from_account_info(pyth_price_info)?;
    let pyth_price = price_feed
        .get_latest_available_price_within_duration(
            clock.unix_timestamp,
            STALE_AFTER_SECONDS_ELAPSED,
        )
        .ok_or_else(|| {
            msg!("Pyth oracle price is too stale!");
            LendingError::InvalidOracleConfig
        })?;

    let price: u64 = pyth_price.price.try_into().map_err(|_| {
        msg!("Oracle price cannot be negative");
        LendingError::InvalidOracleConfig
    })?;

    // Perhaps confidence_ratio should exist as a per reserve config
    // 100/confidence_ratio = maximum size of confidence range as a percent of price
    // confidence_ratio of 10 filters out pyth prices with conf > 10% of price
    if pyth_price
        .conf
        .saturating_mul(MAX_PYTH_CONFIDENCE_RATIO)
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

#[cfg(test)]
mod test {
    use super::*;
    use bytemuck::bytes_of_mut;
    use proptest::prelude::*;
    use pyth_sdk_solana::{
        state::{AccountType, CorpAction, PriceAccount, PriceInfo, PriceType, MAGIC, VERSION_2},
        PriceStatus,
    };
    use solana_program::pubkey::Pubkey;

    #[derive(Clone, Debug)]
    struct PythPriceTestCase {
        price_account: PriceAccount,
        clock: Clock,
        expected_result: Result<Decimal, ProgramError>,
    }

    fn pyth_price_cases() -> impl Strategy<Value = PythPriceTestCase> {
        prop_oneof![
            // case 2: failure. bad magic value
            Just(PythPriceTestCase {
                price_account: PriceAccount {
                    magic: MAGIC + 1,
                    ver: VERSION_2,
                    atype: AccountType::Price as u32,
                    ptype: PriceType::Price,
                    expo: 10,
                    agg: PriceInfo {
                        price: 10,
                        conf: 1,
                        status: PriceStatus::Trading,
                        corp_act: CorpAction::NoCorpAct,
                        pub_slot: 0
                    },
                    ..PriceAccount::default()
                },
                clock: Clock {
                    unix_timestamp: 0,
                    ..Clock::default()
                },
                // PythError::InvalidAccountData. The struct is private
                expected_result: Err(ProgramError::Custom(0)),
            }),
            // case 3: failure. bad version number
            Just(PythPriceTestCase {
                price_account: PriceAccount {
                    magic: MAGIC,
                    ver: VERSION_2 - 1,
                    atype: AccountType::Price as u32,
                    ptype: PriceType::Price,
                    expo: 10,
                    agg: PriceInfo {
                        price: 10,
                        conf: 1,
                        status: PriceStatus::Trading,
                        corp_act: CorpAction::NoCorpAct,
                        pub_slot: 0
                    },
                    ..PriceAccount::default()
                },
                clock: Clock {
                    unix_timestamp: 0,
                    ..Clock::default()
                },
                expected_result: Err(ProgramError::Custom(1)),
            }),
            // case 4: failure. bad account type
            Just(PythPriceTestCase {
                price_account: PriceAccount {
                    magic: MAGIC,
                    ver: VERSION_2,
                    atype: AccountType::Product as u32,
                    ptype: PriceType::Price,
                    expo: 10,
                    agg: PriceInfo {
                        price: 10,
                        conf: 1,
                        status: PriceStatus::Trading,
                        corp_act: CorpAction::NoCorpAct,
                        pub_slot: 0
                    },
                    ..PriceAccount::default()
                },
                clock: Clock {
                    unix_timestamp: 0,
                    ..Clock::default()
                },
                expected_result: Err(ProgramError::Custom(2)),
            }),
            // case 5: ignore. bad price type is fine. not testing this
            // case 6: success. most recent price has status == trading, not stale
            Just(PythPriceTestCase {
                price_account: PriceAccount {
                    magic: MAGIC,
                    ver: VERSION_2,
                    atype: AccountType::Price as u32,
                    ptype: PriceType::Price,
                    expo: 1,
                    timestamp: 0,
                    agg: PriceInfo {
                        price: 200,
                        conf: 1,
                        status: PriceStatus::Trading,
                        corp_act: CorpAction::NoCorpAct,
                        pub_slot: 0
                    },
                    ..PriceAccount::default()
                },
                clock: Clock {
                    unix_timestamp: 120 - 1,
                    ..Clock::default()
                },
                expected_result: Ok(Decimal::from(2000_u64))
            }),
            // case 7: success. most recent price has status == unknown, previous price not stale
            Just(PythPriceTestCase {
                price_account: PriceAccount {
                    magic: MAGIC,
                    ver: VERSION_2,
                    atype: AccountType::Price as u32,
                    ptype: PriceType::Price,
                    expo: 1,
                    timestamp: 20,
                    agg: PriceInfo {
                        price: 200,
                        conf: 1,
                        status: PriceStatus::Unknown,
                        corp_act: CorpAction::NoCorpAct,
                        pub_slot: 0
                    },
                    prev_price: 190,
                    prev_conf: 10,
                    prev_timestamp: 5,
                    ..PriceAccount::default()
                },
                clock: Clock {
                    unix_timestamp: 125 - 1,
                    ..Clock::default()
                },
                expected_result: Ok(Decimal::from(1900_u64))
            }),
            // case 8: failure. most recent price has status == trading and is stale
            Just(PythPriceTestCase {
                price_account: PriceAccount {
                    magic: MAGIC,
                    ver: VERSION_2,
                    atype: AccountType::Price as u32,
                    ptype: PriceType::Price,
                    expo: 1,
                    timestamp: 0,
                    agg: PriceInfo {
                        price: 200,
                        conf: 1,
                        status: PriceStatus::Trading,
                        corp_act: CorpAction::NoCorpAct,
                        pub_slot: 0
                    },
                    ..PriceAccount::default()
                },
                clock: Clock {
                    unix_timestamp: 121,
                    ..Clock::default()
                },
                expected_result: Err(LendingError::InvalidOracleConfig.into())
            }),
            // case 9: failure. most recent price has status == unknown and previous price is stale
            Just(PythPriceTestCase {
                price_account: PriceAccount {
                    magic: MAGIC,
                    ver: VERSION_2,
                    atype: AccountType::Price as u32,
                    ptype: PriceType::Price,
                    expo: 1,
                    timestamp: 1,
                    agg: PriceInfo {
                        price: 200,
                        conf: 1,
                        status: PriceStatus::Unknown,
                        corp_act: CorpAction::NoCorpAct,
                        pub_slot: 0
                    },
                    prev_price: 190,
                    prev_conf: 10,
                    prev_timestamp: 0,
                    ..PriceAccount::default()
                },
                clock: Clock {
                    unix_timestamp: 241,
                    ..Clock::default()
                },
                expected_result: Err(LendingError::InvalidOracleConfig.into())
            }),
            // case 10: failure. price is negative
            Just(PythPriceTestCase {
                price_account: PriceAccount {
                    magic: MAGIC,
                    ver: VERSION_2,
                    atype: AccountType::Price as u32,
                    ptype: PriceType::Price,
                    expo: 1,
                    timestamp: 1,
                    agg: PriceInfo {
                        price: -200,
                        conf: 1,
                        status: PriceStatus::Trading,
                        corp_act: CorpAction::NoCorpAct,
                        pub_slot: 0
                    },
                    ..PriceAccount::default()
                },
                clock: Clock {
                    unix_timestamp: 230,
                    ..Clock::default()
                },
                expected_result: Err(LendingError::InvalidOracleConfig.into())
            }),
            // case 11: failure. confidence interval is too wide
            Just(PythPriceTestCase {
                price_account: PriceAccount {
                    magic: MAGIC,
                    ver: VERSION_2,
                    atype: AccountType::Price as u32,
                    ptype: PriceType::Price,
                    expo: 1,
                    timestamp: 1,
                    agg: PriceInfo {
                        price: 200,
                        conf: 40,
                        status: PriceStatus::Trading,
                        corp_act: CorpAction::NoCorpAct,
                        pub_slot: 0
                    },
                    ..PriceAccount::default()
                },
                clock: Clock {
                    unix_timestamp: 230,
                    ..Clock::default()
                },
                expected_result: Err(LendingError::InvalidOracleConfig.into())
            }),
        ]
    }

    proptest! {
        #[test]
        fn test_pyth_price(mut test_case in pyth_price_cases()) {
            // wrap price account into an account info
            let mut lamports = 20;
            let pubkey = Pubkey::new_unique();
            let account_info = AccountInfo::new(
                &pubkey,
                false,
                false,
                &mut lamports,
                bytes_of_mut(&mut test_case.price_account),
                &pubkey,
                false,
                0,
            );

            let result = get_pyth_price(&account_info, &test_case.clock);
            assert_eq!(
                result,
                test_case.expected_result,
                "actual: {:#?} expected: {:#?}",
                result,
                test_case.expected_result
            );
        }
    }
}

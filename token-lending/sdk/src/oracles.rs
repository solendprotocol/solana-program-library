#![allow(missing_docs)]
use crate::{
    self as solend_program,
    error::LendingError,
    math::{Decimal, TryDiv, TryMul},
    pyth_mainnet, pyth_pull_mainnet, solana_program, switchboard_on_demand_mainnet,
    switchboard_v2_mainnet,
};
use anchor_lang::Key;

use anchor_lang::AccountDeserialize;

use pyth_solana_receiver_sdk::price_update::{PriceUpdateV2, VerificationLevel};
use solana_program::{
    account_info::AccountInfo, msg, program_error::ProgramError, sysvar::clock::Clock,
};
use std::{convert::TryInto, result::Result};

const PYTH_CONFIDENCE_RATIO: u64 = 10;
const STALE_AFTER_SLOTS_ELAPSED: u64 = 240; // roughly 2 min
const STALE_AFTER_SECONDS_ELAPSED: u64 = 120; // roughly 2 min

pub enum OracleType {
    Pyth,
    Switchboard,
    PythPull,
    SbOnDemand,
}

pub fn get_oracle_type(oracle_info: &AccountInfo) -> Result<OracleType, ProgramError> {
    if *oracle_info.owner == pyth_mainnet::id() {
        return Ok(OracleType::Pyth);
    } else if *oracle_info.owner == pyth_pull_mainnet::id() {
        return Ok(OracleType::PythPull);
    } else if *oracle_info.owner == switchboard_v2_mainnet::id() {
        return Ok(OracleType::Switchboard);
    } else if *oracle_info.owner == switchboard_on_demand_mainnet::id() {
        return Ok(OracleType::SbOnDemand);
    }

    msg!(
        "Could not find oracle type for {:?} with owner {:?}",
        oracle_info.key,
        oracle_info.owner
    );
    Err(LendingError::InvalidOracleConfig.into())
}

pub fn validate_pyth_price_account_info(pyth_price_info: &AccountInfo) -> Result<(), ProgramError> {
    if *pyth_price_info.owner != pyth_mainnet::id() {
        msg!("pyth price account is not owned by pyth program");
        return Err(ProgramError::IncorrectProgramId);
    }

    let data = &pyth_price_info.try_borrow_data()?;
    pyth_sdk_solana::state::load_price_account(data).map_err(|e| {
        msg!("Couldn't load price feed from account info: {:?}", e);
        LendingError::InvalidOracleConfig
    })?;

    Ok(())
}

pub fn validate_pyth_pull_price_account_info(
    pyth_price_info: &AccountInfo,
) -> Result<(), ProgramError> {
    if *pyth_price_info.owner != pyth_pull_mainnet::id() {
        msg!("pyth price account is not owned by pyth program");
        return Err(ProgramError::IncorrectProgramId);
    }

    let _price_feed_account: PriceUpdateV2 = account_deserialize(pyth_price_info).map_err(|e| {
        msg!("Couldn't load price feed from account info: {:?}", e);
        LendingError::InvalidOracleConfig
    })?;

    Ok(())
}

/// get pyth price without caring about staleness or variance. only used
pub fn get_pyth_price_unchecked(pyth_price_info: &AccountInfo) -> Result<Decimal, ProgramError> {
    if *pyth_price_info.key == solend_program::NULL_PUBKEY {
        return Err(LendingError::NullOracleConfig.into());
    }

    let data = &pyth_price_info.try_borrow_data()?;
    let price_account = pyth_sdk_solana::state::load_price_account(data).map_err(|e| {
        msg!("Couldn't load price feed from account info: {:?}", e);
        LendingError::InvalidOracleConfig
    })?;

    let price_feed = price_account.to_price_feed(pyth_price_info.key);
    let price = price_feed.get_price_unchecked();
    pyth_price_to_decimal(&price)
}

pub fn get_pyth_pull_price_unchecked(
    pyth_price_info: &AccountInfo,
) -> Result<Decimal, ProgramError> {
    if *pyth_price_info.owner != pyth_pull_mainnet::id() {
        msg!("pyth price account is not owned by pyth program");
        return Err(ProgramError::IncorrectProgramId);
    }

    let price_feed_account: PriceUpdateV2 = account_deserialize(pyth_price_info)?;
    // let data = &pyth_price_info.data.borrow()[..];
    // let price_feed_account: PriceUpdateV2 = PriceUpdateV2::try_from_slice(data).map_err(|e| {
    //     msg!("Couldn't load price feed from account info: {:?}", e);
    //     LendingError::InvalidOracleConfig
    // })?;

    let price = price_feed_account
        .get_price_unchecked(&price_feed_account.price_message.feed_id)
        .map_err(|e| {
            msg!("Couldn't load price feed from account info: {:?}", e);
            LendingError::InvalidOracleConfig
        })?;
    pyth_pull_price_to_decimal(&price)
}

pub fn get_pyth_price(
    pyth_price_info: &AccountInfo,
    clock: &Clock,
) -> Result<(Decimal, Decimal), ProgramError> {
    if *pyth_price_info.key == solend_program::NULL_PUBKEY {
        return Err(LendingError::NullOracleConfig.into());
    }

    let data = &pyth_price_info.try_borrow_data()?;
    let price_account = pyth_sdk_solana::state::load_price_account(data).map_err(|e| {
        msg!("Couldn't load price feed from account info: {:?}", e);
        LendingError::InvalidOracleConfig
    })?;
    let pyth_price = price_account
        .get_price_no_older_than(clock, STALE_AFTER_SLOTS_ELAPSED)
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
    if pyth_price.conf.saturating_mul(PYTH_CONFIDENCE_RATIO) > price {
        msg!(
            "Oracle price confidence is too wide. price: {}, conf: {}",
            price,
            pyth_price.conf,
        );
        return Err(LendingError::InvalidOracleConfig.into());
    }

    let market_price = pyth_price_to_decimal(&pyth_price);
    let ema_price = {
        let price_feed = price_account.to_price_feed(pyth_price_info.key);
        // this can be unchecked bc the ema price is only used to _limit_ borrows and withdraws.
        // ie staleness doesn't _really_ matter for this field.
        //
        // the pyth EMA is also updated every time the regular spot price is updated anyways so in
        // reality the staleness should never be an issue.
        let ema_price = price_feed.get_ema_price_unchecked();
        pyth_price_to_decimal(&ema_price)?
    };

    Ok((market_price?, ema_price))
}

pub fn account_deserialize<T: AccountDeserialize>(
    account: &AccountInfo<'_>,
) -> Result<T, ProgramError> {
    let data = account.clone().data.borrow().to_owned();
    let mut data: &[u8] = &data;

    let user: T = T::try_deserialize(&mut data).map_err(|_| {
        msg!("Account {:?} deserialization failed", account.key());
        LendingError::InvalidAccountInput
    })?;

    Ok(user)
}

pub fn get_pyth_pull_price(
    pyth_price_info: &AccountInfo,
    clock: &Clock,
) -> Result<(Decimal, Decimal), ProgramError> {
    if *pyth_price_info.key == solend_program::NULL_PUBKEY {
        return Err(LendingError::NullOracleConfig.into());
    }

    let price_feed_account: PriceUpdateV2 = account_deserialize(pyth_price_info)?;

    let pyth_price = price_feed_account
        .get_price_no_older_than_with_custom_verification_level(
            clock,
            STALE_AFTER_SECONDS_ELAPSED, // MAXIMUM_AGE, // this should be filtered by the caller
            &price_feed_account.price_message.feed_id,
            VerificationLevel::Full, // All our prices and the sponsored feeds are full verified
        )
        .map_err(|e| {
            msg!("Pyth oracle price is likley too stale! error: {:?}", e);
            LendingError::InvalidOracleConfig
        })?;

    let price: u64 = pyth_price.price.try_into().map_err(|_| {
        msg!("Oracle price cannot be negative");
        LendingError::InvalidOracleConfig
    })?;

    // Perhaps confidence_ratio should exist as a per reserve config
    // 100/confidence_ratio = maximum size of confidence range as a percent of price
    // confidence_ratio of 10 filters out pyth prices with conf > 10% of price
    if pyth_price.conf.saturating_mul(PYTH_CONFIDENCE_RATIO) > price {
        msg!(
            "Oracle price confidence is too wide. price: {}, conf: {}",
            price,
            pyth_price.conf,
        );
        return Err(LendingError::InvalidOracleConfig.into());
    }

    let market_price = pyth_pull_price_to_decimal(&pyth_price)?;

    let ema_price = {
        let ema_price = pyth_solana_receiver_sdk::price_update::Price {
            price: price_feed_account.price_message.ema_price,
            conf: price_feed_account.price_message.ema_conf,
            exponent: price_feed_account.price_message.exponent,
            publish_time: price_feed_account.price_message.publish_time,
        };
        pyth_pull_price_to_decimal(&ema_price)?
    };

    Ok((market_price, ema_price))
}

fn pyth_price_to_decimal(pyth_price: &pyth_sdk_solana::Price) -> Result<Decimal, ProgramError> {
    let price: u64 = pyth_price.price.try_into().map_err(|_| {
        msg!("Oracle price cannot be negative");
        LendingError::InvalidOracleConfig
    })?;

    if pyth_price.expo >= 0 {
        let exponent = pyth_price
            .expo
            .try_into()
            .map_err(|_| LendingError::MathOverflow)?;
        let zeros = 10u64
            .checked_pow(exponent)
            .ok_or(LendingError::MathOverflow)?;
        Decimal::from(price).try_mul(zeros)
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
        Decimal::from(price).try_div(decimals)
    }
}

fn pyth_pull_price_to_decimal(
    pyth_price: &pyth_solana_receiver_sdk::price_update::Price,
) -> Result<Decimal, ProgramError> {
    let price: u64 = pyth_price.price.try_into().map_err(|_| {
        msg!("Oracle price cannot be negative");
        LendingError::InvalidOracleConfig
    })?;

    if pyth_price.exponent >= 0 {
        let exponent = pyth_price
            .exponent
            .try_into()
            .map_err(|_| LendingError::MathOverflow)?;
        let zeros = 10u64
            .checked_pow(exponent)
            .ok_or(LendingError::MathOverflow)?;
        Decimal::from(price).try_mul(zeros)
    } else {
        let exponent = pyth_price
            .exponent
            .checked_abs()
            .ok_or(LendingError::MathOverflow)?
            .try_into()
            .map_err(|_| LendingError::MathOverflow)?;
        let decimals = 10u64
            .checked_pow(exponent)
            .ok_or(LendingError::MathOverflow)?;
        Decimal::from(price).try_div(decimals)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use bytemuck::bytes_of_mut;
    use proptest::prelude::*;
    use pyth_sdk_solana::state::Rational;
    use pyth_sdk_solana::state::{
        AccountType, CorpAction, PriceAccount, PriceInfo, PriceStatus, PriceType, MAGIC, VERSION_2,
    };
    use solana_program::pubkey::Pubkey;
    use std::fs::File;
    use std::io::Read;
    use std::path::Path;
    use std::str::FromStr;

    #[derive(Clone, Debug)]
    struct PythPriceTestCase {
        price_account: PriceAccount,
        clock: Clock,
        expected_result: Result<(Decimal, Decimal), ProgramError>,
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
                    ema_price: Rational {
                        val: 11,
                        numer: 110,
                        denom: 10,
                    },
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
                    slot: 4,
                    ..Clock::default()
                },
                // PythError::InvalidAccountData.
                expected_result: Err(LendingError::InvalidOracleConfig.into()),
            }),
            // case 3: failure. bad version number
            Just(PythPriceTestCase {
                price_account: PriceAccount {
                    magic: MAGIC,
                    ver: VERSION_2 - 1,
                    atype: AccountType::Price as u32,
                    ptype: PriceType::Price,
                    expo: 10,
                    ema_price: Rational {
                        val: 11,
                        numer: 110,
                        denom: 10,
                    },
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
                    slot: 4,
                    ..Clock::default()
                },
                expected_result: Err(LendingError::InvalidOracleConfig.into()),
            }),
            // case 4: failure. bad account type
            Just(PythPriceTestCase {
                price_account: PriceAccount {
                    magic: MAGIC,
                    ver: VERSION_2,
                    atype: AccountType::Product as u32,
                    ptype: PriceType::Price,
                    expo: 10,
                    ema_price: Rational {
                        val: 11,
                        numer: 110,
                        denom: 10,
                    },
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
                    slot: 4,
                    ..Clock::default()
                },
                expected_result: Err(LendingError::InvalidOracleConfig.into()),
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
                    ema_price: Rational {
                        val: 11,
                        numer: 110,
                        denom: 10,
                    },
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
                    slot: 240,
                    ..Clock::default()
                },
                expected_result: Ok((Decimal::from(2000_u64), Decimal::from(110_u64)))
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
                    ema_price: Rational {
                        val: 11,
                        numer: 110,
                        denom: 10,
                    },
                    agg: PriceInfo {
                        price: 200,
                        conf: 1,
                        status: PriceStatus::Unknown,
                        corp_act: CorpAction::NoCorpAct,
                        pub_slot: 1
                    },
                    prev_price: 190,
                    prev_conf: 10,
                    prev_slot: 0,
                    ..PriceAccount::default()
                },
                clock: Clock {
                    slot: 240,
                    ..Clock::default()
                },
                expected_result: Ok((Decimal::from(1900_u64), Decimal::from(110_u64)))
            }),
            // case 8: failure. most recent price is stale
            Just(PythPriceTestCase {
                price_account: PriceAccount {
                    magic: MAGIC,
                    ver: VERSION_2,
                    atype: AccountType::Price as u32,
                    ptype: PriceType::Price,
                    expo: 1,
                    timestamp: 0,
                    ema_price: Rational {
                        val: 11,
                        numer: 110,
                        denom: 10,
                    },
                    agg: PriceInfo {
                        price: 200,
                        conf: 1,
                        status: PriceStatus::Trading,
                        corp_act: CorpAction::NoCorpAct,
                        pub_slot: 1
                    },
                    prev_slot: 0, // there is no case where prev_slot > agg.pub_slot
                    ..PriceAccount::default()
                },
                clock: Clock {
                    slot: 242,
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
                    ema_price: Rational {
                        val: 11,
                        numer: 110,
                        denom: 10,
                    },
                    agg: PriceInfo {
                        price: 200,
                        conf: 1,
                        status: PriceStatus::Unknown,
                        corp_act: CorpAction::NoCorpAct,
                        pub_slot: 1
                    },
                    prev_price: 190,
                    prev_conf: 10,
                    prev_slot: 0,
                    ..PriceAccount::default()
                },
                clock: Clock {
                    slot: 241,
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
                    ema_price: Rational {
                        val: 11,
                        numer: 110,
                        denom: 10,
                    },
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
                    slot: 240,
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
                    ema_price: Rational {
                        val: 11,
                        numer: 110,
                        denom: 10,
                    },
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
                    slot: 240,
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

    #[test]
    fn pyth_price_unchecked_test_cases() {
        let mut price_account = PriceAccount {
            magic: MAGIC,
            ver: VERSION_2,
            atype: AccountType::Price as u32,
            ptype: PriceType::Price,
            expo: 1,
            timestamp: 1,
            ema_price: Rational {
                val: 11,
                numer: 110,
                denom: 10,
            },
            agg: PriceInfo {
                price: 200,
                conf: 40,
                status: PriceStatus::Trading,
                corp_act: CorpAction::NoCorpAct,
                pub_slot: 0,
            },
            ..PriceAccount::default()
        };

        let mut lamports = 20;
        let pubkey = Pubkey::new_unique();
        let account_info = AccountInfo::new(
            &pubkey,
            false,
            false,
            &mut lamports,
            bytes_of_mut(&mut price_account),
            &pubkey,
            false,
            0,
        );

        assert_eq!(
            get_pyth_price_unchecked(&account_info),
            Ok(Decimal::from(2000_u64))
        );
    }

    fn read_file<P: AsRef<Path>>(path: P) -> Vec<u8> {
        let path = path.as_ref();
        let mut file = File::open(path)
            .unwrap_or_else(|err| panic!("Failed to open \"{}\": {}", path.display(), err));

        let mut file_data = Vec::new();
        file.read_to_end(&mut file_data)
            .unwrap_or_else(|err| panic!("Failed to read \"{}\": {}", path.display(), err));
        file_data
    }

    #[test]
    fn test_pyth_pull_price() {
        let mut price_account_data =
            read_file("fixtures/7UVimffxr9ow1uXYxsr4LHAcV58mLzhmwaeKvJ1pjLiE.bin");
        // println!("data {:?}", price_account_data);
        // let price_account: PriceUpdateV2 =
        //     PriceUpdateV2::try_from_slice(&price_account_data.clone()).unwrap();
        // println!("{:#?}", price_account.price_message);

        let mut lamports = 20;
        let pubkey = Pubkey::new_unique();
        let owner = Pubkey::from_str("rec5EKMGg6MxZYaMdyBfgwp4d5rB9T1VQH5pJv5LtFJ").unwrap();
        let account_info = AccountInfo::new(
            &pubkey,
            false,
            false,
            &mut lamports,
            &mut price_account_data,
            &owner,
            false,
            0,
        );

        let price = Decimal::from(135426693_u64)
            .try_div(Decimal::from(1000000_u64))
            .unwrap();
        let ema_price = Decimal::from(134522707_u64)
            .try_div(Decimal::from(1000000_u64))
            .unwrap();
        assert_eq!(get_pyth_pull_price_unchecked(&account_info).unwrap(), price);

        let clock = Clock {
            slot: 240,
            ..Clock::default()
        };
        assert_eq!(
            get_pyth_pull_price(&account_info, &clock).unwrap(),
            (price, ema_price)
        );
    }
}

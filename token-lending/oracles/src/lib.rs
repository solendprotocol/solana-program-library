pub mod pyth;
pub mod switchboard;

use crate::pyth::get_pyth_price_unchecked;
use crate::pyth::get_pyth_pull_price;
use crate::pyth::get_pyth_pull_price_unchecked;
use crate::switchboard::get_switchboard_price;
use crate::switchboard::get_switchboard_price_on_demand;
use crate::switchboard::get_switchboard_price_v2;
use solana_program::{
    account_info::AccountInfo, msg, program_error::ProgramError, sysvar::clock::Clock,
};
use solend_sdk::error::LendingError;
use solend_sdk::math::Decimal;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
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

pub fn get_single_price(
    oracle_account_info: &AccountInfo,
    clock: &Clock,
) -> Result<(Decimal, Option<Decimal>), ProgramError> {
    match get_oracle_type(oracle_account_info)? {
        OracleType::Pyth => {
            let price = pyth::get_pyth_price(oracle_account_info, clock)?;
            Ok((price.0, Some(price.1)))
        }
        OracleType::PythPull => {
            let price = get_pyth_pull_price(oracle_account_info, clock)?;
            Ok((price.0, Some(price.1)))
        }
        OracleType::Switchboard => {
            let price = get_switchboard_price(oracle_account_info, clock)?;
            Ok((price, None))
        }
        OracleType::SbOnDemand => {
            let price = get_switchboard_price(oracle_account_info, clock)?;
            Ok((price, None))
        }
    }
}

pub fn get_single_price_unchecked(
    oracle_account_info: &AccountInfo,
    clock: &Clock,
) -> Result<Decimal, ProgramError> {
    match get_oracle_type(oracle_account_info)? {
        OracleType::Pyth => get_pyth_price_unchecked(oracle_account_info),
        OracleType::PythPull => get_pyth_pull_price_unchecked(oracle_account_info),
        OracleType::Switchboard => get_switchboard_price_v2(oracle_account_info, clock, false),
        OracleType::SbOnDemand => get_switchboard_price_on_demand(oracle_account_info, clock, true),
    }
}

/// Mainnet program id for Switchboard v2.
pub mod switchboard_v2_mainnet {
    solana_program::declare_id!("SW1TCH7qEPTdLsDHRgPuMQjbQxKdH2aBStViMFnt64f");
}

/// Devnet program id for Switchboard v2.
pub mod switchboard_v2_devnet {
    solana_program::declare_id!("2TfB33aLaneQb5TNVwyDz3jSZXS6jdW2ARw1Dgf84XCG");
}

/// Mainnet program id for Switchboard On-Demand Oracle.
pub mod switchboard_on_demand_mainnet {
    solana_program::declare_id!("SBondMDrcV3K4kxZR1HNVT7osZxAHVHgYXL5Ze1oMUv");
}

/// Devnet program id for Switchboard On-Demand Oracle.
pub mod switchboard_on_demand_devnet {
    solana_program::declare_id!("SBondMDrcV3K4kxZR1HNVT7osZxAHVHgYXL5Ze1oMUv");
}

/// Mainnet program id for pyth
pub mod pyth_mainnet {
    solana_program::declare_id!("FsJ3A3u2vn5cTVofAjvy6y5kwABJAqYWpe4975bi2epH");
}

/// Mainnet program id for pyth
pub mod pyth_pull_mainnet {
    solana_program::declare_id!("rec5EKMGg6MxZYaMdyBfgwp4d5rB9T1VQH5pJv5LtFJ");
}

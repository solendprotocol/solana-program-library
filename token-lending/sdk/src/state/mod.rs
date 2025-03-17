//! State types

mod last_update;
mod lending_market;
mod lending_market_metadata;
mod liquidity_mining;
mod obligation;
mod rate_limiter;
mod reserve;

pub use last_update::*;
pub use lending_market::*;
pub use lending_market_metadata::*;
pub use liquidity_mining::*;
pub use obligation::*;
use program_version::ProgramVersion;
pub use rate_limiter::*;
pub use reserve::*;

use crate::math::{Decimal, WAD};
use discriminator::AccountDiscriminator;
use solana_program::{msg, program_error::ProgramError};

/// Collateral tokens are initially valued at a ratio of 5:1 (collateral:liquidity)
// @FIXME: restore to 5
pub const INITIAL_COLLATERAL_RATIO: u64 = 1;
const INITIAL_COLLATERAL_RATE: u64 = INITIAL_COLLATERAL_RATIO * WAD;

/// Number of slots per year
// 2 (slots per second) * 60 * 60 * 24 * 365 = 63072000
pub const SLOTS_PER_YEAR: u64 = 63072000;

pub mod program_version {
    //! There can be at the moment at most 16 different program versions.
    //! Extrapolating from the current program history this should be good enough.
    //! The program versions can also wrap if sufficient precautions are taken.

    /// Match the second 4 bits of an account data against this enum to determine
    /// the program version.
    pub enum ProgramVersion {
        /// Account is not initialized yet.
        Uninitialized = 0,
        /// Version of the program and all new accounts created until inclusive version
        /// @v2.0.2
        ///
        /// These versions will have no account discriminator.
        V2_0_2 = 1,
        /// Version of the program and all new accounts created from inclusive version
        /// @v2.1.0 (liquidity mining)
        ///
        /// Will have an associated account discriminator.
        V2_1_0 = 2,
    }
}

pub mod discriminator {
    //! First 4 bits determine the account kind.
    //!
    //! There can be at the moment at most 15 different discriminators.
    //! Extrapolating from the current program history this should be good enough.

    /// Match the first 4 bits of an account data against this enum to determine
    /// the account type.
    pub enum AccountDiscriminator {
        /// Account is not initialized yet.
        Uninitialized = 0,
        /// [crate::state::LendingMarket]
        LendingMarket = 1,
        /// [crate::state::Reserve]
        Reserve = 2,
        /// [crate::state::Obligation]
        Obligation = 3,
    }
}

/// There can be at the moment at most 16 different program versions.
/// Extrapolating from the current program history this should be good enough.
/// The program versions can also wrap if sufficient precautions are taken.
pub fn set_discriminator_and_version(
    discriminator: AccountDiscriminator,
    version: ProgramVersion,
) -> u8 {
    let discriminator = discriminator as u8;
    debug_assert!(discriminator <= 0x0F);
    let version = version as u8;
    debug_assert!(version <= 0x0F);

    (discriminator << 4) | (version & 0x0F)
}

/// First 4 bytes are the discriminator, next 4 bytes are the version.
pub fn extract_discriminator_and_version(
    byte: u8,
) -> Result<(AccountDiscriminator, ProgramVersion), ProgramError> {
    let version = match byte & 0x0F {
        0 => ProgramVersion::Uninitialized,
        1 => ProgramVersion::V2_0_2,
        2 => ProgramVersion::V2_1_0,
        3..=16 => {
            // unused
            return Err(ProgramError::InvalidAccountData);
        }
        _ => unreachable!("Version is out of bounds"),
    };

    let discriminator = match (byte >> 4) & 0x0F {
        0 => AccountDiscriminator::Uninitialized,
        1 => AccountDiscriminator::LendingMarket,
        2 => AccountDiscriminator::Reserve,
        3 => AccountDiscriminator::Obligation,
        4..=16 => {
            // unused
            return Err(ProgramError::InvalidAccountData);
        }
        17.. => unreachable!("Discriminator is out of bounds"),
    };

    Ok((discriminator, version))
}

// Helpers
fn pack_decimal(decimal: Decimal, dst: &mut [u8; 16]) {
    *dst = decimal
        .to_scaled_val()
        .expect("Decimal cannot be packed")
        .to_le_bytes();
}

fn unpack_decimal(src: &[u8; 16]) -> Decimal {
    Decimal::from_scaled_val(u128::from_le_bytes(*src))
}

fn pack_bool(boolean: bool, dst: &mut [u8; 1]) {
    *dst = (boolean as u8).to_le_bytes()
}

fn unpack_bool(src: &[u8; 1]) -> Result<bool, ProgramError> {
    match u8::from_le_bytes(*src) {
        0 => Ok(false),
        1 => Ok(true),
        _ => {
            msg!("Boolean cannot be unpacked");
            Err(ProgramError::InvalidAccountData)
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn initial_collateral_rate_sanity() {
        assert_eq!(
            INITIAL_COLLATERAL_RATIO.checked_mul(WAD).unwrap(),
            INITIAL_COLLATERAL_RATE
        );
    }
}

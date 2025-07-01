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

/// Unmigrated accounts have this as their leading byte.
pub const PROGRAM_VERSION_2_0_2: u8 = 1;

pub mod discriminator {
    //! First 1 byte determines the account kind.

    use std::convert::TryFrom;

    use crate::error::LendingError;

    /// Match the first byte of an account data against this enum to determine
    /// the account type.
    ///
    /// # Note
    ///
    /// In versions before @v2.1.0 this byte represented program version.
    /// That's why we skip value `1u8`.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum AccountDiscriminator {
        /// Account is not initialized yet.
        #[default]
        Uninitialized = 0,
        /// [crate::state::LendingMarket]
        LendingMarket = 2,
        /// [crate::state::Reserve]
        Reserve = 3,
        /// [crate::state::Obligation]
        Obligation = 4,
    }

    impl TryFrom<u8> for AccountDiscriminator {
        type Error = LendingError;

        fn try_from(value: u8) -> Result<Self, Self::Error> {
            match value {
                // the account data were just created and are filled with 0s
                0 => Ok(Self::Uninitialized),

                // we skip 1 because it was used for program version
                1 => Err(Self::Error::AccountNotMigrated),

                // valid accounts
                2 => Ok(Self::LendingMarket),
                3 => Ok(Self::Reserve),
                4 => Ok(Self::Obligation),

                _ => Err(Self::Error::InvalidAccountDiscriminator),
            }
        }
    }

    impl TryFrom<&[u8; 1]> for AccountDiscriminator {
        type Error = LendingError;

        fn try_from(value: &[u8; 1]) -> Result<Self, Self::Error> {
            Self::try_from(value[0])
        }
    }
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

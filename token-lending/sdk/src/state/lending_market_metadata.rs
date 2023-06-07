use super::*;

use crate::error::LendingError;
use bytemuck::checked::try_from_bytes;
use bytemuck::{Pod, Zeroable};
use solana_program::program_error::ProgramError;
use solana_program::pubkey::Pubkey;
use static_assertions::{assert_eq_size, const_assert};

/// market name size
pub const MARKET_NAME_SIZE: usize = 50;

/// market description size
pub const MARKET_DESCRIPTION_SIZE: usize = 300;

/// market image url size
pub const MARKET_IMAGE_URL_SIZE: usize = 250;

/// padding size
pub const PADDING_SIZE: usize = 100;

/// Lending market state
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct LendingMarketMetadata {
    /// Bump seed
    pub bump_seed: u8,
    /// Market name null padded
    pub market_name: [u8; MARKET_NAME_SIZE],
    /// Market description null padded
    pub market_description: [u8; MARKET_DESCRIPTION_SIZE],
    /// Market image url
    pub market_image_url: [u8; MARKET_IMAGE_URL_SIZE],
    /// Lookup Tables
    pub lookup_tables: [Pubkey; 4],
    /// Padding
    pub padding: [u8; PADDING_SIZE],
}

impl LendingMarketMetadata {
    /// Create a LendingMarketMetadata referernce from a slice
    pub fn new_from_bytes(data: &[u8]) -> Result<&LendingMarketMetadata, ProgramError> {
        try_from_bytes::<LendingMarketMetadata>(&data[1..]).map_err(|_| {
            msg!("Failed to deserialize LendingMarketMetadata");
            LendingError::InstructionUnpackError.into()
        })
    }
}

unsafe impl Zeroable for LendingMarketMetadata {}
unsafe impl Pod for LendingMarketMetadata {}

assert_eq_size!(
    LendingMarketMetadata,
    [u8; MARKET_NAME_SIZE
        + MARKET_DESCRIPTION_SIZE
        + MARKET_IMAGE_URL_SIZE
        + 4 * 32
        + PADDING_SIZE
        + 1],
);

// transaction size limit check
const_assert!(std::mem::size_of::<LendingMarketMetadata>() <= 850);

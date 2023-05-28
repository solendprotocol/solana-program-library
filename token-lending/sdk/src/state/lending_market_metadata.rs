use super::*;
use arrayref::{array_mut_ref, array_ref, array_refs, mut_array_refs};
use solana_program::{
    program_error::ProgramError,
    program_pack::{IsInitialized, Pack, Sealed},
    pubkey::{Pubkey, PUBKEY_BYTES},
};

/// market name size
pub const MARKET_NAME_SIZE: usize = 50;

/// market description size
pub const MARKET_DESCRIPTION_SIZE: usize = 50;

/// market image url size
pub const MARKET_IMAGE_URL_SIZE: usize = 50;

/// Lending market state
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LendingMarketMetadata {
    /// Version of lending market metadata
    pub version: u8,
    /// Market address
    pub market_address: Pubkey,
    /// Market name null padded
    pub market_name: [u8; MARKET_NAME_SIZE],
    /// Market description null padded
    pub market_description: [u8; MARKET_DESCRIPTION_SIZE],
    /// Market image url
    pub market_image_url: [u8; MARKET_IMAGE_URL_SIZE],
}

impl LendingMarketMetadata {
    /// Create a new lending market metadata
    pub fn new(params: InitLendingMarketMetadataParams) -> Self {
        Self {
            version: PROGRAM_VERSION,
            market_address: params.market_address,
            market_name: params.market_name,
            market_description: params.market_description,
            market_image_url: params.market_image_url,
        }
    }

    /// Initialize a lending market metadata
    pub fn init(&mut self, params: InitLendingMarketMetadataParams) {
        self.version = PROGRAM_VERSION;
        self.market_address = params.market_address;
        self.market_name = params.market_name;
        self.market_description = params.market_description;
        self.market_image_url = params.market_image_url;
    }
}

/// Initialize a lending market metadata
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InitLendingMarketMetadataParams {
    /// Bump seed for derived authority address
    pub bump_seed: u8,
    /// Market address
    pub market_address: Pubkey,
    /// Market name null padded
    pub market_name: [u8; MARKET_NAME_SIZE],
    /// Market description null padded
    pub market_description: [u8; MARKET_DESCRIPTION_SIZE],
    /// Market image url
    pub market_image_url: [u8; MARKET_IMAGE_URL_SIZE],
}

impl Sealed for LendingMarketMetadata {}
impl IsInitialized for LendingMarketMetadata {
    fn is_initialized(&self) -> bool {
        self.version != UNINITIALIZED_VERSION
    }
}

const LENDING_MARKET_METADATA_LEN: usize =
    1 + PUBKEY_BYTES + MARKET_NAME_SIZE + MARKET_DESCRIPTION_SIZE + MARKET_IMAGE_URL_SIZE + 1000;

impl Pack for LendingMarketMetadata {
    const LEN: usize = LENDING_MARKET_METADATA_LEN;

    fn pack_into_slice(&self, output: &mut [u8]) {
        let output = array_mut_ref![output, 0, LENDING_MARKET_METADATA_LEN];
        #[allow(clippy::ptr_offset_with_cast)]
        let (version, market_address, market_name, market_description, market_image_url, _padding) = mut_array_refs![
            output,
            1,
            PUBKEY_BYTES,
            MARKET_NAME_SIZE,
            MARKET_DESCRIPTION_SIZE,
            MARKET_IMAGE_URL_SIZE,
            1000
        ];

        *version = self.version.to_le_bytes();
        market_address.copy_from_slice(self.market_address.as_ref());
        market_name.copy_from_slice(self.market_name.as_ref());
        market_description.copy_from_slice(self.market_description.as_ref());
        market_image_url.copy_from_slice(self.market_image_url.as_ref());
    }

    /// Unpacks a byte buffer into a [LendingMarketMetadataInfo](struct.LendingMarketMetadataInfo.html)
    fn unpack_from_slice(input: &[u8]) -> Result<Self, ProgramError> {
        let input = array_ref![input, 0, LENDING_MARKET_METADATA_LEN];
        #[allow(clippy::ptr_offset_with_cast)]
        let (version, market_address, market_name, market_description, market_image_url, _padding) = array_refs![
            input,
            1,
            PUBKEY_BYTES,
            MARKET_NAME_SIZE,
            MARKET_DESCRIPTION_SIZE,
            MARKET_IMAGE_URL_SIZE,
            1000
        ];

        Ok(Self {
            version: u8::from_le_bytes(*version),
            market_address: Pubkey::new_from_array(*market_address),
            market_name: *market_name,
            market_description: *market_description,
            market_image_url: *market_image_url,
        })
    }
}

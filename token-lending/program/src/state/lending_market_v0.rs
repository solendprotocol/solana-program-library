/// Old LendingMarket struct definition and serialization logic
use super::*;
use arrayref::{array_mut_ref, array_ref, array_refs, mut_array_refs};
use solana_program::{
    msg,
    program_error::ProgramError,
    program_pack::{IsInitialized, Pack, Sealed},
    pubkey::{Pubkey, PUBKEY_BYTES},
};

/// Lending market state
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LendingMarketV0 {
    /// Version of lending market
    pub version: u8,
    /// Bump seed for derived authority address
    pub bump_seed: u8,
    /// Owner authority which can add new reserves
    pub owner: Pubkey,
    /// Currency market prices are quoted in
    /// e.g. "USD" null padded (`*b"USD\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0"`) or a SPL token mint pubkey
    pub quote_currency: [u8; 32],
    /// Token program id
    pub token_program_id: Pubkey,
    /// Oracle (Pyth) program id
    pub oracle_program_id: Pubkey,
    /// Oracle (Switchboard) program id
    pub switchboard_oracle_program_id: Pubkey,
}

impl Sealed for LendingMarketV0 {}
impl IsInitialized for LendingMarketV0 {
    fn is_initialized(&self) -> bool {
        self.version != UNINITIALIZED_VERSION
    }
}

const LENDING_MARKET_LEN: usize = 290; // 1 + 1 + 32 + 32 + 32 + 32 + 32 + 128
impl Pack for LendingMarketV0 {
    const LEN: usize = LENDING_MARKET_LEN;

    fn pack_into_slice(&self, output: &mut [u8]) {
        let output = array_mut_ref![output, 0, LENDING_MARKET_LEN];
        #[allow(clippy::ptr_offset_with_cast)]
        let (
            version,
            bump_seed,
            owner,
            quote_currency,
            token_program_id,
            oracle_program_id,
            switchboard_oracle_program_id,
            _padding,
        ) = mut_array_refs![
            output,
            1,
            1,
            PUBKEY_BYTES,
            32,
            PUBKEY_BYTES,
            PUBKEY_BYTES,
            PUBKEY_BYTES,
            128
        ];

        *version = self.version.to_le_bytes();
        *bump_seed = self.bump_seed.to_le_bytes();
        owner.copy_from_slice(self.owner.as_ref());
        quote_currency.copy_from_slice(self.quote_currency.as_ref());
        token_program_id.copy_from_slice(self.token_program_id.as_ref());
        oracle_program_id.copy_from_slice(self.oracle_program_id.as_ref());
        switchboard_oracle_program_id.copy_from_slice(self.switchboard_oracle_program_id.as_ref());
    }

    /// Unpacks a byte buffer into a [LendingMarketV0Info](struct.LendingMarketV0Info.html)
    fn unpack_from_slice(input: &[u8]) -> Result<Self, ProgramError> {
        let input = array_ref![input, 0, LENDING_MARKET_LEN];
        #[allow(clippy::ptr_offset_with_cast)]
        let (
            version,
            bump_seed,
            owner,
            quote_currency,
            token_program_id,
            oracle_program_id,
            switchboard_oracle_program_id,
            _padding,
        ) = array_refs![
            input,
            1,
            1,
            PUBKEY_BYTES,
            32,
            PUBKEY_BYTES,
            PUBKEY_BYTES,
            PUBKEY_BYTES,
            128
        ];

        let version = u8::from_le_bytes(*version);
        if version > PROGRAM_VERSION {
            msg!("Lending market version does not match lending program version");
            return Err(ProgramError::InvalidAccountData);
        }

        Ok(Self {
            version,
            bump_seed: u8::from_le_bytes(*bump_seed),
            owner: Pubkey::new_from_array(*owner),
            quote_currency: *quote_currency,
            token_program_id: Pubkey::new_from_array(*token_program_id),
            oracle_program_id: Pubkey::new_from_array(*oracle_program_id),
            switchboard_oracle_program_id: Pubkey::new_from_array(*switchboard_oracle_program_id),
        })
    }
}

impl From<LendingMarket> for LendingMarketV0 {
    fn from(lending_market: LendingMarket) -> Self {
        LendingMarketV0 {
            version: lending_market.version,
            bump_seed: lending_market.bump_seed,
            owner: lending_market.owner,
            quote_currency: lending_market.quote_currency,
            token_program_id: lending_market.token_program_id,
            // oracle_program_id: lending_market.oracle_program_id,
            oracle_program_id: lending_market.token_program_id,
            switchboard_oracle_program_id: lending_market.switchboard_oracle_program_id,
        }
    }
}

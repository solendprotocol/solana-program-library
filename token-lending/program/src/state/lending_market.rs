use super::{lending_market_v0::LendingMarketV0, *};
use crate::{
    error::LendingError,
    smart_pack::{AccountTag, SmartPack, ValidateTag},
};
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

/// Lending market state
#[derive(Clone, Debug, Default, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct LendingMarket {
    /// Version of lending market
    pub version: u8,
    /// Tag. Should always be AccountTag::LendingMarket. only present in version 2.
    pub tag: AccountTag,
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

impl LendingMarket {
    /// Create a new lending market
    pub fn new(params: InitLendingMarketParams) -> Self {
        let mut lending_market = Self::default();
        Self::init(&mut lending_market, params);
        lending_market
    }

    /// Initialize a lending market
    pub fn init(&mut self, params: InitLendingMarketParams) {
        self.version = PROGRAM_VERSION;
        self.tag = AccountTag::LendingMarket;
        self.bump_seed = params.bump_seed;
        self.owner = params.owner;
        self.quote_currency = params.quote_currency;
        self.token_program_id = params.token_program_id;
        self.oracle_program_id = params.oracle_program_id;
        self.switchboard_oracle_program_id = params.switchboard_oracle_program_id;
    }
}

/// Initialize a lending market
pub struct InitLendingMarketParams {
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

impl ValidateTag for LendingMarket {
    fn validate_tag(&self) -> Result<(), LendingError> {
        match self.tag {
            AccountTag::LendingMarket => Ok(()),
            _ => Err(LendingError::FailedToDeserialize),
        }
    }
}

impl SmartPack<LendingMarketV0, LendingMarket> for LendingMarket {}

impl From<LendingMarketV0> for LendingMarket {
    fn from(lending_market_v0: LendingMarketV0) -> Self {
        LendingMarket {
            version: lending_market_v0.version,
            tag: AccountTag::LendingMarket, // this field doesn't exist in V1
            bump_seed: lending_market_v0.bump_seed,
            owner: lending_market_v0.owner,
            quote_currency: lending_market_v0.quote_currency,
            token_program_id: lending_market_v0.token_program_id,
            oracle_program_id: lending_market_v0.oracle_program_id,
            switchboard_oracle_program_id: lending_market_v0.switchboard_oracle_program_id,
        }
    }
}

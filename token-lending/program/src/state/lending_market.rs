use super::{lending_market_v1::LendingMarketV1, *};
use crate::{
    error::LendingError,
    smart_pack::{AccountTag, SmartPack, ValidateTag},
};
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

/// Lending market state
#[derive(Clone, Debug, Default, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
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

impl SmartPack<LendingMarketV1, LendingMarket> for LendingMarket {}

impl From<LendingMarketV1> for LendingMarket {
    fn from(lending_market_v1: LendingMarketV1) -> Self {
        LendingMarket {
            version: 2,
            tag: AccountTag::LendingMarket, // this field doesn't exist in V1
            bump_seed: lending_market_v1.bump_seed,
            owner: lending_market_v1.owner,
            quote_currency: lending_market_v1.quote_currency,
            token_program_id: lending_market_v1.token_program_id,
            oracle_program_id: lending_market_v1.oracle_program_id,
            switchboard_oracle_program_id: lending_market_v1.switchboard_oracle_program_id,
        }
    }
}

#[cfg(test)]
mod test {
    use solana_program::pubkey::Pubkey;

    use crate::{state::LendingMarketV1, pyth, smart_pack::AccountTag};

    use super::LendingMarket;

    /* from/to LendingMarket tests */
    #[test]
    fn from_lending_market_v1() {
        let v1 = LendingMarketV1 {
            version: 2,
            bump_seed: 1,
            owner: Pubkey::new_rand(),
            quote_currency: [1; 32],
            token_program_id: spl_token::id(),
            oracle_program_id: Pubkey::new_unique(),
            switchboard_oracle_program_id: Pubkey::new_unique(),
        };

        let v1: LendingMarket = v1.clone().into();
        assert_eq!(v1.version, v1.version);
        assert_eq!(v1.tag, AccountTag::LendingMarket);
        assert_eq!(v1.bump_seed, v1.bump_seed);
        assert_eq!(v1.owner, v1.owner);
        assert_eq!(v1.quote_currency, v1.quote_currency);
        assert_eq!(v1.token_program_id, v1.token_program_id);
        assert_eq!(v1.oracle_program_id, v1.oracle_program_id);
        assert_eq!(v1.switchboard_oracle_program_id, v1.switchboard_oracle_program_id);
    }


    /* smart pack tests */
}

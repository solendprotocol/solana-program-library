use super::{lending_market_v0::LendingMarketV0, *};
use crate::{
    error::LendingError,
    smart_pack::{AccountTag, SmartPack},
};
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{
    account_info::AccountInfo, program_error::ProgramError, program_pack::Pack, pubkey::Pubkey,
};

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

impl SmartPack for LendingMarket {
    type Item = LendingMarket;

    fn version(src: &[u8]) -> u8 {
        match src.iter().next() {
            // it's ok if the data buffer is _currently_ empty because we re-allocate in smart_pack
            None => UNINITIALIZED_VERSION,
            Some(v) => *v,
        }
    }

    fn is_initialized(src: &[u8]) -> bool {
        Self::version(src) != UNINITIALIZED_VERSION
    }

    /// deserialize bytes into LendingMarket. This function can be called off-chain as well.
    fn smart_unpack(src: &[u8]) -> Result<Self, ProgramError> {
        match Self::version(src) {
            UNINITIALIZED_VERSION => {
                msg!("Can't unpack an uninitialized object!");
                Err(LendingError::FailedToDeserialize.into())
            }
            1 => Ok(LendingMarketV0::unpack(src)?.into()),
            2 => match LendingMarket::try_from_slice(src) {
                Ok(lending_market) => match lending_market.tag {
                    AccountTag::LendingMarket => Ok(lending_market),
                    tag => {
                        msg!("This account is not a lending market, it is a {:?}", tag);
                        Err(LendingError::FailedToDeserialize.into())
                    }
                },
                Err(e) => {
                    msg!("failed to borsh deserialize {:?}", e);
                    Err(LendingError::FailedToDeserialize.into())
                }
            },
            v => {
                msg!("Unimplemented version detected: {}", v);
                Err(LendingError::FailedToDeserialize.into())
            }
        }
    }

    fn smart_pack(
        mut lending_market: LendingMarket,
        dst_account_info: &AccountInfo,
    ) -> Result<(), ProgramError> {
        lending_market.version = PROGRAM_VERSION;

        match PROGRAM_VERSION {
            1 => LendingMarketV0::pack(
                lending_market.into(),
                &mut dst_account_info.try_borrow_mut_data()?,
            ),
            2 => {
                // serialize into a vector first
                let serialized = lending_market.try_to_vec().map_err(|e| {
                    msg!("failed to borsh serialize: {:?}", e);
                    LendingError::FailedToSerialize
                })?;

                // 1. always realloc because try_from_slice will error on buffer len mismatches
                // 2. zero-init out of paranoia but i don't think we actually need this
                dst_account_info.realloc(serialized.len(), true)?;

                // copy_from_slice panics if the sizes of the two slices don't match.
                // in this case, we're guaranteed to not panic because we just realloc'd the account
                let mut dst = dst_account_info.try_borrow_mut_data()?;
                dst.copy_from_slice(&serialized);

                Ok(())
            }
            v => {
                msg!("Unimplemented pack version detected: {}", v);
                Err(LendingError::FailedToSerialize.into())
            }
        }
    }
}

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

use super::*;
use arrayref::{array_mut_ref, array_ref, array_refs, mut_array_refs};
use solana_program::{
    msg,
    program_error::ProgramError,
    program_pack::{IsInitialized, Pack, Sealed},
    pubkey::{Pubkey, PUBKEY_BYTES},
};

/// Lending market state
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LendingMarket {
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
    /// Outflow rate limiter denominated in dollars
    pub rate_limiter: RateLimiter,
    /// whitelisted liquidator
    pub whitelisted_liquidator: Option<Pubkey>,
    /// risk authority (additional pubkey used for setting params)
    pub risk_authority: Pubkey,
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
        self.bump_seed = params.bump_seed;
        self.owner = params.owner;
        self.quote_currency = params.quote_currency;
        self.token_program_id = params.token_program_id;
        self.oracle_program_id = params.oracle_program_id;
        self.switchboard_oracle_program_id = params.switchboard_oracle_program_id;
        self.rate_limiter = RateLimiter::default();
        self.whitelisted_liquidator = None;
        self.risk_authority = params.owner;
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

impl Sealed for LendingMarket {}
impl IsInitialized for LendingMarket {
    fn is_initialized(&self) -> bool {
        self.version != UNINITIALIZED_VERSION
    }
}

const LENDING_MARKET_LEN: usize = 290; // 1 + 1 + 32 + 32 + 32 + 32 + 32 + 56 + 32 + 40
impl Pack for LendingMarket {
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
            rate_limiter,
            whitelisted_liquidator,
            risk_authority,
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
            RATE_LIMITER_LEN,
            PUBKEY_BYTES,
            PUBKEY_BYTES,
            8
        ];

        *version = self.version.to_le_bytes();
        *bump_seed = self.bump_seed.to_le_bytes();
        owner.copy_from_slice(self.owner.as_ref());
        quote_currency.copy_from_slice(self.quote_currency.as_ref());
        token_program_id.copy_from_slice(self.token_program_id.as_ref());
        oracle_program_id.copy_from_slice(self.oracle_program_id.as_ref());
        switchboard_oracle_program_id.copy_from_slice(self.switchboard_oracle_program_id.as_ref());
        self.rate_limiter.pack_into_slice(rate_limiter);
        match self.whitelisted_liquidator {
            Some(pubkey) => {
                whitelisted_liquidator.copy_from_slice(pubkey.as_ref());
            }
            None => {
                whitelisted_liquidator.copy_from_slice(&[0u8; 32]);
            }
        }
        risk_authority.copy_from_slice(self.risk_authority.as_ref());
    }

    /// Unpacks a byte buffer into a [LendingMarketInfo](struct.LendingMarketInfo.html)
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
            rate_limiter,
            whitelisted_liquidator,
            risk_authority,
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
            RATE_LIMITER_LEN,
            PUBKEY_BYTES,
            PUBKEY_BYTES,
            8
        ];

        let version = u8::from_le_bytes(*version);
        if version > PROGRAM_VERSION {
            msg!("Lending market version does not match lending program version");
            return Err(ProgramError::InvalidAccountData);
        }

        let owner_pubkey = Pubkey::new_from_array(*owner);
        Ok(Self {
            version,
            bump_seed: u8::from_le_bytes(*bump_seed),
            owner: owner_pubkey,
            quote_currency: *quote_currency,
            token_program_id: Pubkey::new_from_array(*token_program_id),
            oracle_program_id: Pubkey::new_from_array(*oracle_program_id),
            switchboard_oracle_program_id: Pubkey::new_from_array(*switchboard_oracle_program_id),
            rate_limiter: RateLimiter::unpack_from_slice(rate_limiter)?,
            whitelisted_liquidator: if whitelisted_liquidator == &[0u8; 32] {
                None
            } else {
                Some(Pubkey::new_from_array(*whitelisted_liquidator))
            },
            // the risk authority can equal [0; 32] when the program is upgraded to v2.0.2. in that
            // case, we set the risk authority to be the owner. This isn't strictly necessary, but
            // better to be safe i guess.
            risk_authority: if *risk_authority == [0; 32] {
                owner_pubkey
            } else {
                Pubkey::new_from_array(*risk_authority)
            },
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use rand::Rng;

    #[test]
    fn pack_and_unpack_lending_market() {
        let mut rng = rand::thread_rng();
        let lending_market = LendingMarket {
            version: PROGRAM_VERSION,
            bump_seed: rng.gen(),
            owner: Pubkey::new_unique(),
            quote_currency: [rng.gen(); 32],
            token_program_id: Pubkey::new_unique(),
            oracle_program_id: Pubkey::new_unique(),
            switchboard_oracle_program_id: Pubkey::new_unique(),
            rate_limiter: rand_rate_limiter(),
            whitelisted_liquidator: if rng.gen_bool(0.5) {
                None
            } else {
                Some(Pubkey::new_unique())
            },
            risk_authority: Pubkey::new_unique(),
        };

        let mut packed = vec![0u8; LendingMarket::LEN];
        LendingMarket::pack(lending_market.clone(), &mut packed).unwrap();
        let unpacked = LendingMarket::unpack_from_slice(&packed).unwrap();
        assert_eq!(unpacked, lending_market);
    }
}

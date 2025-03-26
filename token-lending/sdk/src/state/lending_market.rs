use std::convert::TryFrom;

use crate::error::LendingError;

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
    /// For uninitialized accounts, this will be equal to [AccountDiscriminator::Uninitialized].
    /// Otherwise this is [AccountDiscriminator::LendingMarket].
    ///
    /// # Note
    /// For accounts last used with version prior to @v2.1.0 this will be equal
    /// to [PROGRAM_VERSION_2_0_2].
    pub discriminator: AccountDiscriminator,
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
        self.discriminator = AccountDiscriminator::LendingMarket;
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
        !matches!(self.discriminator, AccountDiscriminator::Uninitialized)
    }
}

const LENDING_MARKET_LEN: usize = 290; // 1 + 1 + 32 + 32 + 32 + 32 + 32 + 56 + 32 + 40
impl Pack for LendingMarket {
    const LEN: usize = LENDING_MARKET_LEN;

    fn pack_into_slice(&self, output: &mut [u8]) {
        let output = array_mut_ref![output, 0, LENDING_MARKET_LEN];
        #[allow(clippy::ptr_offset_with_cast)]
        let (
            discriminator,
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

        discriminator[0] = self.discriminator as _;
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
            discriminator,
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

        let discriminator = match AccountDiscriminator::try_from(discriminator) {
            Ok(d @ AccountDiscriminator::Uninitialized) => d, // yet to be set
            Ok(d @ AccountDiscriminator::LendingMarket) => d, // migrated to v2.1.0
            Ok(_) => {
                msg!("Lending market discriminator does not match");
                return Err(LendingError::InvalidAccountDiscriminator.into());
            }
            #[allow(clippy::assertions_on_constants)]
            Err(LendingError::AccountNotMigrated) => {
                // We're migrating the account from v2.0.2 to v2.1.0.
                // The reason this is safe to do is conveyed in these asserts:
                debug_assert_eq!(Self::LEN, input.len());
                debug_assert!(Self::LEN < Reserve::LEN);
                debug_assert!(Self::LEN < RESERVE_LEN_V2_0_2);
                debug_assert!(Self::LEN < Obligation::MIN_LEN);
                // Ie. there's no confusion with other account types.

                AccountDiscriminator::LendingMarket
            }
            Err(e) => return Err(e.into()),
        };

        let owner_pubkey = Pubkey::new_from_array(*owner);
        Ok(Self {
            discriminator,
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

    impl LendingMarket {
        fn new_rand(rng: &mut impl Rng) -> Self {
            Self {
                discriminator: AccountDiscriminator::LendingMarket,
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
            }
        }
    }

    #[test]
    fn pack_and_unpack_lending_market_v2_1_0() {
        let mut rng = rand::thread_rng();
        let lending_market = LendingMarket::new_rand(&mut rng);

        let mut packed = vec![0u8; LendingMarket::LEN];
        LendingMarket::pack(lending_market.clone(), &mut packed).unwrap();
        let unpacked = LendingMarket::unpack_from_slice(&packed).unwrap();
        assert_eq!(unpacked, lending_market);
    }

    #[test]
    fn pack_and_unpack_lending_market_v2_0_2() {
        let mut rng = rand::thread_rng();
        let lending_market = LendingMarket::new_rand(&mut rng);

        let mut packed = vec![0u8; LendingMarket::LEN];
        LendingMarket::pack(lending_market.clone(), &mut packed).unwrap();
        // this is what version looked like before the upgrade to v2.1.0
        packed[0] = PROGRAM_VERSION_2_0_2;

        let unpacked = LendingMarket::unpack_from_slice(&packed).unwrap();
        // upgraded
        assert_eq!(unpacked, lending_market);
    }
}

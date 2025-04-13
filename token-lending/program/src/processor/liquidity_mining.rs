//! Liquidity mining is a feature where depositors and borrowers are rewarded
//! for using the protocol.
//! The rewards are in the form of tokens that a lending market owner can attach
//! to each reserve.
//!
//! The feature is built with reference to the [Suilend][suilend-lm]
//! implementation of the same feature.
//!
//! There are three admin-only ixs:
//! - [add_pool_reward]
//! - [cancel_pool_reward]
//! - [close_pool_reward]
//!
//! There is an ix related to migration:
//! - [upgrade_reserve] (has anchor integration test)
//!
//! There is one user ix:
//! - [claim_user_reward]
//!
//! [suilend-lm]: https://github.com/solendprotocol/suilend/blob/dc53150416f352053ac3acbb320ee143409c4a5d/contracts/suilend/sources/liquidity_mining.move#L2

pub(crate) mod add_pool_reward;
pub(crate) mod cancel_pool_reward;
pub(crate) mod claim_user_reward;
pub(crate) mod close_pool_reward;
pub(crate) mod upgrade_reserve;

use solana_program::program_pack::Pack;
use solana_program::{account_info::AccountInfo, msg, program_error::ProgramError, pubkey::Pubkey};
use solend_sdk::instruction::create_reward_vault_authority;
use solend_sdk::{error::LendingError, state::LendingMarket};
use spl_token::state::Account as TokenAccount;

use super::ReserveBorrow;
struct Bumps {
    reward_authority: u8,
}

/// Unpacks a spl_token [TokenAccount].
fn unpack_token_account(data: &[u8]) -> Result<TokenAccount, LendingError> {
    TokenAccount::unpack(data).map_err(|_| LendingError::InvalidTokenAccount)
}

/// Named args for [check_and_unpack_pool_reward_accounts]
struct CheckAndUnpackPoolRewardAccounts<'a, 'info> {
    reserve_info: &'a AccountInfo<'info>,
    reward_mint_info: &'a AccountInfo<'info>,
    reward_authority_info: &'a AccountInfo<'info>,
    lending_market_info: &'a AccountInfo<'info>,
    token_program_info: &'a AccountInfo<'info>,
}

/// Does all the checks of [check_and_unpack_pool_reward_accounts] and additionally:
///
/// * ✅ `lending_market_owner_info` is a signer
/// * ✅ `lending_market_owner_info` matches `lending_market_info`
fn check_and_unpack_pool_reward_accounts_for_admin_ixs<'a, 'info>(
    program_id: &Pubkey,
    bumps: Bumps,
    accs: CheckAndUnpackPoolRewardAccounts<'a, 'info>,
    lending_market_owner_info: &AccountInfo<'info>,
) -> Result<(LendingMarket, ReserveBorrow<'a, 'info>), ProgramError> {
    let (lending_market, reserve) = check_and_unpack_pool_reward_accounts(program_id, bumps, accs)?;

    if lending_market.owner != *lending_market_owner_info.key {
        msg!("Lending market owner does not match the lending market owner provided");
        return Err(LendingError::InvalidMarketOwner.into());
    }
    if !lending_market_owner_info.is_signer {
        msg!("Lending market owner provided must be a signer");
        return Err(LendingError::InvalidSigner.into());
    }

    Ok((lending_market, reserve))
}

/// Checks that:
///
/// * ✅ `reserve_info` belongs to this program
/// * ✅ `reserve_info` unpacks
/// * ✅ `reserve_info` belongs to `lending_market_info`
/// * ✅ `lending_market_info` belongs to this program
/// * ✅ `lending_market_info` unpacks
/// * ✅ `token_program_info` matches `lending_market_info`
/// * ✅ `reward_mint_info` belongs to the token program
/// * ✅ `reward_authority_info` is seed of `lending_market_info`, `reserve_info`, `reward_mint_info`
fn check_and_unpack_pool_reward_accounts<'a, 'info>(
    program_id: &Pubkey,
    bumps: Bumps,
    CheckAndUnpackPoolRewardAccounts {
        reserve_info,
        reward_mint_info,
        reward_authority_info,
        lending_market_info,
        token_program_info,
    }: CheckAndUnpackPoolRewardAccounts<'a, 'info>,
) -> Result<(LendingMarket, ReserveBorrow<'a, 'info>), ProgramError> {
    let reserve = ReserveBorrow::new_mut(program_id, reserve_info)?;

    if lending_market_info.owner != program_id {
        msg!("Lending market provided is not owned by the lending program");
        return Err(LendingError::InvalidAccountOwner.into());
    }
    let lending_market = LendingMarket::unpack(&lending_market_info.data.borrow())?;

    if reserve.lending_market != *lending_market_info.key {
        msg!("Reserve lending market does not match the lending market provided");
        return Err(LendingError::InvalidAccountInput.into());
    }

    if lending_market.token_program_id != *token_program_info.key {
        msg!("Lending market token program does not match the token program provided");
        return Err(LendingError::InvalidTokenProgram.into());
    }

    if reward_mint_info.owner != token_program_info.key {
        msg!("Reward mint provided must be owned by the token program");
        return Err(LendingError::InvalidTokenOwner.into());
    }

    let expected_reward_vault_authority = create_reward_vault_authority(
        program_id,
        lending_market_info.key,
        reserve_info.key,
        reward_mint_info.key,
        bumps.reward_authority,
    )?;
    if expected_reward_vault_authority != *reward_authority_info.key {
        msg!("Reward vault authority does not match the expected value");
        return Err(LendingError::InvalidAccountInput.into());
    }

    Ok((lending_market, reserve))
}

#[cfg(test)]
mod tests {
    //! For each ✅ in [check_and_unpack_pool_reward_accounts] and
    //! [check_and_unpack_pool_reward_accounts_for_admin_ixs] there is a test
    //! that expects a failure if that conditions is not met.

    use solana_program::system_program;
    use solend_sdk::{
        instruction::find_reward_vault_authority,
        state::{discriminator::AccountDiscriminator, Reserve},
    };
    use spl_token::state::Mint;

    use super::*;

    #[test]
    fn test_check_and_unpack_pool_reward_accounts_ok() {
        let (account_info_builders, bumps) = CheckAndUnpackPoolRewardAccountInfoBuilders::new();

        account_info_builders
            .check_and_unpack_pool_reward_accounts(crate::id(), bumps)
            .expect("Should succeed");
    }

    /// ❌ `reserve_info` belongs to this program
    #[test]
    fn test_fails_if_reserve_info_does_not_belong_to_program() {
        let (mut account_info_builders, bumps) = CheckAndUnpackPoolRewardAccountInfoBuilders::new();
        account_info_builders.reserve.owner = Pubkey::new_unique();

        account_info_builders
            .check_and_unpack_pool_reward_accounts(crate::id(), bumps)
            .expect_err("Should fail");
    }

    /// ❌ `reserve_info` unpacks
    #[test]
    fn test_fails_if_reserve_info_does_not_unpack() {
        let (mut account_info_builders, bumps) = CheckAndUnpackPoolRewardAccountInfoBuilders::new();
        account_info_builders.reserve.data = vec![0; Reserve::get_packed_len() - 1];

        account_info_builders
            .check_and_unpack_pool_reward_accounts(crate::id(), bumps)
            .expect_err("Should fail");
    }

    /// ❌ `reserve_info` belongs to `lending_market_info`
    #[test]
    fn test_fails_if_reserve_info_does_not_belong_to_lending_market() {
        let (mut account_info_builders, bumps) = CheckAndUnpackPoolRewardAccountInfoBuilders::new();

        account_info_builders.reserve = AccountInfoBuilder::from(Reserve {
            discriminator: AccountDiscriminator::Reserve,
            lending_market: Pubkey::new_unique(),
            ..Default::default()
        });

        account_info_builders
            .check_and_unpack_pool_reward_accounts(crate::id(), bumps)
            .expect_err("Should fail");
    }

    ///  ❌ `lending_market_info` belongs to this program
    #[test]
    fn test_fails_if_lending_market_info_does_not_belong_to_program() {
        let (mut account_info_builders, bumps) = CheckAndUnpackPoolRewardAccountInfoBuilders::new();
        account_info_builders.lending_market.owner = Pubkey::new_unique();

        account_info_builders
            .check_and_unpack_pool_reward_accounts(crate::id(), bumps)
            .expect_err("Should fail");
    }

    ///  ❌ `lending_market_info` unpacks
    #[test]
    fn test_fails_if_lending_market_info_does_not_unpack() {
        let (mut account_info_builders, bumps) = CheckAndUnpackPoolRewardAccountInfoBuilders::new();
        account_info_builders.lending_market.data = vec![0; LendingMarket::get_packed_len() - 1];

        account_info_builders
            .check_and_unpack_pool_reward_accounts(crate::id(), bumps)
            .expect_err("Should fail");
    }

    ///  ❌ `token_program_info` matches `lending_market_info`
    #[test]
    fn test_fails_if_token_program_info_does_not_match_lending_market() {
        let (mut account_info_builders, bumps) = CheckAndUnpackPoolRewardAccountInfoBuilders::new();

        account_info_builders.lending_market = AccountInfoBuilder::from(LendingMarket {
            discriminator: AccountDiscriminator::LendingMarket,
            token_program_id: Pubkey::new_unique(),
            ..Default::default()
        });

        account_info_builders
            .check_and_unpack_pool_reward_accounts(crate::id(), bumps)
            .expect_err("Should fail");
    }

    ///  ❌ `reward_mint_info` belongs to the token program
    #[test]
    fn test_fails_if_reward_mint_info_does_not_belong_to_token_program() {
        let (mut account_info_builders, bumps) = CheckAndUnpackPoolRewardAccountInfoBuilders::new();
        account_info_builders.mint.owner = Pubkey::new_unique();

        account_info_builders
            .check_and_unpack_pool_reward_accounts(crate::id(), bumps)
            .expect_err("Should fail");
    }

    /// ❌ `reward_authority_info` is seed of `lending_market_info`, `reserve_info`, `reward_mint_info`
    #[test]
    fn test_fails_if_reward_authority_info_is_not_seed() {
        let (mut account_info_builders, og_bumps) =
            CheckAndUnpackPoolRewardAccountInfoBuilders::new();
        let og_reward_authority = account_info_builders.reward_authority.key;

        // wrong lending market

        let (new_reward_authority, new_reward_authority_bump) = find_reward_vault_authority(
            &crate::id(),
            &Pubkey::new_unique(),
            &account_info_builders.reserve.key,
            &account_info_builders.mint.key,
        );
        account_info_builders.reward_authority.key = new_reward_authority;
        account_info_builders
            .clone()
            .check_and_unpack_pool_reward_accounts(
                crate::id(),
                Bumps {
                    reward_authority: new_reward_authority_bump,
                },
            )
            .expect_err("Should fail");

        // wrong reserve

        let (new_reward_authority, new_reward_authority_bump) = find_reward_vault_authority(
            &crate::id(),
            &account_info_builders.lending_market.key,
            &Pubkey::new_unique(),
            &account_info_builders.mint.key,
        );
        account_info_builders.reward_authority.key = new_reward_authority;
        account_info_builders
            .clone()
            .check_and_unpack_pool_reward_accounts(
                crate::id(),
                Bumps {
                    reward_authority: new_reward_authority_bump,
                },
            )
            .expect_err("Should fail");

        // wrong mint

        let (new_reward_authority, new_reward_authority_bump) = find_reward_vault_authority(
            &crate::id(),
            &account_info_builders.lending_market.key,
            &account_info_builders.reserve.key,
            &Pubkey::new_unique(),
        );
        account_info_builders.reward_authority.key = new_reward_authority;
        account_info_builders
            .clone()
            .check_and_unpack_pool_reward_accounts(
                crate::id(),
                Bumps {
                    reward_authority: new_reward_authority_bump,
                },
            )
            .expect_err("Should fail");

        // wrong bump

        account_info_builders.reward_authority.key = og_reward_authority;
        account_info_builders
            .clone()
            .check_and_unpack_pool_reward_accounts(
                crate::id(),
                Bumps {
                    reward_authority: og_bumps.reward_authority.wrapping_add(1),
                },
            )
            .expect_err("Should fail");
    }

    #[test]
    fn test_check_and_unpack_pool_reward_accounts_for_admin_ixs_ok() {
        let (account_info_builders, bumps) = CheckAndUnpackPoolRewardAccountInfoBuilders::new();

        account_info_builders
            .check_and_unpack_pool_reward_accounts_for_admin_ixs(crate::id(), bumps)
            .expect("Should succeed");
    }

    /// ❌ `lending_market_owner_info` is a signer
    #[test]
    fn test_fails_if_lending_market_owner_info_is_not_signer() {
        let (mut account_info_builders, bumps) = CheckAndUnpackPoolRewardAccountInfoBuilders::new();
        account_info_builders.lending_market_owner.is_signer = false;

        account_info_builders
            .check_and_unpack_pool_reward_accounts_for_admin_ixs(crate::id(), bumps)
            .expect_err("Should fail");
    }

    /// ❌ `lending_market_owner_info` matches `lending_market_info`
    #[test]
    fn test_fails_if_lending_market_owner_info_does_not_match_lending_market() {
        let (mut account_info_builders, bumps) = CheckAndUnpackPoolRewardAccountInfoBuilders::new();
        account_info_builders.lending_market_owner.key = Pubkey::new_unique();

        account_info_builders
            .check_and_unpack_pool_reward_accounts_for_admin_ixs(crate::id(), bumps)
            .expect_err("Should fail");
    }

    #[derive(Clone)]
    struct CheckAndUnpackPoolRewardAccountInfoBuilders {
        lending_market: AccountInfoBuilder,
        lending_market_owner: AccountInfoBuilder,
        mint: AccountInfoBuilder,
        reserve: AccountInfoBuilder,
        reward_authority: AccountInfoBuilder,
        token_program: AccountInfoBuilder,
    }

    #[derive(Clone)]
    struct AccountInfoBuilder {
        key: Pubkey,
        lamports: u64,
        data: Vec<u8>,
        owner: Pubkey,
        rent_epoch: u64,
        is_signer: bool,
        is_writable: bool,
        is_executable: bool,
    }

    impl CheckAndUnpackPoolRewardAccountInfoBuilders {
        fn new() -> (Self, Bumps) {
            let token_program = AccountInfoBuilder::new_token_program();
            let lending_market_owner = AccountInfoBuilder::new_lending_market_owner();
            let lending_market: AccountInfoBuilder = AccountInfoBuilder::from(LendingMarket {
                discriminator: AccountDiscriminator::LendingMarket,
                token_program_id: token_program.key,
                owner: lending_market_owner.key,
                ..Default::default()
            });
            let mint = AccountInfoBuilder::from(Mint {
                is_initialized: true,
                ..Default::default()
            });
            let reserve = AccountInfoBuilder::from(Reserve {
                discriminator: AccountDiscriminator::Reserve,
                lending_market: lending_market.key,
                ..Default::default()
            });
            let (reward_authority, bumps) = AccountInfoBuilder::new_reward_authority(
                &lending_market.key,
                &reserve.key,
                &mint.key,
            );

            (
                Self {
                    lending_market_owner,
                    lending_market,
                    mint,
                    reserve,
                    reward_authority,
                    token_program,
                },
                bumps,
            )
        }

        fn check_and_unpack_pool_reward_accounts(
            mut self,
            program_id: Pubkey,
            bumps: Bumps,
        ) -> Result<(), ProgramError> {
            let lending_market_info = self.lending_market.as_account_info();
            let mint_info = self.mint.as_account_info();
            let reserve_info = self.reserve.as_account_info();
            let reward_authority_info = self.reward_authority.as_account_info();
            let token_program_info = self.token_program.as_account_info();

            check_and_unpack_pool_reward_accounts(
                &program_id,
                bumps,
                CheckAndUnpackPoolRewardAccounts {
                    lending_market_info: &lending_market_info,
                    reserve_info: &reserve_info,
                    reward_authority_info: &reward_authority_info,
                    reward_mint_info: &mint_info,
                    token_program_info: &token_program_info,
                },
            )
            .map(drop)
        }

        fn check_and_unpack_pool_reward_accounts_for_admin_ixs(
            mut self,
            program_id: Pubkey,
            bumps: Bumps,
        ) -> Result<(), ProgramError> {
            let lending_market_info = self.lending_market.as_account_info();
            let mint_info = self.mint.as_account_info();
            let reserve_info = self.reserve.as_account_info();
            let reward_authority_info = self.reward_authority.as_account_info();
            let token_program_info = self.token_program.as_account_info();
            let lending_market_owner_info = self.lending_market_owner.as_account_info();

            check_and_unpack_pool_reward_accounts_for_admin_ixs(
                &program_id,
                bumps,
                CheckAndUnpackPoolRewardAccounts {
                    lending_market_info: &lending_market_info,
                    reserve_info: &reserve_info,
                    reward_authority_info: &reward_authority_info,
                    reward_mint_info: &mint_info,
                    token_program_info: &token_program_info,
                },
                &lending_market_owner_info,
            )
            .map(drop)
        }
    }

    impl From<LendingMarket> for AccountInfoBuilder {
        fn from(lending_market: LendingMarket) -> Self {
            let mut data = vec![0; LendingMarket::get_packed_len()];
            LendingMarket::pack(lending_market, &mut data).unwrap();

            Self {
                key: Pubkey::new_unique(),
                lamports: 1,
                data,
                owner: crate::id(),
                rent_epoch: 0,
                is_signer: false,
                is_writable: false,
                is_executable: false,
            }
        }
    }

    impl From<Mint> for AccountInfoBuilder {
        fn from(mint: Mint) -> Self {
            let mut data = vec![0; Mint::get_packed_len()];
            Mint::pack(mint, &mut data).unwrap();

            Self {
                key: Pubkey::new_unique(),
                lamports: 1,
                data,
                owner: spl_token::id(),
                rent_epoch: 0,
                is_signer: false,
                is_writable: false,
                is_executable: false,
            }
        }
    }

    impl From<Reserve> for AccountInfoBuilder {
        fn from(reserve: Reserve) -> Self {
            let mut data = vec![0; Reserve::get_packed_len()];
            Reserve::pack(reserve, &mut data).unwrap();

            Self {
                key: Pubkey::new_unique(),
                lamports: 1,
                data,
                owner: crate::id(),
                rent_epoch: 0,
                is_signer: false,
                is_writable: false,
                is_executable: false,
            }
        }
    }

    impl AccountInfoBuilder {
        fn as_account_info(&mut self) -> AccountInfo {
            AccountInfo::new(
                &self.key,
                self.is_signer,
                self.is_writable,
                &mut self.lamports,
                &mut self.data,
                &self.owner,
                self.is_executable,
                self.rent_epoch,
            )
        }

        fn new_token_program() -> Self {
            Self {
                key: spl_token::id(),
                lamports: 0,
                data: vec![],
                owner: system_program::id(),
                rent_epoch: 0,
                is_signer: false,
                is_writable: false,
                is_executable: true,
            }
        }

        fn new_reward_authority(
            lending_market_key: &Pubkey,
            reserve_key: &Pubkey,
            reward_mint_key: &Pubkey,
        ) -> (Self, Bumps) {
            let (key, bump) = find_reward_vault_authority(
                &crate::id(),
                lending_market_key,
                reserve_key,
                reward_mint_key,
            );

            let s = Self {
                key,
                lamports: 0,
                data: vec![],
                owner: system_program::id(),
                rent_epoch: 0,
                is_signer: false,
                is_writable: false,
                is_executable: false,
            };

            (
                s,
                Bumps {
                    reward_authority: bump,
                },
            )
        }

        fn new_lending_market_owner() -> Self {
            Self {
                key: Pubkey::new_unique(),
                lamports: 0,
                data: vec![],
                owner: system_program::id(),
                rent_epoch: 0,
                is_signer: true,
                is_writable: false,
                is_executable: false,
            }
        }
    }
}

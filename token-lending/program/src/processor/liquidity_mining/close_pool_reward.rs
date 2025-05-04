//! Closes a pool reward, making its slot vacant and ready for a new reward.
//!
//! Before closing a pool reward that pool reward must first be cancelled
//! and all rewards must be claimed by the users.
//!
//! The claim ix is permission-less and therefore it can be cranked.

use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    msg,
    program_error::ProgramError,
    pubkey::Pubkey,
};
use solend_sdk::{
    error::LendingError, instruction::reward_vault_authority_seeds, state::PositionKind,
};
use spl_token::state::Account as TokenAccount;

use crate::processor::{
    spl_token_close_account, spl_token_transfer, TokenCloseAccountParams, TokenTransferParams,
};

use super::{
    check_and_unpack_pool_reward_accounts_for_admin_ixs, unpack_token_account, Bumps,
    CheckAndUnpackPoolRewardAccounts, ReserveBorrow,
};

/// Use [Self::from_unchecked_iter] to validate the accounts.
struct ClosePoolRewardAccounts<'a, 'info> {
    /// ✅ belongs to this program
    /// ✅ unpacks
    /// ✅ belongs to `lending_market_info`
    /// ✅ is writable
    _reserve_info: &'a AccountInfo<'info>,
    /// ✅ belongs to the token program
    _reward_mint_info: &'a AccountInfo<'info>,
    /// ✅ belongs to the token program
    /// ✅ owned by `lending_market_owner_info`
    /// ✅ matches `reward_mint_info`
    /// ✅ is writable
    reward_token_destination_info: &'a AccountInfo<'info>,
    /// ✅ seed of `lending_market_info`, `reward_token_vault_info`
    reward_authority_info: &'a AccountInfo<'info>,
    /// ❓ we don't know whether it matches vault in the [Reserve]
    /// ✅ is writable
    /// ✅ unpacks
    reward_token_vault_info: &'a AccountInfo<'info>,
    /// ✅ belongs to this program
    /// ✅ unpacks
    lending_market_info: &'a AccountInfo<'info>,
    /// ✅ is a signer
    /// ✅ matches `lending_market_info`
    lending_market_owner_info: &'a AccountInfo<'info>,
    /// ✅ matches `lending_market_info`
    token_program_info: &'a AccountInfo<'info>,

    reserve: ReserveBorrow<'a, 'info>,
    reward_token_vault: TokenAccount,
}

/// # Effects
///
/// 1. Closes reward in the [Reserve] account if all users have claimed.
/// 2. Transfers dust to the `reward_token_destination` account.
/// 3. Closes reward vault token account.
pub(crate) fn process(
    program_id: &Pubkey,
    reward_authority_bump: u8,
    position_kind: PositionKind,
    pool_reward_index: usize,
    accounts: &[AccountInfo],
) -> ProgramResult {
    let mut accounts = ClosePoolRewardAccounts::from_unchecked_iter(
        program_id,
        Bumps {
            reward_authority: reward_authority_bump,
        },
        &mut accounts.iter(),
    )?;

    // 1.

    let expected_vault = accounts
        .reserve
        .pool_reward_manager_mut(position_kind)
        .close_pool_reward(pool_reward_index)?;
    if expected_vault != *accounts.reward_token_vault_info.key {
        msg!("Reward token vault provided does not match the expected vault");
        return Err(LendingError::InvalidAccountInput.into());
    }

    // 2.

    let bump_seed = [reward_authority_bump];
    let signer_seeds = [
        reward_vault_authority_seeds(
            accounts.lending_market_info.key,
            accounts.reward_token_vault_info.key,
        )
        .as_slice(),
        &[&bump_seed],
    ]
    .concat();

    msg!(
        "Transferring {} reward tokens to {}",
        accounts.reward_token_vault.amount,
        accounts.reward_token_destination_info.key
    );
    spl_token_transfer(TokenTransferParams {
        source: accounts.reward_token_vault_info.clone(),
        destination: accounts.reward_token_destination_info.clone(),
        amount: accounts.reward_token_vault.amount,
        authority: accounts.reward_authority_info.clone(),
        authority_signer_seeds: &signer_seeds,
        token_program: accounts.token_program_info.clone(),
    })?;

    // 3.

    msg!(
        "Closing reward token vault {}",
        accounts.reward_token_vault_info.key
    );
    spl_token_close_account(TokenCloseAccountParams {
        account: accounts.reward_token_vault_info.clone(),
        destination: accounts.lending_market_owner_info.clone(),
        authority: accounts.reward_authority_info.clone(),
        authority_signer_seeds: &signer_seeds,
        token_program: accounts.token_program_info.clone(),
    })?;

    Ok(())
}

impl<'a, 'info> ClosePoolRewardAccounts<'a, 'info> {
    fn from_unchecked_iter(
        program_id: &Pubkey,
        bumps: Bumps,
        iter: &mut impl Iterator<Item = &'a AccountInfo<'info>>,
    ) -> Result<ClosePoolRewardAccounts<'a, 'info>, ProgramError> {
        let reserve_info = next_account_info(iter)?;
        let reward_mint_info = next_account_info(iter)?;
        let reward_token_destination_info = next_account_info(iter)?;
        let reward_authority_info = next_account_info(iter)?;
        let reward_token_vault_info = next_account_info(iter)?;
        let lending_market_info = next_account_info(iter)?;
        let lending_market_owner_info = next_account_info(iter)?;
        let token_program_info = next_account_info(iter)?;

        let (_, reserve) = check_and_unpack_pool_reward_accounts_for_admin_ixs(
            program_id,
            bumps,
            CheckAndUnpackPoolRewardAccounts {
                reserve_info,
                reward_mint_info,
                reward_authority_info,
                lending_market_info,
                token_program_info,
                reward_token_vault_info,
            },
            lending_market_owner_info,
        )?;

        if reward_token_destination_info.owner != token_program_info.key {
            msg!("Reward token destination provided must be owned by the token program");
            return Err(LendingError::InvalidTokenOwner.into());
        }
        let reward_token_destination =
            unpack_token_account(&reward_token_destination_info.data.borrow())?;
        if reward_token_destination.mint != *reward_mint_info.key {
            msg!("Reward token destination mint does not match the reward mint provided");
            return Err(LendingError::InvalidAccountInput.into());
        }

        let reward_token_vault = unpack_token_account(&reward_token_vault_info.data.borrow())?;
        if reward_token_vault.mint != *reward_mint_info.key {
            msg!("Reward token vault mint does not match the reward mint provided");
            return Err(LendingError::InvalidAccountInput.into());
        }

        // check that accounts that should be writable are writable

        if !reserve_info.is_writable {
            msg!("Reserve provided must be writable");
            return Err(ProgramError::InvalidAccountData);
        }
        if !reward_token_destination_info.is_writable {
            msg!("Reward token destination provided must be writable");
            return Err(ProgramError::InvalidAccountData);
        }
        if !reward_token_vault_info.is_writable {
            msg!("Reward token vault provided must be writable");
            return Err(ProgramError::InvalidAccountData);
        }

        Ok(Self {
            _reserve_info: reserve_info,
            _reward_mint_info: reward_mint_info,
            reward_token_destination_info,
            reward_authority_info,
            reward_token_vault_info,
            lending_market_info,
            lending_market_owner_info,
            token_program_info,

            reserve,
            reward_token_vault,
        })
    }
}

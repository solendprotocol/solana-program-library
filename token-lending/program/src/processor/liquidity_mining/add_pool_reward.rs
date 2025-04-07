//! Adds a new pool reward to a reserve.
//!
//! Each pool reward has a unique vault that holds the reward tokens.

use crate::processor::{
    assert_rent_exempt, spl_token_init_account, spl_token_transfer, TokenInitializeAccountParams,
    TokenTransferParams,
};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    clock::Clock,
    entrypoint::ProgramResult,
    msg,
    program_error::ProgramError,
    pubkey::Pubkey,
    rent::Rent,
    sysvar::Sysvar,
};
use solend_sdk::{error::LendingError, state::PositionKind};

use super::{
    check_and_unpack_pool_reward_accounts_for_admin_ixs, unpack_token_account, ReserveBorrow,
};

/// Use [Self::from_unchecked_iter] to validate the accounts except for
/// * `reward_token_vault_info`
/// * `rent_info`
struct AddPoolRewardAccounts<'a, 'info> {
    /// ✅ belongs to this program
    /// ✅ unpacks
    /// ✅ belongs to `lending_market_info`
    /// ✅ is writable
    _reserve_info: &'a AccountInfo<'info>,
    /// ✅ belongs to the token program
    reward_mint_info: &'a AccountInfo<'info>,
    /// ✅ belongs to the token program
    /// ✅ owned by `lending_market_owner_info`
    /// ❓ we don't know yet whether it has enough tokens
    /// ✅ matches `reward_mint_info`
    /// ✅ is writable
    reward_token_source_info: &'a AccountInfo<'info>,
    /// ✅ seed of `lending_market_info`, `reserve_info`, `reward_mint_info`
    reward_authority_info: &'a AccountInfo<'info>,
    /// ✅ belongs to the token program
    /// ✅ has no data
    /// ✅ is writable
    /// ❓ we don't yet know whether it's rent exempt
    reward_token_vault_info: &'a AccountInfo<'info>,
    /// ✅ belongs to this program
    /// ✅ unpacks
    _lending_market_info: &'a AccountInfo<'info>,
    /// ✅ is a signer
    /// ✅ matches `lending_market_info`
    ///
    /// TBD: do we want to create another signer authority to be able to
    /// delegate reward management to a softer multisig?
    lending_market_owner_info: &'a AccountInfo<'info>,
    /// ❓ we don't yet know whether this is rent info
    rent_info: &'a AccountInfo<'info>,
    /// ✅ matches `lending_market_info`
    token_program_info: &'a AccountInfo<'info>,

    reserve: ReserveBorrow<'a, 'info>,
}

/// # Effects
///
/// 1. Initializes a new reward vault account and transfers
///    `reward_token_amount` tokens from the `reward_token_source` account to
///     the new reward vault account.
/// 2. Finds an empty slot in the [Reserve]'s LM reward vector and adds it there.
pub(crate) fn process(
    program_id: &Pubkey,
    position_kind: PositionKind,
    start_time_secs: u64,
    end_time_secs: u64,
    reward_token_amount: u64,
    accounts: &[AccountInfo],
) -> ProgramResult {
    msg!("Adding {position_kind:?} pool reward from {start_time_secs}s to {end_time_secs}s",);

    let clock = &Clock::get()?;

    let mut accounts =
        AddPoolRewardAccounts::from_unchecked_iter(program_id, &mut accounts.iter())?;

    // 1.

    spl_token_init_account(TokenInitializeAccountParams {
        account: accounts.reward_token_vault_info.clone(),
        mint: accounts.reward_mint_info.clone(),
        owner: accounts.reward_authority_info.clone(),
        rent: accounts.rent_info.clone(),
        token_program: accounts.token_program_info.clone(),
    })?;
    let rent = &Rent::from_account_info(accounts.rent_info)?;
    assert_rent_exempt(rent, accounts.reward_token_vault_info)?;

    spl_token_transfer(TokenTransferParams {
        source: accounts.reward_token_source_info.clone(),
        destination: accounts.reward_token_vault_info.clone(),
        amount: reward_token_amount,
        authority: accounts.lending_market_owner_info.clone(),
        authority_signer_seeds: &[],
        token_program: accounts.token_program_info.clone(),
    })?;

    // 2.

    accounts
        .reserve
        .pool_reward_manager_mut(position_kind)
        .add_pool_reward(
            *accounts.reward_token_vault_info.key,
            start_time_secs,
            end_time_secs,
            reward_token_amount,
            clock,
        )?;

    Ok(())
}

impl<'a, 'info> AddPoolRewardAccounts<'a, 'info> {
    fn from_unchecked_iter(
        program_id: &Pubkey,
        iter: &mut impl Iterator<Item = &'a AccountInfo<'info>>,
    ) -> Result<AddPoolRewardAccounts<'a, 'info>, ProgramError> {
        let reserve_info = next_account_info(iter)?;
        let reward_mint_info = next_account_info(iter)?;
        let reward_token_source_info = next_account_info(iter)?;
        let reward_authority_info = next_account_info(iter)?;
        let reward_token_vault_info = next_account_info(iter)?;
        let lending_market_info = next_account_info(iter)?;
        let lending_market_owner_info = next_account_info(iter)?;
        let rent_info = next_account_info(iter)?;
        let token_program_info = next_account_info(iter)?;

        let (_, reserve) = check_and_unpack_pool_reward_accounts_for_admin_ixs(
            program_id,
            reserve_info,
            reward_mint_info,
            reward_authority_info,
            lending_market_info,
            lending_market_owner_info,
            token_program_info,
        )?;

        if reward_token_source_info.owner != token_program_info.key {
            msg!("Reward token source provided must be owned by the token program");
            return Err(LendingError::InvalidTokenOwner.into());
        }
        let reward_token_source = unpack_token_account(&reward_token_source_info.data.borrow())?;
        if reward_token_source.owner != *lending_market_owner_info.key {
            msg!("Reward token source owner does not match the lending market owner provided");
            return Err(LendingError::InvalidAccountInput.into());
        }
        if reward_token_source.mint != *reward_mint_info.key {
            msg!("Reward token source mint does not match the reward mint provided");
            return Err(LendingError::InvalidAccountInput.into());
        }

        if reward_token_vault_info.owner != token_program_info.key {
            msg!("Reward token vault provided must be owned by the token program");
            return Err(LendingError::InvalidTokenOwner.into());
        }

        // check that accounts that should be writable are writable

        if !reward_token_vault_info.is_writable {
            msg!("Reward token vault provided must be writable");
            return Err(ProgramError::InvalidAccountData);
        }
        if !reserve_info.is_writable {
            msg!("Reserve provided must be writable");
            return Err(ProgramError::InvalidAccountData);
        }
        if !reward_token_source_info.is_writable {
            msg!("Reward token source provided must be writable");
            return Err(ProgramError::InvalidAccountData);
        }

        Ok(Self {
            _reserve_info: reserve_info,
            reward_mint_info,
            reward_token_source_info,
            reward_authority_info,
            reward_token_vault_info,
            _lending_market_info: lending_market_info,
            lending_market_owner_info,
            rent_info,
            token_program_info,

            reserve,
        })
    }
}

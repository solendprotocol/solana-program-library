//! Edits a pool reward.
//!
//! # Cancel
//! Cancelling a pool reward can be done by setting the end time to 0.
//! Note that only rewards longer than [solend_sdk::MIN_REWARD_PERIOD_SECS] can be cancelled.
//! In this case we transfer tokens from the reward vault to the lending market reward token account.
//!
//! # Shorten
//! If the new endtime is in the future, larger than start time and smaller than previous end time
//! then we shorten the reward period, refunding the unallocated rewards to the lending market
//! reward token account.
//!
//! # Extend
//! If the new endtime is in the future, larger than start time and larger than previous end time
//! then we extend the reward period, taking more tokens from the lending market reward token
//! account.
//!
//! ---
//!
//! Both extending and shortening calculate the difference between total rewards linearly.

use crate::processor::liquidity_mining::{
    check_and_unpack_pool_reward_accounts_for_admin_ixs, unpack_token_account,
};
use crate::processor::{spl_token_transfer, TokenTransferParams};
use solana_program::sysvar::Sysvar;
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    clock::Clock,
    entrypoint::ProgramResult,
    msg,
    program_error::ProgramError,
    pubkey::Pubkey,
};
use solend_sdk::instruction::reward_vault_authority_seeds;
use solend_sdk::{error::LendingError, state::PositionKind};

use super::{Bumps, CheckAndUnpackPoolRewardAccounts, ReserveBorrow};

/// Use [Self::from_unchecked_iter] to validate the accounts.
struct EditPoolRewardAccounts<'a, 'info> {
    /// ✅ belongs to this program
    /// ✅ unpacks
    /// ✅ belongs to `lending_market_info`
    /// ✅ is writable
    _reserve_info: &'a AccountInfo<'info>,
    /// ✅ belongs to the token program
    reward_mint_info: &'a AccountInfo<'info>,
    /// ✅ belongs to the token program
    /// ✅ matches `reward_mint_info`
    /// ✅ is writable
    lending_market_reward_token_account_info: &'a AccountInfo<'info>,
    /// ✅ seed of `lending_market_info`, `reserve_info`, `reward_mint_info`
    reward_authority_info: &'a AccountInfo<'info>,
    /// ❓ we don't know whether it matches the reward vault pubkey stored in [Reserve]
    /// ✅ is writable
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
}

/// # Effects
///
/// 1. Sets the new time
/// 2. Either refunds the admin or takes more tokens from the admin, based on the new end time
///    relation to the old end time
pub(crate) fn process(
    program_id: &Pubkey,
    reward_authority_bump: u8,
    position_kind: PositionKind,
    pool_reward_index: usize,
    new_end_time_secs: u64,
    accounts: &[AccountInfo],
) -> ProgramResult {
    let mut accounts = EditPoolRewardAccounts::from_unchecked_iter(
        program_id,
        Bumps {
            reward_authority: reward_authority_bump,
        },
        &mut accounts.iter(),
    )?;

    // 1.

    let (expected_vault, unallocated_rewards) = accounts
        .reserve
        .pool_reward_manager_mut(position_kind)
        .edit_pool_reward(pool_reward_index, new_end_time_secs, &Clock::get()?)?;

    if expected_vault != *accounts.reward_token_vault_info.key {
        msg!("Reward vault provided does not match the reward vault pubkey stored in [Reserve]");
        return Err(LendingError::InvalidAccountInput.into());
    }

    // 2.

    if unallocated_rewards == 0 {
        msg!("No unallocated rewards to transfer");
        return Ok(());
    }

    match unallocated_rewards {
        0 => {
            msg!("No unallocated rewards to transfer");
            Ok(())
        }
        // refund to lending market reward token account
        1.. => spl_token_transfer(TokenTransferParams {
            source: accounts.lending_market_reward_token_account_info.clone(),
            destination: accounts.reward_token_vault_info.clone(),
            amount: unallocated_rewards.unsigned_abs(),
            authority: accounts.lending_market_owner_info.clone(),
            authority_signer_seeds: &[],
            token_program: accounts.token_program_info.clone(),
        }),
        // take from reward vault
        ..=-1 => spl_token_transfer(TokenTransferParams {
            source: accounts.reward_token_vault_info.clone(),
            destination: accounts.lending_market_reward_token_account_info.clone(),
            amount: unallocated_rewards.unsigned_abs(),
            authority: accounts.reward_authority_info.clone(),
            authority_signer_seeds: &[
                reward_vault_authority_seeds(
                    accounts.lending_market_info.key,
                    &accounts.reserve.key(),
                    accounts.reward_mint_info.key,
                )
                .as_slice(),
                &[&[reward_authority_bump]],
            ]
            .concat(),
            token_program: accounts.token_program_info.clone(),
        }),
    }
}

impl<'a, 'info> EditPoolRewardAccounts<'a, 'info> {
    fn from_unchecked_iter(
        program_id: &Pubkey,
        bump: Bumps,
        iter: &mut impl Iterator<Item = &'a AccountInfo<'info>>,
    ) -> Result<EditPoolRewardAccounts<'a, 'info>, ProgramError> {
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
            bump,
            CheckAndUnpackPoolRewardAccounts {
                reserve_info,
                reward_mint_info,
                reward_authority_info,
                lending_market_info,
                token_program_info,
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

        // check that accounts that should be writable are writable

        if !reward_token_vault_info.is_writable {
            msg!("Reward token vault provided must be writable");
            return Err(ProgramError::InvalidAccountData);
        }
        if !reserve_info.is_writable {
            msg!("Reserve provided must be writable");
            return Err(ProgramError::InvalidAccountData);
        }
        if !reward_token_destination_info.is_writable {
            msg!("Reward token destination provided must be writable");
            return Err(ProgramError::InvalidAccountData);
        }

        Ok(Self {
            _reserve_info: reserve_info,
            reward_mint_info,
            lending_market_reward_token_account_info: reward_token_destination_info,
            reward_authority_info,
            reward_token_vault_info,
            lending_market_info,
            lending_market_owner_info,
            token_program_info,

            reserve,
        })
    }
}

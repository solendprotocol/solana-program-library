use crate::processor::liquidity_mining::{
    check_and_unpack_pool_reward_accounts_for_admin_ixs, unpack_token_account,
};
use crate::processor::{spl_token_transfer, TokenTransferParams};
use solana_program::program_pack::Pack;
use solana_program::sysvar::Sysvar;
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    clock::Clock,
    entrypoint::ProgramResult,
    msg,
    program_error::ProgramError,
    pubkey::Pubkey,
};
use solend_sdk::{
    error::LendingError,
    state::{PositionKind, Reserve},
};

use super::reward_vault_authority_seeds;

/// Use [Self::from_unchecked_iter] to validate the accounts.
struct CancelPoolRewardAccounts<'a, 'info> {
    /// ✅ belongs to this program
    /// ✅ unpacks
    /// ✅ belongs to `lending_market_info`
    /// ✅ is writable
    reserve_info: &'a AccountInfo<'info>,
    /// ✅ belongs to the token program
    reward_mint_info: &'a AccountInfo<'info>,
    /// ✅ belongs to the token program
    /// ✅ matches `reward_mint_info`
    /// ✅ is writable
    reward_token_destination_info: &'a AccountInfo<'info>,
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
    _lending_market_owner_info: &'a AccountInfo<'info>,
    /// ✅ matches `lending_market_info`
    token_program_info: &'a AccountInfo<'info>,

    reserve: Box<Reserve>,
}

/// # Effects
///
/// 1. Cancels any further reward emission, effectively setting end time to now.
/// 2. Transfers any unallocated rewards to the `reward_token_destination` account.
/// 3. Packs all changes into account buffers.
pub(crate) fn process(
    program_id: &Pubkey,
    position_kind: PositionKind,
    pool_reward_index: usize,
    accounts: &[AccountInfo],
) -> ProgramResult {
    let mut accounts =
        CancelPoolRewardAccounts::from_unchecked_iter(program_id, &mut accounts.iter())?;

    // 1.

    let pool_reward_manager = match position_kind {
        PositionKind::Borrow => &mut accounts.reserve.borrows_pool_reward_manager,
        PositionKind::Deposit => &mut accounts.reserve.deposits_pool_reward_manager,
    };
    let (expected_vault, unallocated_rewards) =
        pool_reward_manager.cancel_pool_reward(pool_reward_index, &Clock::get()?)?;

    if expected_vault != *accounts.reward_token_vault_info.key {
        msg!("Reward vault provided does not match the reward vault pubkey stored in [Reserve]");
        return Err(LendingError::InvalidAccountInput.into());
    }

    // 2.

    spl_token_transfer(TokenTransferParams {
        source: accounts.reward_token_vault_info.clone(),
        destination: accounts.reward_token_destination_info.clone(),
        amount: unallocated_rewards,
        authority: accounts.reward_authority_info.clone(),
        authority_signer_seeds: &reward_vault_authority_seeds(
            accounts.lending_market_info.key,
            accounts.reserve_info.key,
            accounts.reward_mint_info.key,
        ),
        token_program: accounts.token_program_info.clone(),
    })?;

    // 3.

    Reserve::pack(
        *accounts.reserve,
        &mut accounts.reserve_info.data.borrow_mut(),
    )?;

    Ok(())
}

impl<'a, 'info> CancelPoolRewardAccounts<'a, 'info> {
    fn from_unchecked_iter(
        program_id: &Pubkey,
        iter: &mut impl Iterator<Item = &'a AccountInfo<'info>>,
    ) -> Result<CancelPoolRewardAccounts<'a, 'info>, ProgramError> {
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
            reserve_info,
            reward_mint_info,
            reward_authority_info,
            lending_market_info,
            lending_market_owner_info,
            token_program_info,
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
            reserve_info,
            reward_mint_info,
            reward_token_destination_info,
            reward_authority_info,
            reward_token_vault_info,
            lending_market_info,
            _lending_market_owner_info: lending_market_owner_info,
            token_program_info,

            reserve,
        })
    }
}

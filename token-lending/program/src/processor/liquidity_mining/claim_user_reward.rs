use crate::processor::{spl_token_transfer, TokenTransferParams};
use solana_program::program_pack::Pack;
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    clock::Clock,
    entrypoint::ProgramResult,
    msg,
    program_error::ProgramError,
    pubkey::Pubkey,
    sysvar::Sysvar,
};
use solend_sdk::state::{CreatingNewUserRewardManager, Obligation};
use solend_sdk::{
    error::LendingError,
    state::{PositionKind, Reserve},
};

use super::{
    check_and_unpack_pool_reward_accounts, reward_vault_authority_seeds, unpack_token_account,
};

/// Use [Self::from_unchecked_iter] to validate the accounts.
struct ClaimUserReward<'a, 'info> {
    /// ✅ belongs to this program
    /// ✅ unpacks
    /// ✅ matches `lending_market_info`
    /// ✅ is writable
    obligation_info: &'a AccountInfo<'info>,
    /// ✅ belongs to the token program
    /// ✅ is writable
    /// ✅ matches `reward_mint_info`
    /// ✅ owned by the obligation owner
    obligation_owner_token_account_info: &'a AccountInfo<'info>,
    /// ✅ belongs to this program
    /// ✅ unpacks
    /// ✅ belongs to `lending_market_info`
    /// ✅ is writable
    reserve_info: &'a AccountInfo<'info>,
    /// ✅ belongs to the token program
    reward_mint_info: &'a AccountInfo<'info>,
    /// ✅ seed of `lending_market_info`, `reserve_info`, `reward_mint_info`
    reward_authority_info: &'a AccountInfo<'info>,
    /// ✅ belongs to the token program
    /// ✅ unpacks to a [TokenAccount]
    /// ✅ owned by `reward_authority_info`
    /// ✅ matches `reward_mint_info`
    /// ✅ is writable
    reward_token_vault_info: &'a AccountInfo<'info>,
    /// ✅ belongs to this program
    /// ✅ unpacks
    lending_market_info: &'a AccountInfo<'info>,
    /// ✅ matches `lending_market_info`
    token_program_info: &'a AccountInfo<'info>,

    obligation: Box<Obligation>,
    reserve: Box<Reserve>,
}

/// # Effects
///
/// 1. Updates the user reward manager with the pool reward manager and accrues rewards
/// 2. Withdraws all eligible rewards from [UserRewardManager].
///    Eligible rewards are those that match the vault and user has earned any.
/// 3. Transfers the withdrawn rewards to the user's token account.
/// 4. Packs all changes into account buffers for [Obligation] and [Reserve].
pub(crate) fn process(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let mut accounts = ClaimUserReward::from_unchecked_iter(program_id, &mut accounts.iter())?;

    // 1.

    let position_kind = accounts
        .obligation
        .find_position_kind(*accounts.reserve_info.key)?;

    let Some(user_reward_manager) = accounts
        .obligation
        .find_user_reward_manager_mut(*accounts.reserve_info.key)
    else {
        // Let's not error if a user has no rewards to claim for this reserve.
        // Having this ix idempotent makes cranking easier.
        return Ok(());
    };

    let pool_reward_manager = match position_kind {
        PositionKind::Borrow => &mut accounts.reserve.borrows_pool_reward_manager,
        PositionKind::Deposit => &mut accounts.reserve.deposits_pool_reward_manager,
    };

    let clock = &Clock::get()?;

    // Syncs the pool reward manager with the user manager and accrues rewards.
    // If we wanted to optimize CU usage then we could make a dedicated update
    // function only for claiming rewards to avoid iterating twice over the rewards.
    user_reward_manager.update(pool_reward_manager, clock, CreatingNewUserRewardManager::No)?;

    // 2.

    let total_reward_amount = user_reward_manager.claim_rewards(
        pool_reward_manager,
        *accounts.reward_token_vault_info.key,
        clock,
    )?;

    // 3.

    spl_token_transfer(TokenTransferParams {
        source: accounts.reward_token_vault_info.clone(),
        destination: accounts.obligation_owner_token_account_info.clone(),
        amount: total_reward_amount,
        authority: accounts.reward_authority_info.clone(),
        authority_signer_seeds: &reward_vault_authority_seeds(
            accounts.lending_market_info.key,
            accounts.reserve_info.key,
            accounts.reward_mint_info.key,
        ),
        token_program: accounts.token_program_info.clone(),
    })?;

    // 4.

    Obligation::pack(
        *accounts.obligation,
        &mut accounts.obligation_info.data.borrow_mut(),
    )?;

    Reserve::pack(
        *accounts.reserve,
        &mut accounts.reserve_info.data.borrow_mut(),
    )?;

    Ok(())
}

impl<'a, 'info> ClaimUserReward<'a, 'info> {
    fn from_unchecked_iter(
        program_id: &Pubkey,
        iter: &mut impl Iterator<Item = &'a AccountInfo<'info>>,
    ) -> Result<ClaimUserReward<'a, 'info>, ProgramError> {
        let obligation_info = next_account_info(iter)?;
        let obligation_owner_token_account_info = next_account_info(iter)?;
        let reserve_info = next_account_info(iter)?;
        let reward_mint_info = next_account_info(iter)?;
        let reward_authority_info = next_account_info(iter)?;
        let reward_token_vault_info = next_account_info(iter)?;
        let lending_market_info = next_account_info(iter)?;
        let token_program_info = next_account_info(iter)?;

        let (_, reserve) = check_and_unpack_pool_reward_accounts(
            program_id,
            reserve_info,
            reward_mint_info,
            reward_authority_info,
            lending_market_info,
            token_program_info,
        )?;

        if obligation_info.owner != program_id {
            msg!("Obligation provided is not owned by the lending program");
            return Err(LendingError::InvalidAccountOwner.into());
        }

        let obligation = Box::new(Obligation::unpack(&obligation_info.data.borrow())?);

        if obligation.lending_market != *lending_market_info.key {
            msg!("Obligation lending market does not match the lending market provided");
            return Err(LendingError::InvalidAccountInput.into());
        }

        if obligation_owner_token_account_info.owner != token_program_info.key {
            msg!("Obligation owner token account provided must be owned by the token program");
            return Err(LendingError::InvalidTokenOwner.into());
        }
        let obligation_owner_token_account =
            unpack_token_account(&obligation_owner_token_account_info.data.borrow())?;

        if obligation_owner_token_account.owner != obligation.owner {
            msg!(
                "Obligation owner token account owner does not match the obligation owner provided"
            );
            return Err(LendingError::InvalidAccountInput.into());
        }
        if obligation_owner_token_account.mint != *reward_mint_info.key {
            msg!("Obligation owner token account mint does not match the reward mint provided");
            return Err(LendingError::InvalidAccountInput.into());
        }

        if reward_token_vault_info.owner != token_program_info.key {
            msg!("Reward token vault provided must be owned by the token program");
            return Err(LendingError::InvalidTokenOwner.into());
        }
        let reward_token_vault = unpack_token_account(&reward_token_vault_info.data.borrow())?;

        if reward_token_vault.owner != *reward_authority_info.key {
            msg!("Reward token vault owner does not match the reward authority provided");
            return Err(LendingError::InvalidAccountInput.into());
        }
        if reward_token_vault.mint != *reward_mint_info.key {
            msg!("Reward token vault mint does not match the reward mint provided");
            return Err(LendingError::InvalidAccountInput.into());
        }

        // check that accounts that should be writable are writable

        if !obligation_info.is_writable {
            msg!("Obligation provided must be writable");
            return Err(ProgramError::InvalidAccountData);
        }
        if !obligation_owner_token_account_info.is_writable {
            msg!("Obligation owner token account provided must be writable");
            return Err(ProgramError::InvalidAccountData);
        }
        if !reward_token_vault_info.is_writable {
            msg!("Reward token vault provided must be writable");
            return Err(ProgramError::InvalidAccountData);
        }
        if !reserve_info.is_writable {
            msg!("Reserve provided must be writable");
            return Err(ProgramError::InvalidAccountData);
        }

        Ok(Self {
            obligation_info,
            obligation_owner_token_account_info,
            reserve_info,
            reward_mint_info,
            reward_authority_info,
            reward_token_vault_info,
            lending_market_info,
            token_program_info,

            reserve,
            obligation,
        })
    }
}

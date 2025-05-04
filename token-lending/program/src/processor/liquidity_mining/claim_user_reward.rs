//! Permission-less way to claim allocated user liquidity mining rewards.
//!
//! # Migration
//!
//! Prior to version @2.1.0 there was no concept of liq. mining.
//! That means user shares are going to be 0 even if they have a borrow or
//! deposit.
//! This ix can be used to start tracking obligation's rewards.

use crate::processor::{
    realloc_obligation_if_necessary, spl_token_transfer, ReserveBorrow, TokenTransferParams,
};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    clock::Clock,
    entrypoint::ProgramResult,
    msg,
    program_error::ProgramError,
    pubkey::Pubkey,
    sysvar::Sysvar,
};
use solend_sdk::state::{Obligation, PositionKind};
use solend_sdk::{error::LendingError, instruction::reward_vault_authority_seeds};

use super::{
    check_and_unpack_pool_reward_accounts, unpack_token_account, Bumps,
    CheckAndUnpackPoolRewardAccounts,
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
    _reserve_info: &'a AccountInfo<'info>,
    /// ✅ belongs to the token program
    _reward_mint_info: &'a AccountInfo<'info>,
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
    reserve: ReserveBorrow<'a, 'info>,
}

/// # Effects
///
/// 1. Finds the [UserRewardManager] for the reserve and obligation.
/// 2. Withdraws all eligible rewards from [UserRewardManager].
///    Eligible rewards are those that match the vault and user has earned any.
/// 3. Transfers the withdrawn rewards to the user's token account.
/// 4. Packs all changes into account buffers for [Obligation] and [Reserve].
pub(crate) fn process(
    program_id: &Pubkey,
    reward_authority_bump: u8,
    position_kind: PositionKind,
    accounts: &[AccountInfo],
) -> ProgramResult {
    let clock = &Clock::get()?;

    let mut accounts = ClaimUserReward::from_unchecked_iter(
        program_id,
        Bumps {
            reward_authority: reward_authority_bump,
        },
        &mut accounts.iter(),
    )?;
    let reserve_key = accounts.reserve.key();

    // 1.

    let pool_reward_manager = accounts.reserve.pool_reward_manager_mut(position_kind);

    if let Some(user_reward_manager) = accounts
        .obligation
        .user_reward_managers
        .find_mut(reserve_key, position_kind)
    {
        msg!(
            "Found user reward manager that was last updated at {} and has {}/{} shares",
            user_reward_manager.last_update_time_secs,
            user_reward_manager.share,
            pool_reward_manager.total_shares
        );

        // 2.

        let total_reward_amount = user_reward_manager.claim_rewards(
            pool_reward_manager,
            *accounts.reward_token_vault_info.key,
            clock,
        )?;

        // 3.

        if total_reward_amount > 0 {
            spl_token_transfer(TokenTransferParams {
                source: accounts.reward_token_vault_info.clone(),
                destination: accounts.obligation_owner_token_account_info.clone(),
                amount: total_reward_amount,
                authority: accounts.reward_authority_info.clone(),
                authority_signer_seeds: &[
                    reward_vault_authority_seeds(
                        accounts.lending_market_info.key,
                        accounts.reward_token_vault_info.key,
                    )
                    .as_slice(),
                    &[&[reward_authority_bump]],
                ]
                .concat(),
                token_program: accounts.token_program_info.clone(),
            })?;
        }
    } else {
        let expected_position_kind = accounts.obligation.find_position_kind(reserve_key)?;

        if expected_position_kind != position_kind {
            msg!("Obligation does not have {:?} for reserve", position_kind);
            return Err(LendingError::InvalidAccountInput.into());
        }

        // We've checked that the obligation associates this reserve but it's
        // not in the user reward managers yet.
        // This means that the obligation hasn't been migrated to track the
        // pool reward manager.
        //
        // We'll upgrade it here.

        let migrated_share = match position_kind {
            PositionKind::Borrow => accounts
                .obligation
                .find_liquidity_in_borrows(reserve_key)?
                .0
                .liability_shares()?,
            PositionKind::Deposit => {
                accounts
                    .obligation
                    .find_collateral_in_deposits(reserve_key)?
                    .0
                    .deposited_amount
            }
        };

        msg!(
            "Migrating obligation to track pool reward manager with share of {}/{}",
            migrated_share,
            pool_reward_manager.total_shares
        );

        accounts.obligation.user_reward_managers.set_share(
            reserve_key,
            position_kind,
            pool_reward_manager,
            migrated_share,
            clock,
        )?;
    };

    // 4.

    realloc_obligation_if_necessary(&accounts.obligation, accounts.obligation_info)?;
    Obligation::pack(
        *accounts.obligation,
        &mut accounts.obligation_info.data.borrow_mut(),
    )?;

    // reserve is packed on drop

    Ok(())
}

impl<'a, 'info> ClaimUserReward<'a, 'info> {
    fn from_unchecked_iter(
        program_id: &Pubkey,
        bumps: Bumps,
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
            bumps,
            CheckAndUnpackPoolRewardAccounts {
                reserve_info,
                reward_mint_info,
                reward_authority_info,
                lending_market_info,
                token_program_info,
                reward_token_vault_info,
            },
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
            _reserve_info: reserve_info,
            _reward_mint_info: reward_mint_info,
            reward_authority_info,
            reward_token_vault_info,
            lending_market_info,
            token_program_info,

            reserve,
            obligation,
        })
    }
}

use crate::processor::{
    assert_rent_exempt, spl_token_init_account, spl_token_transfer, TokenInitializeAccountParams,
    TokenTransferParams,
};
use solana_program::program_pack::Pack;
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
use solend_sdk::state::MIN_REWARD_PERIOD_SECS;
use solend_sdk::{
    error::LendingError,
    state::{PositionKind, Reserve},
};
use std::convert::TryInto;

use super::{check_and_unpack_pool_reward_accounts_for_admin_ixs, unpack_token_account};

/// Use [Self::new] to validate the parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AddPoolRewardParams {
    position_kind: PositionKind,
    /// At least the current timestamp.
    start_time_secs: u64,
    /// Larger than [MIN_REWARD_PERIOD_SECS].
    duration_secs: u32,
    /// Larger than zero.
    reward_token_amount: u64,
}

/// Use [Self::from_unchecked_iter] to validate the accounts except for
/// * `reward_token_vault_info`
/// * `rent_info`
struct AddPoolRewardAccounts<'a, 'info> {
    /// ✅ belongs to this program
    /// ✅ unpacks
    /// ✅ belongs to `lending_market_info`
    /// ✅ is writable
    reserve_info: &'a AccountInfo<'info>,
    /// ✅ belongs to the token program
    reward_mint_info: &'a AccountInfo<'info>,
    /// ✅ belongs to the token program
    /// ✅ owned by `lending_market_owner_info`
    /// ✅ has enough tokens
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
    /// TBD: do we want to create another signer authority to be able to
    /// delegate reward management to a softer multisig?
    lending_market_owner_info: &'a AccountInfo<'info>,
    /// ❓ we don't yet whether this is rent info
    rent_info: &'a AccountInfo<'info>,
    /// ✅ matches `lending_market_info`
    token_program_info: &'a AccountInfo<'info>,

    reserve: Box<Reserve>,
}

/// # Effects
///
/// 1. Initializes a new reward vault account and transfers
///    `reward_token_amount` tokens from the `reward_token_source` account to
///     the new reward vault account.
/// 2. Finds an empty slot in the [Reserve]'s LM reward vector and adds it there.
/// 3. Packs all changes into account buffers.
pub(crate) fn process(
    program_id: &Pubkey,
    position_kind: PositionKind,
    start_time_secs: u64,
    end_time_secs: u64,
    reward_token_amount: u64,
    accounts: &[AccountInfo],
) -> ProgramResult {
    let params = AddPoolRewardParams::new(
        position_kind,
        start_time_secs,
        end_time_secs,
        reward_token_amount,
    )?;

    let accounts =
        AddPoolRewardAccounts::from_unchecked_iter(program_id, &params, &mut accounts.iter())?;

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
        amount: params.reward_token_amount,
        authority: accounts.lending_market_owner_info.clone(),
        authority_signer_seeds: &[],
        token_program: accounts.token_program_info.clone(),
    })?;

    // 2.

    // TODO: accounts.reserve.add_pool_reward(..)

    // 3.

    Reserve::pack(
        *accounts.reserve,
        &mut accounts.reserve_info.data.borrow_mut(),
    )?;

    Ok(())
}

impl AddPoolRewardParams {
    fn new(
        position_kind: PositionKind,
        start_time_secs: u64,
        end_time_secs: u64,
        reward_token_amount: u64,
    ) -> Result<Self, ProgramError> {
        let clock = &Clock::get()?;

        let start_time_secs = start_time_secs.max(clock.unix_timestamp as u64);

        if start_time_secs <= end_time_secs {
            msg!("Pool reward must end after it starts");
            return Err(LendingError::MathOverflow.into());
        }

        let duration_secs: u32 = {
            // SAFETY: just checked that start time is strictly smaller
            let d = end_time_secs - start_time_secs;
            d.try_into().map_err(|_| {
                msg!("Pool reward duration is too long");
                LendingError::MathOverflow
            })?
        };
        if MIN_REWARD_PERIOD_SECS > duration_secs as u64 {
            msg!("Pool reward duration must be at least {MIN_REWARD_PERIOD_SECS} secs");
            return Err(LendingError::PoolRewardPeriodTooShort.into());
        }

        if reward_token_amount == 0 {
            msg!("Pool reward amount must be greater than zero");
            return Err(LendingError::InvalidAmount.into());
        }

        Ok(Self {
            position_kind,
            start_time_secs,
            duration_secs,
            reward_token_amount,
        })
    }
}

impl<'a, 'info> AddPoolRewardAccounts<'a, 'info> {
    fn from_unchecked_iter(
        program_id: &Pubkey,
        params: &AddPoolRewardParams,
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
        if reward_token_source.amount >= params.reward_token_amount {
            msg!("Reward token source is empty");
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
        if !reward_token_vault_info.data.borrow().is_empty() {
            msg!("Reward token vault provided must be empty");
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
        if !reward_token_source_info.is_writable {
            msg!("Reward token source provided must be writable");
            return Err(ProgramError::InvalidAccountData);
        }

        Ok(Self {
            reserve_info,
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

use crate::processor::{
    assert_rent_exempt, spl_token_close_account, spl_token_init_account, spl_token_transfer,
    TokenCloseAccountParams, TokenInitializeAccountParams, TokenTransferParams,
};
use add_pool_reward::{AddPoolRewardAccounts, AddPoolRewardParams};
use cancel_pool_reward::{CancelPoolRewardAccounts, CancelPoolRewardParams};
use close_pool_reward::{ClosePoolRewardAccounts, ClosePoolRewardParams};
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
use solend_sdk::{
    error::LendingError,
    state::{LendingMarket, PositionKind, Reserve},
};
use spl_token::state::Account as TokenAccount;
use std::convert::TryInto;

/// Cannot create a reward shorter than this.
const MIN_REWARD_PERIOD_SECS: u64 = 3_600;

/// # Accounts
///
/// See [add_pool_reward::AddPoolRewardAccounts::from_unchecked_iter] for a list
/// of accounts and their constraints.
///
/// # Effects
///
/// 1. Initializes a new reward vault account and transfers
///    `reward_token_amount` tokens from the `reward_token_source` account to
///     the new reward vault account.
/// 2. Finds an empty slot in the [Reserve]'s LM reward vector and adds it there.
/// 3. Packs all changes into account buffers.
pub(crate) fn process_add_pool_reward(
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

    todo!("accounts.reserve.add_pool_reward(..)");

    // 3.

    Reserve::pack(
        *accounts.reserve,
        &mut accounts.reserve_info.data.borrow_mut(),
    )?;

    Ok(())
}

/// # Accounts
///
/// See [cancel_pool_reward::CancelPoolRewardAccounts::from_unchecked_iter] for a list
/// of accounts and their constraints.
///
/// # Effects
///
/// 1. Cancels any further reward emission, effectively setting end time to now.
/// 2. Transfers any unallocated rewards to the `reward_token_destination` account.
/// 3. Packs all changes into account buffers.
pub(crate) fn process_cancel_pool_reward(
    program_id: &Pubkey,
    position_kind: PositionKind,
    pool_reward_index: u64,
    accounts: &[AccountInfo],
) -> ProgramResult {
    let params = CancelPoolRewardParams::new(position_kind, pool_reward_index);

    let accounts =
        CancelPoolRewardAccounts::from_unchecked_iter(program_id, &params, &mut accounts.iter())?;

    // 1.

    let unallocated_rewards = todo!("accounts.reserve.cancel_pool_reward(..)");

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

/// # Accounts
///
/// See [close_pool_reward::ClosePoolRewardAccounts::from_unchecked_iter] for a list
/// of accounts and their constraints.
///
/// # Effects
///
/// 1. Closes reward in the [Reserve] account if all users have claimed.
/// 2. Transfers any unallocated rewards to the `reward_token_destination` account.
/// 3. Closes reward vault token account.
/// 3. Packs all changes into account buffers.
pub(crate) fn process_close_pool_reward(
    program_id: &Pubkey,
    position_kind: PositionKind,
    pool_reward_index: u64,
    accounts: &[AccountInfo],
) -> ProgramResult {
    let params = ClosePoolRewardParams::new(position_kind, pool_reward_index);

    let accounts =
        ClosePoolRewardAccounts::from_unchecked_iter(program_id, &params, &mut accounts.iter())?;

    // 1.

    let unallocated_rewards = todo!("accounts.reserve.close_pool_reward(..)");

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

    spl_token_close_account(TokenCloseAccountParams {
        account: accounts.reward_token_vault_info.clone(),
        destination: accounts.lending_market_owner_info.clone(),
        authority: accounts.reward_authority_info.clone(),
        authority_signer_seeds: &reward_vault_authority_seeds(
            accounts.lending_market_info.key,
            accounts.reserve_info.key,
            accounts.reward_mint_info.key,
        ),
        token_program: accounts.token_program_info.clone(),
    })?;

    // 4.
    Reserve::pack(
        *accounts.reserve,
        &mut accounts.reserve_info.data.borrow_mut(),
    )?;

    Ok(())
}

/// Unpacks a spl_token [TokenAccount].
fn unpack_token_account(data: &[u8]) -> Result<TokenAccount, LendingError> {
    TokenAccount::unpack(data).map_err(|_| LendingError::InvalidTokenAccount)
}

/// Derives the reward vault authority PDA address.
fn reward_vault_authority(
    program_id: &Pubkey,
    lending_market_key: &Pubkey,
    reserve_key: &Pubkey,
    reward_mint_key: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &reward_vault_authority_seeds(lending_market_key, reserve_key, reward_mint_key),
        program_id,
    )
}

fn reward_vault_authority_seeds<'keys>(
    lending_market_key: &'keys Pubkey,
    reserve_key: &'keys Pubkey,
    reward_mint_key: &'keys Pubkey,
) -> [&'keys [u8]; 4] {
    [
        b"RewardVaultAuthority",
        lending_market_key.as_ref(),
        reserve_key.as_ref(),
        reward_mint_key.as_ref(),
    ]
}

mod add_pool_reward {
    use super::*;

    /// Use [Self::new] to validate the parameters.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) struct AddPoolRewardParams {
        pub(super) position_kind: PositionKind,
        /// At least the current timestamp.
        pub(super) start_time_secs: u64,
        /// Larger than [MIN_REWARD_PERIOD_SECS].
        pub(super) duration_secs: u32,
        /// Larger than zero.
        pub(super) reward_token_amount: u64,

        _priv: (),
    }

    /// Use [Self::from_unchecked_iter] to validate the accounts except for
    /// * `reward_token_vault_info`
    /// * `rent_info`
    pub(super) struct AddPoolRewardAccounts<'a, 'info> {
        /// ✅ belongs to this program
        /// ✅ unpacks
        /// ✅ belongs to `lending_market_info`
        pub(super) reserve_info: &'a AccountInfo<'info>,
        pub(super) reward_mint_info: &'a AccountInfo<'info>,
        /// ✅ belongs to the token program
        /// ✅ owned by `lending_market_owner_info`
        /// ✅ has enough tokens
        /// ✅ matches `reward_mint_info`
        pub(super) reward_token_source_info: &'a AccountInfo<'info>,
        /// ✅ seed of `lending_market_info`, `reserve_info`, `reward_mint_info`
        pub(super) reward_authority_info: &'a AccountInfo<'info>,
        /// ✅ belongs to the token program
        /// ✅ has no data
        /// ❓ we don't yet know whether it's rent exempt
        pub(super) reward_token_vault_info: &'a AccountInfo<'info>,
        /// ✅ belongs to this program
        /// ✅ unpacks
        pub(super) lending_market_info: &'a AccountInfo<'info>,
        /// ✅ is a signer
        /// ✅ matches `lending_market_info`
        /// TBD: do we want to create another signer authority to be able to
        /// delegate reward management to a softer multisig?
        pub(super) lending_market_owner_info: &'a AccountInfo<'info>,
        /// ❓ we don't yet whether this is rent info
        pub(super) rent_info: &'a AccountInfo<'info>,
        /// ✅ matches `lending_market_info`
        pub(super) token_program_info: &'a AccountInfo<'info>,

        pub(super) reserve: Box<Reserve>,

        _priv: (),
    }

    impl AddPoolRewardParams {
        pub(super) fn new(
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
                return Err(LendingError::PoolRewardTooShort.into());
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

                _priv: (),
            })
        }
    }

    impl<'a, 'info> AddPoolRewardAccounts<'a, 'info> {
        pub(super) fn from_unchecked_iter(
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

            let reserve = check_pool_reward_accounts_for_admin_ixs_and_unpack_reserve(
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
            let reward_token_source =
                unpack_token_account(&reward_token_source_info.data.borrow())?;
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

            Ok(Self {
                reserve_info,
                reward_mint_info,
                reward_token_source_info,
                reward_authority_info,
                reward_token_vault_info,
                lending_market_info,
                lending_market_owner_info,
                rent_info,
                token_program_info,

                reserve,

                _priv: (),
            })
        }
    }
}

mod cancel_pool_reward {
    use super::*;

    pub(super) struct CancelPoolRewardParams {
        position_kind: PositionKind,
        pool_reward_index: u64,

        _priv: (),
    }

    /// Use [Self::from_unchecked_iter] to validate the accounts.
    pub(super) struct CancelPoolRewardAccounts<'a, 'info> {
        /// ✅ belongs to this program
        /// ✅ unpacks
        /// ✅ belongs to `lending_market_info`
        pub(super) reserve_info: &'a AccountInfo<'info>,
        pub(super) reward_mint_info: &'a AccountInfo<'info>,
        /// ✅ belongs to the token program
        /// ✅ owned by `lending_market_owner_info`
        /// ✅ matches `reward_mint_info`
        pub(super) reward_token_destination_info: &'a AccountInfo<'info>,
        /// ✅ seed of `lending_market_info`, `reserve_info`, `reward_mint_info`
        pub(super) reward_authority_info: &'a AccountInfo<'info>,
        /// ✅ matches reward vault pubkey stored in the [Reserve]
        pub(super) reward_token_vault_info: &'a AccountInfo<'info>,
        /// ✅ belongs to this program
        /// ✅ unpacks
        pub(super) lending_market_info: &'a AccountInfo<'info>,
        /// ✅ is a signer
        /// ✅ matches `lending_market_info`
        pub(super) lending_market_owner_info: &'a AccountInfo<'info>,
        /// ✅ matches `lending_market_info`
        pub(super) token_program_info: &'a AccountInfo<'info>,

        pub(super) reserve: Box<Reserve>,

        _priv: (),
    }

    impl<'a, 'info> CancelPoolRewardAccounts<'a, 'info> {
        pub(super) fn from_unchecked_iter(
            program_id: &Pubkey,
            params: &CancelPoolRewardParams,
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

            let reserve = check_pool_reward_accounts_for_admin_ixs_and_unpack_reserve(
                program_id,
                reserve_info,
                reward_mint_info,
                reward_authority_info,
                lending_market_info,
                lending_market_owner_info,
                token_program_info,
            )?;

            todo!("Check that reward_token_vault_info matches reward vault pubkey stored in [Reserve]");

            if reward_token_destination_info.owner != token_program_info.key {
                msg!("Reward token destination provided must be owned by the token program");
                return Err(LendingError::InvalidTokenOwner.into());
            }
            let reward_token_destination =
                unpack_token_account(&reward_token_destination_info.data.borrow())?;
            if reward_token_destination.owner != *lending_market_owner_info.key {
                // TBD: superfluous check?
                msg!("Reward token destination owner does not match the lending market owner provided");
                return Err(LendingError::InvalidAccountInput.into());
            }
            if reward_token_destination.mint != *reward_mint_info.key {
                msg!("Reward token destination mint does not match the reward mint provided");
                return Err(LendingError::InvalidAccountInput.into());
            }

            Ok(Self {
                _priv: (),

                reserve_info,
                reward_mint_info,
                reward_token_destination_info,
                reward_authority_info,
                reward_token_vault_info,
                lending_market_info,
                lending_market_owner_info,
                token_program_info,

                reserve,
            })
        }
    }

    impl CancelPoolRewardParams {
        pub(super) fn new(position_kind: PositionKind, pool_reward_index: u64) -> Self {
            Self {
                position_kind,
                pool_reward_index,

                _priv: (),
            }
        }
    }
}

mod close_pool_reward {
    use super::*;

    pub(super) struct ClosePoolRewardParams {
        position_kind: PositionKind,
        pool_reward_index: u64,

        _priv: (),
    }

    /// Use [Self::from_unchecked_iter] to validate the accounts.
    pub(super) struct ClosePoolRewardAccounts<'a, 'info> {
        _priv: (),

        /// ✅ belongs to this program
        /// ✅ unpacks
        /// ✅ belongs to `lending_market_info`
        pub(super) reserve_info: &'a AccountInfo<'info>,
        pub(super) reward_mint_info: &'a AccountInfo<'info>,
        /// ✅ belongs to the token program
        /// ✅ owned by `lending_market_owner_info`
        /// ✅ matches `reward_mint_info`
        pub(super) reward_token_destination_info: &'a AccountInfo<'info>,
        /// ✅ seed of `lending_market_info`, `reserve_info`, `reward_mint_info`
        pub(super) reward_authority_info: &'a AccountInfo<'info>,
        /// ✅ matches reward vault pubkey stored in the [Reserve]
        pub(super) reward_token_vault_info: &'a AccountInfo<'info>,
        /// ✅ belongs to this program
        /// ✅ unpacks
        pub(super) lending_market_info: &'a AccountInfo<'info>,
        /// ✅ is a signer
        /// ✅ matches `lending_market_info`
        pub(super) lending_market_owner_info: &'a AccountInfo<'info>,
        /// ✅ matches `lending_market_info`
        pub(super) token_program_info: &'a AccountInfo<'info>,

        pub(super) reserve: Box<Reserve>,
    }

    impl<'a, 'info> ClosePoolRewardAccounts<'a, 'info> {
        pub(super) fn from_unchecked_iter(
            program_id: &Pubkey,
            params: &ClosePoolRewardParams,
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

            let reserve = check_pool_reward_accounts_for_admin_ixs_and_unpack_reserve(
                program_id,
                reserve_info,
                reward_mint_info,
                reward_authority_info,
                lending_market_info,
                lending_market_owner_info,
                token_program_info,
            )?;

            todo!("Check that reward_token_vault_info matches reward vault pubkey stored in [Reserve]");

            if reward_token_destination_info.owner != token_program_info.key {
                msg!("Reward token destination provided must be owned by the token program");
                return Err(LendingError::InvalidTokenOwner.into());
            }
            let reward_token_destination =
                unpack_token_account(&reward_token_destination_info.data.borrow())?;
            if reward_token_destination.owner != *lending_market_owner_info.key {
                // TBD: superfluous check?
                msg!("Reward token destination owner does not match the lending market owner provided");
                return Err(LendingError::InvalidAccountInput.into());
            }
            if reward_token_destination.mint != *reward_mint_info.key {
                msg!("Reward token destination mint does not match the reward mint provided");
                return Err(LendingError::InvalidAccountInput.into());
            }

            Ok(Self {
                reserve_info,
                reward_mint_info,
                reward_token_destination_info,
                reward_authority_info,
                reward_token_vault_info,
                lending_market_info,
                lending_market_owner_info,
                token_program_info,

                reserve,

                _priv: (),
            })
        }
    }

    impl ClosePoolRewardParams {
        pub(super) fn new(position_kind: PositionKind, pool_reward_index: u64) -> Self {
            Self {
                position_kind,
                pool_reward_index,

                _priv: (),
            }
        }
    }
}

/// Common checks within the admin ixs are:
///
/// * ✅ `reserve_info` belongs to this program
/// * ✅ `reserve_info` unpacks
/// * ✅ `reserve_info` belongs to `lending_market_info`
/// * ✅ `reward_authority_info` is seed of `lending_market_info`, `reserve_info`, `reward_mint_info`
/// * ✅ `lending_market_info` belongs to this program
/// * ✅ `lending_market_info` unpacks
/// * ✅ `lending_market_owner_info` is a signer
/// * ✅ `lending_market_owner_info` matches `lending_market_info`
/// * ✅ `token_program_info` matches `lending_market_info`
///
/// To avoid unpacking reserve twice we return it.
fn check_pool_reward_accounts_for_admin_ixs_and_unpack_reserve<'info>(
    program_id: &Pubkey,
    reserve_info: &AccountInfo<'info>,
    reward_mint_info: &AccountInfo<'info>,
    reward_authority_info: &AccountInfo<'info>,
    lending_market_info: &AccountInfo<'info>,
    lending_market_owner_info: &AccountInfo<'info>,
    token_program_info: &AccountInfo<'info>,
) -> Result<Box<Reserve>, ProgramError> {
    if reserve_info.owner != program_id {
        msg!("Reserve provided is not owned by the lending program");
        return Err(LendingError::InvalidAccountOwner.into());
    }
    let reserve = Box::new(Reserve::unpack(&reserve_info.data.borrow())?);

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

    if lending_market.owner != *lending_market_owner_info.key {
        msg!("Lending market owner does not match the lending market owner provided");
        return Err(LendingError::InvalidMarketOwner.into());
    }
    if !lending_market_owner_info.is_signer {
        msg!("Lending market owner provided must be a signer");
        return Err(LendingError::InvalidSigner.into());
    }

    let (expected_reward_vault_authority, _bump_seed) = reward_vault_authority(
        program_id,
        lending_market_info.key,
        reserve_info.key,
        reward_mint_info.key,
    );
    if expected_reward_vault_authority != *reward_authority_info.key {
        msg!("Reward vault authority does not match the expected value");
        return Err(LendingError::InvalidAccountInput.into());
    }

    Ok(reserve)
}

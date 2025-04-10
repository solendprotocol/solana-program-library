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
/// * ✅ `reward_authority_info` is seed of `lending_market_info`, `reserve_info`, `reward_mint_info`
/// * ✅ `lending_market_info` belongs to this program
/// * ✅ `lending_market_info` unpacks
/// * ✅ `token_program_info` matches `lending_market_info`
/// * ✅ `reward_mint_info` belongs to the token program
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

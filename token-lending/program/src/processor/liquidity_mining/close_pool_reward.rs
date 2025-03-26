//! Closes a pool reward, making its slot vacant and ready for a new reward.
//!
//! Before closing a pool reward that pool reward must first be cancelled
//! and all rewards must be claimed by the users.
//!
//! The claim ix is permission-less and therefore it can be cranked.

use solana_program::program_pack::Pack;
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    msg,
    program_error::ProgramError,
    pubkey::Pubkey,
};
use solend_sdk::state::LendingMarket;
use solend_sdk::{
    error::LendingError,
    state::{PositionKind, Reserve},
};

/// Use [Self::from_unchecked_iter] to validate the accounts.
struct ClosePoolRewardAccounts<'a, 'info> {
    /// ✅ belongs to this program
    /// ✅ unpacks
    /// ✅ belongs to `lending_market_info`
    /// ✅ is writable
    reserve_info: &'a AccountInfo<'info>,
    /// ✅ belongs to this program
    /// ✅ unpacks
    _lending_market_info: &'a AccountInfo<'info>,
    /// ✅ is a signer
    /// ✅ matches `lending_market_info` owner
    _lending_market_owner_info: &'a AccountInfo<'info>,

    reserve: Box<Reserve>,
}

/// # Effects
///
/// 1. Closes reward in the [Reserve] account if all users have claimed.
/// 2. Packs all changes into account buffers.
pub(crate) fn process(
    program_id: &Pubkey,
    position_kind: PositionKind,
    pool_reward_index: usize,
    accounts: &[AccountInfo],
) -> ProgramResult {
    let mut accounts =
        ClosePoolRewardAccounts::from_unchecked_iter(program_id, &mut accounts.iter())?;

    // 1.

    let pool_reward_manager = match position_kind {
        PositionKind::Borrow => &mut accounts.reserve.borrows_pool_reward_manager,
        PositionKind::Deposit => &mut accounts.reserve.deposits_pool_reward_manager,
    };
    pool_reward_manager.close_pool_reward(pool_reward_index)?;

    // 2.
    Reserve::pack(
        *accounts.reserve,
        &mut accounts.reserve_info.data.borrow_mut(),
    )?;

    Ok(())
}

impl<'a, 'info> ClosePoolRewardAccounts<'a, 'info> {
    fn from_unchecked_iter(
        program_id: &Pubkey,
        iter: &mut impl Iterator<Item = &'a AccountInfo<'info>>,
    ) -> Result<ClosePoolRewardAccounts<'a, 'info>, ProgramError> {
        let reserve_info = next_account_info(iter)?;
        let lending_market_info = next_account_info(iter)?;
        let lending_market_owner_info = next_account_info(iter)?;

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

        if lending_market.owner != *lending_market_owner_info.key {
            msg!("Lending market owner does not match the lending market owner provided");
            return Err(LendingError::InvalidMarketOwner.into());
        }
        if !lending_market_owner_info.is_signer {
            msg!("Lending market owner provided must be a signer");
            return Err(LendingError::InvalidSigner.into());
        }

        // check that accounts that should be writable are writable

        if !reserve_info.is_writable {
            msg!("Reserve provided must be writable");
            return Err(ProgramError::InvalidAccountData);
        }

        Ok(Self {
            reserve_info,
            _lending_market_info: lending_market_info,
            _lending_market_owner_info: lending_market_owner_info,

            reserve,
        })
    }
}

use solana_program::program_pack::Pack;
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    msg,
    program::invoke,
    program_error::ProgramError,
    pubkey::Pubkey,
    rent::Rent,
    system_instruction,
    sysvar::Sysvar,
};
use solend_sdk::state::discriminator::AccountDiscriminator;
use solend_sdk::state::RESERVE_LEN_V2_0_2;
use solend_sdk::{error::LendingError, state::Reserve};

struct UpgradeReserveAccounts<'a, 'info> {
    /// Reserve sized as v2.0.2.
    ///
    /// ✅ belongs to this program
    /// ✅ is sized [RESERVE_LEN_V2_0_2], ie. for sure [Reserve] account
    /// ✅ is writable
    reserve_info: &'a AccountInfo<'info>,
    /// The pool fella who pays for this.
    ///
    /// ✅ is a signer
    /// ✅ is writable
    payer: &'a AccountInfo<'info>,
    /// The system program.
    ///
    /// ✅ is the system program
    system_program: &'a AccountInfo<'info>,
}

/// Temporary ix to upgrade a reserve to LM feature added in @v2.0.2.
/// Fails if reserve was not sized as @v2.0.2.
///
/// Until this ix is called for a [Reserve] account, all other ixs that try to
/// unpack the [Reserve] will fail due to size mismatch.
///
/// # Effects
///
/// 1. Takes payer's lamports and pays for the rent increase.
/// 2. Reallocates the reserve account to the latest size.
/// 3. Repacks the reserve account.
pub(crate) fn process(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let accounts = UpgradeReserveAccounts::from_unchecked_iter(program_id, &mut accounts.iter())?;

    // 1.

    let current_rent = accounts.reserve_info.lamports();
    let new_rent = Rent::get()?.minimum_balance(Reserve::LEN);

    if let Some(extra_rent) = new_rent.checked_sub(current_rent) {
        // some reserves have more rent than necessary, let's not assume that
        // the payer always needs to add more rent

        invoke(
            &system_instruction::transfer(
                accounts.payer.key,
                accounts.reserve_info.key,
                extra_rent,
            ),
            &[
                accounts.payer.clone(),
                accounts.reserve_info.clone(),
                accounts.system_program.clone(),
            ],
        )?;
    }

    // 2.

    // From the [AccountInfo::realloc] docs:
    //
    // > Memory used to grow is already zero-initialized upon program entrypoint
    // > and re-zeroing it wastes compute units. If within the same call a program
    // > reallocs from larger to smaller and back to larger again the new space
    // > could contain stale data. Pass true for zero_init in this case,
    // > otherwise compute units will be wasted re-zero-initializing.
    let zero_init = false;
    accounts.reserve_info.realloc(Reserve::LEN, zero_init)?;

    // 3.

    // we upgrade discriminator as we've checked that the account is indeed
    // a reserve account in [UpgradeReserveAccounts::from_unchecked_iter]
    let mut data = accounts.reserve_info.data.borrow_mut();
    data[0] = AccountDiscriminator::Reserve as u8;
    // Now the reserve can unpack fine and doesn't have to worry about
    // migrations.
    // Instead it returns an error on an invalid discriminator.
    // This way a reserve cannot be mistaken for an obligation.
    let reserve = Reserve::unpack(&data)?;
    Reserve::pack(reserve, &mut data)?;

    Ok(())
}

impl<'a, 'info> UpgradeReserveAccounts<'a, 'info> {
    fn from_unchecked_iter(
        program_id: &Pubkey,
        iter: &mut impl Iterator<Item = &'a AccountInfo<'info>>,
    ) -> Result<UpgradeReserveAccounts<'a, 'info>, ProgramError> {
        let reserve_info = next_account_info(iter)?;
        let payer = next_account_info(iter)?;
        let system_program = next_account_info(iter)?;

        if !payer.is_signer {
            msg!("Payer provided must be a signer");
            return Err(LendingError::InvalidSigner.into());
        }

        if reserve_info.owner != program_id {
            msg!("Reserve provided must be owned by the lending program");
            return Err(LendingError::InvalidAccountOwner.into());
        }

        if reserve_info.data_len() != RESERVE_LEN_V2_0_2 {
            msg!("Reserve provided must be sized as v2.0.2");
            return Err(LendingError::InvalidAccountInput.into());
        }

        if system_program.key != &solana_program::system_program::id() {
            msg!("System program provided must be the system program");
            return Err(LendingError::InvalidAccountInput.into());
        }

        // check that accounts that should be writable are writable

        if !payer.is_writable {
            msg!("Payer provided must be writable");
            return Err(ProgramError::InvalidAccountData);
        }
        if !reserve_info.is_writable {
            msg!("Reserve provided must be writable");
            return Err(ProgramError::InvalidAccountData);
        }

        Ok(Self {
            payer,
            reserve_info,
            system_program,
        })
    }
}

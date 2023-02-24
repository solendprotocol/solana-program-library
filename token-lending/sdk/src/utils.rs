//! helper functions used in sdk and program
use crate::error::LendingError;
use bytemuck::Pod;
use solana_program::{
    account_info::AccountInfo, program_error::ProgramError, program_pack::IsInitialized,
};
use std::{
    cell::{Ref, RefMut},
    result::Result,
};

/// Borrow the data in 'account' as a value of type 'T'.
pub fn load_account<'a, T: Pod>(account: &'a AccountInfo) -> Result<Ref<'a, T>, ProgramError> {
    let data = account.try_borrow_data()?;

    Ok(Ref::map(data, |data| {
        bytemuck::from_bytes(&data[0..std::mem::size_of::<T>()])
    }))
}

/// Mutably borrow the data in `account` as a value of type `T`.
/// Any mutations to the returned value will be reflected in the account data.
pub fn load_account_as_mut<'a, T: Pod>(
    account: &'a AccountInfo,
) -> Result<RefMut<'a, T>, ProgramError> {
    let data = account.try_borrow_mut_data()?;

    Ok(RefMut::map(data, |data| {
        bytemuck::from_bytes_mut(&mut data[0..std::mem::size_of::<T>()])
    }))
}

/// Borrow the data in 'account' as a value of type 'T' and make sure it is initialized.
pub fn load_initialized_account<'a, T: Pod + IsInitialized>(
    account: &'a AccountInfo,
) -> Result<Ref<'a, T>, ProgramError> {
    let obj = load_account::<T>(account)?;
    if !obj.is_initialized() {
        return Err(ProgramError::UninitializedAccount);
    }

    Ok(obj)
}

/// Mutably borrow the data in 'account' as a value of type 'T' and make sure it is initialized.
pub fn load_initialized_account_as_mut<'a, T: Pod + IsInitialized>(
    account: &'a AccountInfo,
) -> Result<RefMut<'a, T>, ProgramError> {
    let obj = load_account_as_mut::<T>(account)?;
    if !obj.is_initialized() {
        return Err(ProgramError::UninitializedAccount);
    }

    Ok(obj)
}

/// Mutably borrow the data in 'account' as a value of type 'T' and make sure it is uninitialized.
pub fn load_uninitialized_account_as_mut<'a, T: Pod + IsInitialized>(
    account: &'a AccountInfo,
) -> Result<RefMut<'a, T>, ProgramError> {
    let obj = load_account_as_mut::<T>(account)?;
    if obj.is_initialized() {
        return Err(LendingError::AlreadyInitialized.into());
    }

    Ok(obj)
}

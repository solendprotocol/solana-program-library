//! # Why do we wrap account data?
//!
//! Previous version of borrow-lending implementation unpacked and then packed
//! the account data several times in a single ix to avoid errors of overwriting
//! data written by other functions.
//! However, this was still fragile as all function calls between unpack and
//! pack would have to be checked to ensure they do not write to the same data.
//!
//! Instead we now have a convention that data access is created in the
//! `process_*` functions and are passed as a reference to other functions.
//!
//! This structure guarantees at runtime that the double write error does not
//! occur while avoiding the cost of unpacking and packing the data.

use crate::{error::LendingError, state::Reserve};
use solana_program::{
    account_info::AccountInfo, entrypoint::ProgramResult, msg, program_error::ProgramError,
    program_pack::Pack, pubkey::Pubkey,
};

use std::ops::{Deref, DerefMut};
use std::result::Result;

/// Wraps around a [Reserve] data and provides runtime borrow semantics.
///
/// Is either in state of
/// - `release`. The underlying data is not borrowed at all and can be read and
///    written to by other holders of the account info.
/// - `Ref`. The underlying data is borrowed as immutable and can be read by
///    other holders of the account info but not written to.
/// - `RefMut`. The underlying data is borrowed as mutable and can be read and
///    written only via this borrow.
///
/// # Persistence
///
/// The data is written to the underlying account buffer when the borrow is
/// done mutably with either [Self::new_mut] or [Self::acquire_reload_mut].
/// The write happens on [Self::release] or in any function that calls it and on
/// [drop].
pub(crate) struct ReserveBorrow<'a, 'info> {
    info: &'a AccountInfo<'info>,
    guard: ReserveDataGuard<'a, 'info>,
}

enum ReserveDataGuard<'a, 'info> {
    Released,
    Ref(
        #[allow(dead_code)] std::cell::Ref<'a, &'info mut [u8]>,
        Box<Reserve>,
    ),
    RefMut(std::cell::RefMut<'a, &'info mut [u8]>, Box<Reserve>),
}

enum ReserveDataGuardKind {
    Release,
    Ref,
    RefMut,
}

impl Drop for ReserveBorrow<'_, '_> {
    fn drop(&mut self) {
        if let Err(e) = self.release() {
            msg!("Failed to release reserve data");
            panic!("{}", e);
        }
    }
}

impl Deref for ReserveBorrow<'_, '_> {
    type Target = Box<Reserve>;

    fn deref(&self) -> &Self::Target {
        match &self.guard {
            ReserveDataGuard::Ref(_, inner) => inner,
            ReserveDataGuard::RefMut(_, inner) => inner,
            ReserveDataGuard::Released => panic!("Reserve data has been released"),
        }
    }
}

impl DerefMut for ReserveBorrow<'_, '_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match &mut self.guard {
            ReserveDataGuard::RefMut(_, inner) => inner,
            ReserveDataGuard::Ref(_, _) => panic!("Reserve data is not mutable"),
            ReserveDataGuard::Released => panic!("Reserve data has been released"),
        }
    }
}

impl<'a, 'info> ReserveBorrow<'a, 'info> {
    /// Creates a new `Ref` guard over the data.
    ///
    /// Many readers can exist at the same time if no writer is present,
    /// otherwise panics.
    pub(crate) fn new(
        program_id: &Pubkey,
        info: &'a AccountInfo<'info>,
    ) -> Result<Self, ProgramError> {
        if info.owner != program_id {
            msg!("Reserve provided is not owned by the lending program");
            return Err(LendingError::InvalidAccountOwner.into());
        }

        let data = info.data.borrow();
        let reserve = Box::new(Reserve::unpack(&data)?);
        let guard = ReserveDataGuard::Ref(data, reserve);

        Ok(Self { guard, info })
    }

    /// Creates a new `RefMut` guard over the data.
    ///
    /// Only one writer can exist at a time and no readers, otherwise panics.
    pub(crate) fn new_mut(
        program_id: &Pubkey,
        info: &'a AccountInfo<'info>,
    ) -> Result<Self, ProgramError> {
        if info.owner != program_id {
            msg!("Reserve provided is not owned by the lending program");
            return Err(LendingError::InvalidAccountOwner.into());
        }

        let data = info.data.borrow_mut();
        let reserve = Box::new(Reserve::unpack(&data)?);
        let guard = ReserveDataGuard::RefMut(data, reserve);

        Ok(Self { guard, info })
    }

    pub(crate) fn key(&self) -> Pubkey {
        *self.info.key
    }

    /// Explicit version of [drop]ping that panics if the data is not guarded
    /// as `RefMut`.
    pub(crate) fn commit(self) {
        if let ReserveDataGuard::RefMut(_, _) = self.guard {
            // drop self
        } else {
            panic!("Cannot commit a non mutable borrow");
        }
    }

    /// Releases the guard over the data.
    ///
    /// If the data was guarded as `RefMut`, it will be packed back to the
    /// account.
    pub(crate) fn release(&mut self) -> ProgramResult {
        let prev_guard = std::mem::replace(&mut self.guard, ReserveDataGuard::Released);

        if let ReserveDataGuard::RefMut(mut data, inner) = prev_guard {
            Reserve::pack(*inner, &mut data)?;
        }

        Ok(())
    }

    /// Calls [Self::release] and then creates a new `RefMut` guard.
    pub(crate) fn acquire_reload_mut(&mut self) -> ProgramResult {
        self.release()?;

        let data_ref = self.info.data.borrow_mut();
        let inner = Reserve::unpack(&data_ref)?;
        self.guard = ReserveDataGuard::RefMut(data_ref, Box::new(inner));

        Ok(())
    }

    /// Calls [Self::release] and then creates a new `RefMut` guard.
    pub(crate) fn acquire_reload(&mut self) -> ProgramResult {
        self.release()?;

        let data_ref = self.info.data.borrow();
        let inner = Reserve::unpack(&data_ref)?;
        self.guard = ReserveDataGuard::Ref(data_ref, Box::new(inner));

        Ok(())
    }

    /// Releases the guard, calls the given function and returns the guard to
    /// the same state it was before the call.
    pub(crate) fn while_released<T>(
        &mut self,
        f: impl FnOnce() -> Result<T, ProgramError>,
    ) -> Result<T, ProgramError> {
        let prev_guard = ReserveDataGuardKind::from(&self.guard);
        self.release()?;

        let res = f();

        match prev_guard {
            ReserveDataGuardKind::Ref => {
                self.acquire_reload()?;
            }
            ReserveDataGuardKind::RefMut => {
                self.acquire_reload_mut()?;
            }
            ReserveDataGuardKind::Release => {
                // already released
            }
        }

        res
    }
}

impl From<&'_ ReserveDataGuard<'_, '_>> for ReserveDataGuardKind {
    fn from(guard: &'_ ReserveDataGuard) -> Self {
        match guard {
            ReserveDataGuard::Released => ReserveDataGuardKind::Release,
            ReserveDataGuard::Ref(_, _) => ReserveDataGuardKind::Ref,
            ReserveDataGuard::RefMut(_, _) => ReserveDataGuardKind::RefMut,
        }
    }
}

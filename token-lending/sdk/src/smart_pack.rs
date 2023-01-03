//! Wrapper trait that can deserialize multiple versions of an object, and can re-alloc space if needed
use solana_program::{
    account_info::AccountInfo,
    program_error::ProgramError,
    program_pack::{IsInitialized, Pack},
};

use crate::{error::LendingError, state::UNINITIALIZED_VERSION};

/// Wrapper trait that can deserialize multiple versions of an object, and can re-alloc space if
/// needed
pub trait SmartPack<V1: Pack + IsInitialized> {
    /// Find version of the object from the bytes representation
    fn version(src: &[u8]) -> Option<u8> {
        src.iter().next().copied()
    }

    /// Check if object is initialized from the bytes representation
    fn is_initialized(src: &[u8]) -> bool {
        !matches!(Self::version(src), None | Some(UNINITIALIZED_VERSION))
    }

    /// Unpack object from slice and check if initialized
    fn smart_unpack(src: &[u8]) -> Result<V1, LendingError> {
        match Self::version(src) {
            None => Err(LendingError::FailedToDeserialize),
            Some(UNINITIALIZED_VERSION) => {
                // msg!("Can't unpack an uninitialized object!");
                Err(LendingError::FailedToDeserialize)
            }
            Some(1) => match V1::unpack(src) {
                Err(_e) => Err(LendingError::FailedToDeserialize),
                Ok(object) => Ok(object),
            },
            Some(_v) => {
                // msg!("Unimplemented version detected: {}", v);
                Err(LendingError::FailedToDeserialize)
            }
        }
    }

    /// Pack into slice
    fn smart_pack(
        object: V1,
        version: u8,
        dst_account_info: &AccountInfo,
    ) -> Result<(), ProgramError> {
        match version {
            1 => V1::pack(object, &mut dst_account_info.try_borrow_mut_data()?),
            _v => {
                // msg!("Unimplemented pack version detected: {}", v);
                Err(LendingError::FailedToSerialize.into())
            }
        }
    }
}

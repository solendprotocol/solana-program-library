//! Wrapper trait that can deserialize multiple versions of an object, and can re-alloc space if needed
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{
    account_info::AccountInfo,
    program_error::ProgramError,
    program_pack::{IsInitialized, Pack},
};

use crate::{error::LendingError, state::UNINITIALIZED_VERSION};

/// Wrapper trait that can deserialize multiple versions of an object, and can re-alloc space if
/// needed
pub trait SmartPack<
    V1: Pack + IsInitialized + From<V2>,
    V2: BorshSerialize + BorshDeserialize + From<V1> + ValidateTag,
>
{
    /// Find version of the object from the bytes representation
    fn version(src: &[u8]) -> u8 {
        match src.iter().next() {
            // it's ok if the data buffer is _currently_ empty because we re-allocate in smart_pack
            None => UNINITIALIZED_VERSION,
            Some(v) => *v,
        }
    }

    /// Check if object is initialized from the bytes representation
    fn is_initialized(src: &[u8]) -> bool {
        Self::version(src) != UNINITIALIZED_VERSION
    }

    /// Unpack object from slice and check if initialized
    fn smart_unpack(src: &[u8]) -> Result<V2, LendingError> {
        match Self::version(src) {
            UNINITIALIZED_VERSION => {
                // msg!("Can't unpack an uninitialized object!");
                Err(LendingError::FailedToDeserialize)
            }
            1 => match V1::unpack(src) {
                Err(_e) => Err(LendingError::FailedToDeserialize),
                Ok(object) => Ok(object.into()),
            },
            2 => match V2::try_from_slice(src) {
                Ok(object) => {
                    object.validate_tag()?;
                    Ok(object)
                }
                Err(_e) => {
                    // msg!("failed to borsh deserialize {:?}", e);
                    Err(LendingError::FailedToDeserialize)
                }
            },
            _v => {
                // msg!("Unimplemented version detected: {}", v);
                Err(LendingError::FailedToDeserialize)
            }
        }
    }

    /// Pack into slice. Re-alloc if the AccountInfo's data buffer is too small.
    fn smart_pack(
        object: V2,
        version: u8,
        dst_account_info: &AccountInfo,
    ) -> Result<(), ProgramError> {
        match version {
            1 => V1::pack(object.into(), &mut dst_account_info.try_borrow_mut_data()?),
            2 => {
                // serialize into a vector first
                let serialized = object.try_to_vec().map_err(|_e| {
                    // msg!("failed to borsh serialize: {:?}", e);
                    LendingError::FailedToSerialize
                })?;

                // 1. always realloc because try_from_slice will error on buffer len mismatches
                // 2. zero-init out of paranoia but i don't think we actually need this
                dst_account_info.realloc(serialized.len(), true)?;

                // copy_from_slice panics if the sizes of the two slices don't match.
                // in this case, we're guaranteed to not panic because we just realloc'd the account
                let mut dst = dst_account_info.try_borrow_mut_data()?;
                dst.copy_from_slice(&serialized);

                Ok(())
            }
            _v => {
                // msg!("Unimplemented pack version detected: {}", v);
                Err(LendingError::FailedToSerialize.into())
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
/// Tag used to identify type of structure when deserializing
pub enum AccountTag {
    /// Uninitialized
    UnInitialized,
    /// LendingMarket
    LendingMarket,
    /// Reserve
    Reserve,
    /// Obligation
    Obligation,
}

impl Default for AccountTag {
    fn default() -> Self {
        AccountTag::UnInitialized
    }
}

/// trait used to make sure the deserialized object has the correct tag.
pub trait ValidateTag {
    /// Returns a LendingError if the tag is incorrect.
    fn validate_tag(&self) -> Result<(), LendingError>;
}

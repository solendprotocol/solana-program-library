//! Wrapper trait that can deserialize multiple versions of an object, and can re-alloc space if needed
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{account_info::AccountInfo, program_error::ProgramError};

/// Wrapper trait that can deserialize multiple versions of an object, and can re-alloc space if
/// needed
pub trait SmartPack {
    /// Type of item that is being packed/unpacked.
    type Item;

    /// Find version of object using the serialized representation
    fn version(src: &[u8]) -> u8;

    /// Check if a program account state is initialized
    fn is_initialized(src: &[u8]) -> bool;

    /// Unpack from slice and check if initialized
    fn smart_unpack(src: &[u8]) -> Result<Self::Item, ProgramError>;

    /// Pack into slice. Re-alloc if the AccountInfo's data buffer is too small.
    fn smart_pack(src: Self::Item, dst_account_info: &AccountInfo) -> Result<(), ProgramError>;
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

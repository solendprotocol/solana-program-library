#![deny(missing_docs)]

//! A lending program for the Solana blockchain.

pub mod entrypoint;
pub mod error;
pub mod instruction;
pub mod math;
pub mod processor;
pub mod pyth;
pub mod state;

// Export current sdk types for downstream users building with a different sdk version
pub use solana_program;

solana_program::declare_id!("So1endDq2YkqhipRh3WViPa8hdiSpxWy6z3Z6tMCpAo");

/// Canonical null pubkey. Prints out as "nu11111111111111111111111111111111111111111"
pub const NULL_PUBKEY: solana_program::pubkey::Pubkey =
    solana_program::pubkey::Pubkey::new_from_array([
        11, 193, 238, 216, 208, 116, 241, 195, 55, 212, 76, 22, 75, 202, 40, 216, 76, 206, 27, 169,
        138, 64, 177, 28, 19, 90, 156, 0, 0, 0, 0, 0,
    ]);

/// Mainnet program id for Switchboard v2. Prints out as "SW1TCH7qEPTdLsDHRgPuMQjbQxKdH2aBStViMFnt64f"
pub const SWITCHBOARD_V2_MAINNET: solana_program::pubkey::Pubkey =
    solana_program::pubkey::Pubkey::new_from_array([
        6, 136, 81, 198, 140, 104, 50, 240, 47, 165, 129, 177, 191, 73, 27, 119, 202, 65, 119, 107,
        162, 185, 136, 181, 166, 250, 186, 142, 227, 162, 236, 144,
    ]);

/// Devnet program id for Switchboard v2. Prints out as "2TfB33aLaneQb5TNVwyDz3jSZXS6jdW2ARw1Dgf84XCG"
pub const SWITCHBOARD_V2_DEVNET: solana_program::pubkey::Pubkey =
    solana_program::pubkey::Pubkey::new_from_array([
        21, 175, 243, 73, 45, 68, 245, 12, 42, 213, 156, 141, 129, 194, 65, 181, 115, 202, 11, 225,
        119, 62, 247, 42, 73, 206, 175, 81, 212, 253, 178, 45,
    ]);

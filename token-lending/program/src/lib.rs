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
pub use std::str::FromStr;

solana_program::declare_id!("So1endDq2YkqhipRh3WViPa8hdiSpxWy6z3Z6tMCpAo");

/// Canonical null pubkey. Prints out as "nu11111111111111111111111111111111111111111"
pub const NULL_PUBKEY: solana_program::pubkey::Pubkey =
    solana_program::pubkey::Pubkey::new_from_array([
        11, 193, 238, 216, 208, 116, 241, 195, 55, 212, 76, 22, 75, 202, 40, 216, 76, 206, 27, 169,
        138, 64, 177, 28, 19, 90, 156, 0, 0, 0, 0, 0,
    ]);
         
/// whitelist liquidator wallets
pub const WHITELIST_LIQUIDATORS : [solana_program::pubkey::Pubkey; 3] = [
    // F4q2bm6k4AW2rgQMBfvoNnfn1xFkoD98We6uJd6tV7tq
    solana_program::pubkey::Pubkey::new_from_array([
        208, 254, 170,  18, 176, 113,  47,  78,
        0, 176, 134,  93, 161,  19,  98, 161,
        114, 185, 140, 222,  45, 158, 143, 251,
        53, 114,  83, 154, 201, 207, 178, 198
    ]),
    // owoD8aRRvKZXRfDiGvXYc18gfQ5cQBcKgEXFk6PCTva
    solana_program::pubkey::Pubkey::new_from_array([
        12,   6, 173,  17, 230,  35, 81, 149,
        182, 240, 121,  19, 165, 180, 38, 237,
        34,  50, 245,   8, 230, 123, 18, 122,
        114,  60, 166, 205, 191, 247, 88, 195
    ]),
    // 8i3ufSbnCDi3ZGyGJxDco26AQTGB5G81G1SBsUD55mK6  this is for tests
    solana_program::pubkey::Pubkey::new_from_array([
        114, 133, 232, 227,  86,  67, 182,  15,
        253,  36, 214,  87, 201,  19, 105, 189,
        111, 157, 211, 250,  12, 167, 115,  73,
        3, 116, 254,  73, 245,  75, 104, 105
    ])
];
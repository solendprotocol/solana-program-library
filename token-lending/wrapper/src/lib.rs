#![deny(missing_docs)]

//! A brick.

pub use solana_program;

solana_program::declare_id!("2eEso2sAipRHNZ54d4fRJyeC6mVJq73F5mvsL1wZb3tp");

pub mod entrypoint;
pub mod processor;

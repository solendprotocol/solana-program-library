#![allow(dead_code)]

pub mod flash_loan_proxy;
pub mod flash_loan_receiver;
pub mod genesis;
pub mod mock_pyth;
pub mod mock_switchboard;
pub mod solend_program_test;

use bytemuck::{cast_slice_mut, from_bytes_mut, try_cast_slice_mut, Pod, PodCastError};

use solana_program::{program_option::COption, program_pack::Pack, pubkey::Pubkey};
use solana_program_test::*;
use solana_sdk::{
    account::Account,
    signature::{Keypair, Signer},
};
use solend_program::state::{ReserveConfig, ReserveFees, ReserveType};

use spl_token::state::Mint;

use std::mem::size_of;
use switchboard_v2::AggregatorAccountData;

pub const QUOTE_CURRENCY: [u8; 32] =
    *b"USD\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";

pub const LAMPORTS_TO_SOL: u64 = 1_000_000_000;
pub const FRACTIONAL_TO_USDC: u64 = 1_000_000;

pub fn reserve_config_no_fees() -> ReserveConfig {
    ReserveConfig {
        optimal_utilization_rate: 80,
        max_utilization_rate: 80,
        loan_to_value_ratio: 50,
        liquidation_bonus: 0,
        max_liquidation_bonus: 0,
        liquidation_threshold: 55,
        max_liquidation_threshold: 65,
        min_borrow_rate: 0,
        optimal_borrow_rate: 0,
        max_borrow_rate: 0,
        super_max_borrow_rate: 0,
        fees: ReserveFees {
            borrow_fee_wad: 0,
            flash_loan_fee_wad: 0,
            host_fee_percentage: 0,
        },
        deposit_limit: u64::MAX,
        borrow_limit: u64::MAX,
        fee_receiver: Keypair::new().pubkey(),
        protocol_liquidation_fee: 0,
        protocol_take_rate: 0,
        added_borrow_weight_bps: 0,
        reserve_type: ReserveType::Regular,
        scaled_price_offset_bps: 0,
        extra_oracle_pubkey: None,
        attributed_borrow_limit_open: u64::MAX,
        attributed_borrow_limit_close: u64::MAX,
    }
}

pub fn test_reserve_config() -> ReserveConfig {
    ReserveConfig {
        optimal_utilization_rate: 80,
        max_utilization_rate: 80,
        loan_to_value_ratio: 50,
        liquidation_bonus: 4,
        max_liquidation_bonus: 4,
        liquidation_threshold: 55,
        max_liquidation_threshold: 65,
        min_borrow_rate: 0,
        optimal_borrow_rate: 4,
        max_borrow_rate: 30,
        super_max_borrow_rate: 30,
        fees: ReserveFees {
            borrow_fee_wad: 0,
            flash_loan_fee_wad: 0,
            host_fee_percentage: 0,
        },
        deposit_limit: u64::MAX,
        borrow_limit: u64::MAX,
        fee_receiver: Keypair::new().pubkey(),
        protocol_liquidation_fee: 10,
        protocol_take_rate: 0,
        added_borrow_weight_bps: 0,
        reserve_type: ReserveType::Regular,
        scaled_price_offset_bps: 0,
        extra_oracle_pubkey: None,
        attributed_borrow_limit_open: u64::MAX,
        attributed_borrow_limit_close: u64::MAX,
    }
}

pub mod usdc_mint {
    solana_program::declare_id!("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
}

pub mod usdt_mint {
    solana_program::declare_id!("Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB");
}

pub mod wsol_mint {
    // fake mint, not the real wsol bc i can't mint wsol programmatically
    solana_program::declare_id!("So1m5eppzgokXLBt9Cg8KCMPWhHfTzVaGh26Y415MRG");
}

pub mod msol_mint {
    // fake mint, not the real wsol bc i can't mint wsol programmatically
    solana_program::declare_id!("mSoLzYCxHdYgdzU16g5QSh3i5K3z3KZK7ytfqcJm7So");
}

pub mod bonk_mint {
    solana_program::declare_id!("bonk99WdRCGrh56xQaeQuRMpMHgiNZEfVoZ53DJAoHS");
}

pub trait AddPacked {
    fn add_packable_account<T: Pack>(
        &mut self,
        pubkey: Pubkey,
        amount: u64,
        data: &T,
        owner: &Pubkey,
    );
}

impl AddPacked for ProgramTest {
    fn add_packable_account<T: Pack>(
        &mut self,
        pubkey: Pubkey,
        amount: u64,
        data: &T,
        owner: &Pubkey,
    ) {
        let mut account = Account::new(amount, T::get_packed_len(), owner);
        data.pack_into_slice(&mut account.data);
        self.add_account(pubkey, account);
    }
}

pub struct TestMint {
    pub pubkey: Pubkey,
    pub authority: Keypair,
    pub decimals: u8,
}

pub fn load_mut<T: Pod>(data: &mut [u8]) -> Result<&mut T, PodCastError> {
    let size = size_of::<T>();
    Ok(from_bytes_mut(cast_slice_mut::<u8, u8>(
        try_cast_slice_mut(&mut data[0..size])?,
    )))
}

fn add_mint(test: &mut ProgramTest, mint: Pubkey, decimals: u8, authority: Pubkey) {
    test.add_packable_account(
        mint,
        u32::MAX as u64,
        &Mint {
            is_initialized: true,
            mint_authority: COption::Some(authority),
            decimals,
            ..Mint::default()
        },
        &spl_token::id(),
    );
}

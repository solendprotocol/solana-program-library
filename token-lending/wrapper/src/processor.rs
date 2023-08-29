//! Program state processor

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::PUBKEY_BYTES;
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    instruction::{AccountMeta, Instruction},
    msg,
    program::{invoke, invoke_signed},
    program_error::ProgramError,
    program_pack::{IsInitialized, Pack},
    pubkey::Pubkey,
    system_instruction::create_account,
    sysvar::instructions::{load_current_index_checked, load_instruction_at_checked},
    sysvar::{
        clock::{self, Clock},
        rent::Rent,
        Sysvar,
    },
};
use thiserror::Error;
use num_derive::FromPrimitive;
use num_traits::FromPrimitive;
use solend_sdk::instruction::liquidate_obligation_and_redeem_reserve_collateral;

/// Instruction types
#[derive(BorshSerialize, BorshDeserialize)]
pub enum WrapperInstruction {
    /// Accounts:
    /// 0: PriceAccount (uninitialized)
    /// 1: ProductAccount (uninitialized)
    LiquidateWithoutReceivingCtokens {
        /// amount to liquidate
        liquidity_amount: u64,
    },
}

/// Processes an instruction
pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    input: &[u8],
) -> ProgramResult {
    let instruction = WrapperInstruction::try_from_slice(input)?;
    match instruction {
        WrapperInstruction::LiquidateWithoutReceivingCtokens { liquidity_amount } => {
            msg!("Instruction: LiquidateWithoutReceivingCtokens");
            let account_info_iter = &mut accounts.iter();
            let solend_program_info = next_account_info(account_info_iter)?;
            let source_liquidity_info = next_account_info(account_info_iter)?;
            let destination_collateral_info = next_account_info(account_info_iter)?;
            let destination_liquidity_info = next_account_info(account_info_iter)?;
            let repay_reserve_info = next_account_info(account_info_iter)?;
            let repay_reserve_liquidity_supply_info = next_account_info(account_info_iter)?;
            let withdraw_reserve_info = next_account_info(account_info_iter)?;
            let withdraw_reserve_collateral_mint_info = next_account_info(account_info_iter)?;
            let withdraw_reserve_collateral_supply_info = next_account_info(account_info_iter)?;
            let withdraw_reserve_liquidity_supply_info = next_account_info(account_info_iter)?;
            let withdraw_reserve_liquidity_fee_receiver_info =
                next_account_info(account_info_iter)?;
            let obligation_info = next_account_info(account_info_iter)?;
            let lending_market_info = next_account_info(account_info_iter)?;
            let lending_market_authority_info = next_account_info(account_info_iter)?;
            let user_transfer_authority_info = next_account_info(account_info_iter)?;
            let token_program_id = next_account_info(account_info_iter)?;

            // print all info variables
            msg!("solend_program_info: {}", solend_program_info.key);
            msg!("source_liquidity_info: {}", source_liquidity_info.key);
            msg!(
                "destination_collateral_info: {}",
                destination_collateral_info.key
            );
            msg!(
                "destination_liquidity_info: {}",
                destination_liquidity_info.key
            );
            msg!("repay_reserve_info: {}", repay_reserve_info.key);
            msg!(
                "repay_reserve_liquidity_supply_info: {}",
                repay_reserve_liquidity_supply_info.key
            );
            msg!("withdraw_reserve_info: {}", withdraw_reserve_info.key);
            msg!(
                "withdraw_reserve_collateral_mint_info: {}",
                withdraw_reserve_collateral_mint_info.key
            );
            msg!(
                "withdraw_reserve_collateral_supply_info: {}",
                withdraw_reserve_collateral_supply_info.key
            );
            msg!(
                "withdraw_reserve_liquidity_supply_info: {}",
                withdraw_reserve_liquidity_supply_info.key
            );
            msg!(
                "withdraw_reserve_liquidity_fee_receiver_info: {}",
                withdraw_reserve_liquidity_fee_receiver_info.key
            );
            msg!("obligation_info: {}", obligation_info.key);
            msg!("lending_market_info: {}", lending_market_info.key);
            msg!(
                "lending_market_authority_info: {}",
                lending_market_authority_info.key
            );
            msg!(
                "user_transfer_authority_info: {}",
                user_transfer_authority_info.key
            );
            msg!("token_program_id: {}", token_program_id.key);

            let instruction = liquidate_obligation_and_redeem_reserve_collateral(
                *solend_program_info.key,
                liquidity_amount,
                *source_liquidity_info.key,
                *destination_collateral_info.key,
                *destination_liquidity_info.key,
                *repay_reserve_info.key,
                *repay_reserve_liquidity_supply_info.key,
                *withdraw_reserve_info.key,
                *withdraw_reserve_collateral_mint_info.key,
                *withdraw_reserve_collateral_supply_info.key,
                *withdraw_reserve_liquidity_supply_info.key,
                *withdraw_reserve_liquidity_fee_receiver_info.key,
                *obligation_info.key,
                *lending_market_info.key,
                *user_transfer_authority_info.key,
            );

            let account_infos = [
                solend_program_info.clone(),
                source_liquidity_info.clone(),
                destination_collateral_info.clone(),
                destination_liquidity_info.clone(),
                repay_reserve_info.clone(),
                repay_reserve_liquidity_supply_info.clone(),
                withdraw_reserve_info.clone(),
                withdraw_reserve_collateral_mint_info.clone(),
                withdraw_reserve_collateral_supply_info.clone(),
                withdraw_reserve_liquidity_supply_info.clone(),
                withdraw_reserve_liquidity_fee_receiver_info.clone(),
                obligation_info.clone(),
                lending_market_info.clone(),
                lending_market_authority_info.clone(),
                user_transfer_authority_info.clone(),
                token_program_id.clone(),
            ];

            let ctoken_balance_before = spl_token::state::Account::unpack_from_slice(
                &destination_collateral_info.try_borrow_data()?,
            )?
            .amount;

            invoke(&instruction, &account_infos)?;

            let ctoken_balance_after = spl_token::state::Account::unpack_from_slice(
                &destination_collateral_info.try_borrow_data()?,
            )?
            .amount;

            if ctoken_balance_after > ctoken_balance_before {
                msg!("We received ctokens, aborting");
                return Err(WrapperError::ReceivedCTokens.into());
            }
        }
    }
    Ok(())
}


/// Errors that may be returned by the TokenLending program.
#[derive(Clone, Debug, Eq, Error, FromPrimitive, PartialEq)]
pub enum WrapperError {
    /// Received ctokens
    #[error("Received ctokens")]
    ReceivedCTokens
}

impl From<WrapperError> for ProgramError {
    fn from(e: WrapperError) -> Self {
        ProgramError::Custom(e as u32)
    }
}

/// Creates a `LiquidateObligationAndRedeemReserveCollateral` instruction
#[allow(clippy::too_many_arguments)]
pub fn liquidate_without_receiving_ctokens(
    program_id: Pubkey,
    liquidity_amount: u64,
    solend_program_id: Pubkey,
    source_liquidity_pubkey: Pubkey,
    destination_collateral_pubkey: Pubkey,
    destination_liquidity_pubkey: Pubkey,
    repay_reserve_pubkey: Pubkey,
    repay_reserve_liquidity_supply_pubkey: Pubkey,
    withdraw_reserve_pubkey: Pubkey,
    withdraw_reserve_collateral_mint_pubkey: Pubkey,
    withdraw_reserve_collateral_supply_pubkey: Pubkey,
    withdraw_reserve_liquidity_supply_pubkey: Pubkey,
    withdraw_reserve_liquidity_fee_receiver_pubkey: Pubkey,
    obligation_pubkey: Pubkey,
    lending_market_pubkey: Pubkey,
    user_transfer_authority_pubkey: Pubkey,
) -> Instruction {
    let (lending_market_authority_pubkey, _bump_seed) = Pubkey::find_program_address(
        &[&lending_market_pubkey.to_bytes()[..PUBKEY_BYTES]],
        &solend_program_id,
    );
    Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new_readonly(solend_program_id, false),
            AccountMeta::new(source_liquidity_pubkey, false),
            AccountMeta::new(destination_collateral_pubkey, false),
            AccountMeta::new(destination_liquidity_pubkey, false),
            AccountMeta::new(repay_reserve_pubkey, false),
            AccountMeta::new(repay_reserve_liquidity_supply_pubkey, false),
            AccountMeta::new(withdraw_reserve_pubkey, false),
            AccountMeta::new(withdraw_reserve_collateral_mint_pubkey, false),
            AccountMeta::new(withdraw_reserve_collateral_supply_pubkey, false),
            AccountMeta::new(withdraw_reserve_liquidity_supply_pubkey, false),
            AccountMeta::new(withdraw_reserve_liquidity_fee_receiver_pubkey, false),
            AccountMeta::new(obligation_pubkey, false),
            AccountMeta::new(lending_market_pubkey, false),
            AccountMeta::new_readonly(lending_market_authority_pubkey, false),
            AccountMeta::new_readonly(user_transfer_authority_pubkey, true),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: WrapperInstruction::LiquidateWithoutReceivingCtokens { liquidity_amount }
            .try_to_vec()
            .unwrap(),
    }
}

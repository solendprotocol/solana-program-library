//! Program state processor

use borsh::{BorshDeserialize, BorshSerialize};
use num_derive::FromPrimitive;
use solana_program::pubkey::PUBKEY_BYTES;
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    instruction::{AccountMeta, Instruction},
    msg,
    program::invoke,
    program_error::ProgramError,
    program_pack::Pack,
    pubkey::Pubkey,
};
use solend_sdk::instruction::{
    deposit_reserve_liquidity_and_obligation_collateral,
    liquidate_obligation_and_redeem_reserve_collateral, repay_obligation_liquidity,
};
use solend_sdk::math::Decimal;
use solend_sdk::math::SaturatingSub;
use solend_sdk::state::Reserve;
use thiserror::Error;

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
    /// Repay obligation liquidity with max amount in token account
    RepayMax,
    /// Deposit max
    DepositMax,
}

/// Processes an instruction
pub fn process_instruction(
    _program_id: &Pubkey,
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

            Ok(())
        }
        WrapperInstruction::RepayMax => {
            msg!("Instruction: RepayMax");
            let account_info_iter = &mut accounts.iter();
            let solend_program_id = next_account_info(account_info_iter)?;
            let source_liquidity_info = next_account_info(account_info_iter)?;
            let destination_liquidity_info = next_account_info(account_info_iter)?;
            let repay_reserve_info = next_account_info(account_info_iter)?;
            let obligation_info = next_account_info(account_info_iter)?;
            let lending_market_info = next_account_info(account_info_iter)?;
            let user_transfer_authority_info = next_account_info(account_info_iter)?;
            let token_program_id = next_account_info(account_info_iter)?;

            let source_liquidity_balance = spl_token::state::Account::unpack_from_slice(
                &source_liquidity_info.try_borrow_data()?,
            )?
            .amount;
            msg!("source_liquidity_balance: {}", source_liquidity_balance);

            let instruction = repay_obligation_liquidity(
                *solend_program_id.key,
                source_liquidity_balance,
                *source_liquidity_info.key,
                *destination_liquidity_info.key,
                *repay_reserve_info.key,
                *obligation_info.key,
                *lending_market_info.key,
                *user_transfer_authority_info.key,
            );

            invoke(
                &instruction,
                &[
                    solend_program_id.clone(),
                    source_liquidity_info.clone(),
                    destination_liquidity_info.clone(),
                    repay_reserve_info.clone(),
                    obligation_info.clone(),
                    lending_market_info.clone(),
                    user_transfer_authority_info.clone(),
                    token_program_id.clone(),
                ],
            )?;

            Ok(())
        }
        WrapperInstruction::DepositMax => {
            msg!("Instruction: DepositMax");
            let account_info_iter = &mut accounts.iter();
            let solend_program_id = next_account_info(account_info_iter)?;
            let source_liquidity_info = next_account_info(account_info_iter)?;
            let user_collateral_info = next_account_info(account_info_iter)?;
            let reserve_info = next_account_info(account_info_iter)?;
            let reserve_liquidity_supply_info = next_account_info(account_info_iter)?;
            let reserve_collateral_mint_info = next_account_info(account_info_iter)?;
            let lending_market_info = next_account_info(account_info_iter)?;
            let lending_market_authority_info = next_account_info(account_info_iter)?;
            let destination_collateral_info = next_account_info(account_info_iter)?;
            let obligation_info = next_account_info(account_info_iter)?;
            let obligation_owner_info = next_account_info(account_info_iter)?;
            let pyth_price_info = next_account_info(account_info_iter)?;
            let switchboard_feed_info = next_account_info(account_info_iter)?;
            let user_transfer_authority_info = next_account_info(account_info_iter)?;
            let token_program_id = next_account_info(account_info_iter)?;

            let source_liquidity_balance = spl_token::state::Account::unpack_from_slice(
                &source_liquidity_info.try_borrow_data()?,
            )?
            .amount;

            let reserve = Reserve::unpack(&reserve_info.try_borrow_data()?)?;
            let remaining_deposit_capacity = Decimal::from(reserve.config.deposit_limit)
                .saturating_sub(reserve.liquidity.total_supply()?);
            let source_liquidity_balance =
                std::cmp::min(source_liquidity_balance, remaining_deposit_capacity.try_floor_u64()?);

            msg!("source_liquidity_balance: {}", source_liquidity_balance);
            let instruction = deposit_reserve_liquidity_and_obligation_collateral(
                *solend_program_id.key,
                source_liquidity_balance,
                *source_liquidity_info.key,
                *user_collateral_info.key,
                *reserve_info.key,
                *reserve_liquidity_supply_info.key,
                *reserve_collateral_mint_info.key,
                *lending_market_info.key,
                *destination_collateral_info.key,
                *obligation_info.key,
                *obligation_owner_info.key,
                *pyth_price_info.key,
                *switchboard_feed_info.key,
                *user_transfer_authority_info.key,
            );
            invoke(
                &instruction,
                &[
                    solend_program_id.clone(),
                    source_liquidity_info.clone(),
                    user_collateral_info.clone(),
                    reserve_info.clone(),
                    reserve_liquidity_supply_info.clone(),
                    reserve_collateral_mint_info.clone(),
                    lending_market_info.clone(),
                    lending_market_authority_info.clone(),
                    destination_collateral_info.clone(),
                    obligation_info.clone(),
                    obligation_owner_info.clone(),
                    pyth_price_info.clone(),
                    switchboard_feed_info.clone(),
                    user_transfer_authority_info.clone(),
                    token_program_id.clone(),
                ],
            )?;

            Ok(())
        }
    }
}

/// Errors that may be returned by the TokenLending program.
#[derive(Clone, Debug, Eq, Error, FromPrimitive, PartialEq)]
pub enum WrapperError {
    /// Received ctokens
    #[error("Received ctokens")]
    ReceivedCTokens,
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

/// max repay instruction
#[allow(clippy::too_many_arguments)]
pub fn max_repay(
    program_id: Pubkey,
    solend_program_id: Pubkey,
    source_liquidity_pubkey: Pubkey,
    destination_liquidity_pubkey: Pubkey,
    repay_reserve_pubkey: Pubkey,
    obligation_pubkey: Pubkey,
    lending_market_pubkey: Pubkey,
    user_transfer_authority_pubkey: Pubkey,
) -> Instruction {
    Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new_readonly(solend_program_id, false),
            AccountMeta::new(source_liquidity_pubkey, false),
            AccountMeta::new(destination_liquidity_pubkey, false),
            AccountMeta::new(repay_reserve_pubkey, false),
            AccountMeta::new(obligation_pubkey, false),
            AccountMeta::new(lending_market_pubkey, false),
            AccountMeta::new_readonly(user_transfer_authority_pubkey, true),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: WrapperInstruction::RepayMax.try_to_vec().unwrap(),
    }
}

/// max deposit
#[allow(clippy::too_many_arguments)]
pub fn max_deposit(
    program_id: Pubkey,
    solend_program_id: Pubkey,
    source_liquidity_pubkey: Pubkey,
    user_collateral_pubkey: Pubkey,
    reserve_pubkey: Pubkey,
    reserve_liquidity_supply_pubkey: Pubkey,
    reserve_collateral_mint_pubkey: Pubkey,
    lending_market_pubkey: Pubkey,
    destination_deposit_collateral_pubkey: Pubkey,
    obligation_pubkey: Pubkey,
    obligation_owner_pubkey: Pubkey,
    reserve_liquidity_pyth_oracle_pubkey: Pubkey,
    reserve_liquidity_switchboard_oracle_pubkey: Pubkey,
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
            AccountMeta::new(user_collateral_pubkey, false),
            AccountMeta::new(reserve_pubkey, false),
            AccountMeta::new(reserve_liquidity_supply_pubkey, false),
            AccountMeta::new(reserve_collateral_mint_pubkey, false),
            AccountMeta::new_readonly(lending_market_pubkey, false),
            AccountMeta::new_readonly(lending_market_authority_pubkey, false),
            AccountMeta::new(destination_deposit_collateral_pubkey, false),
            AccountMeta::new(obligation_pubkey, false),
            AccountMeta::new(obligation_owner_pubkey, true),
            AccountMeta::new_readonly(reserve_liquidity_pyth_oracle_pubkey, false),
            AccountMeta::new_readonly(reserve_liquidity_switchboard_oracle_pubkey, false),
            AccountMeta::new_readonly(user_transfer_authority_pubkey, true),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: WrapperInstruction::DepositMax.try_to_vec().unwrap(),
    }
}

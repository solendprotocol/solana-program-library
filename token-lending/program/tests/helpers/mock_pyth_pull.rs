use anchor_lang::{AccountDeserialize, AccountSerialize};
use pyth_solana_receiver_sdk::price_update::{PriceFeedMessage, PriceUpdateV2, VerificationLevel};
/// mock oracle prices in tests with this program.
use solana_program::{
    account_info::AccountInfo,
    clock::Clock,
    entrypoint::ProgramResult,
    instruction::{AccountMeta, Instruction},
    msg,
    pubkey::Pubkey,
    sysvar::Sysvar,
};

use borsh::{BorshDeserialize, BorshSerialize};
use spl_token::solana_program::{account_info::next_account_info, program_error::ProgramError};
use thiserror::Error;

#[derive(BorshSerialize, BorshDeserialize)]
pub enum MockPythPullInstruction {
    /// Accounts:
    /// 0: PriceAccount (uninitialized)
    Init,

    /// Accounts:
    /// 0: PriceAccount
    SetPrice {
        price: i64,
        conf: u64,
        expo: i32,
        ema_price: i64,
        ema_conf: u64,
    },
}

pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    Processor::process(program_id, accounts, instruction_data)
}

fn account_deserialize<T: AccountDeserialize>(
    account: &AccountInfo<'_>,
) -> Result<T, Box<dyn std::error::Error>> {
    let data = account.clone().data.borrow().to_owned();
    let mut data: &[u8] = &data;

    let user: T = T::try_deserialize(&mut data)?;

    Ok(user)
}

pub struct Processor;
impl Processor {
    pub fn process(
        _program_id: &Pubkey,
        accounts: &[AccountInfo],
        instruction_data: &[u8],
    ) -> ProgramResult {
        let instruction = MockPythPullInstruction::try_from_slice(instruction_data)?;
        let account_info_iter = &mut accounts.iter().peekable();

        match instruction {
            MockPythPullInstruction::Init => {
                msg!("Mock Pyth Pull: Init");

                let price_account_info = next_account_info(account_info_iter)?;

                // write PriceAccount
                let price_update_v2 = PriceUpdateV2 {
                    write_authority: Pubkey::new_unique(),
                    verification_level: VerificationLevel::Full,
                    price_message: PriceFeedMessage {
                        feed_id: [1u8; 32],
                        price: 1,
                        conf: 1,
                        exponent: 1,
                        publish_time: 1,
                        prev_publish_time: 1,
                        ema_price: 1,
                        ema_conf: 1,
                    },
                    posted_slot: 0,
                };

                // let mut data = price_account_info.try_borrow_mut_data()?;
                let mut buf = Vec::new();
                price_update_v2.try_serialize(&mut buf)?;
                msg!("buf: {:?}", buf.len());

                let mut buf_sized = [0u8; PriceUpdateV2::LEN];
                buf_sized[0..buf.len()].copy_from_slice(&buf);

                price_account_info
                    .try_borrow_mut_data()?
                    .copy_from_slice(&buf_sized);

                Ok(())
            }
            MockPythPullInstruction::SetPrice {
                price,
                conf,
                expo,
                ema_price,
                ema_conf,
            } => {
                msg!("Mock Pyth Pull: Set price");
                let price_account_info = next_account_info(account_info_iter)?;

                let mut price_feed_account: PriceUpdateV2 = account_deserialize(price_account_info)
                    .map_err(|e| {
                        msg!("Failed to deserialize account: {:?}", e);
                        MockPythPullError::FailedToDeserialize
                    })?;

                price_feed_account.price_message.price = price;
                price_feed_account.price_message.conf = conf;
                price_feed_account.price_message.exponent = expo;
                price_feed_account.price_message.ema_price = ema_price;
                price_feed_account.price_message.ema_conf = ema_conf;
                price_feed_account.price_message.publish_time = Clock::get()?.unix_timestamp;

                price_feed_account.verification_level = VerificationLevel::Full;
                price_feed_account.posted_slot = Clock::get()?.slot;

                let mut buf = Vec::new();
                price_feed_account.try_serialize(&mut buf)?;

                let mut buf_sized = [0u8; PriceUpdateV2::LEN];
                buf_sized[0..buf.len()].copy_from_slice(&buf);

                price_account_info
                    .try_borrow_mut_data()?
                    .copy_from_slice(&buf_sized);

                Ok(())
            }
        }
    }
}

#[derive(Error, Debug, Copy, Clone)]
pub enum MockPythPullError {
    /// Invalid instruction
    #[error("Invalid Instruction")]
    InvalidInstruction,
    #[error("The account is not currently owned by the program")]
    IncorrectProgramId,
    #[error("Failed to deserialize")]
    FailedToDeserialize,
}

impl From<MockPythPullError> for ProgramError {
    fn from(e: MockPythPullError) -> Self {
        ProgramError::Custom(e as u32)
    }
}

pub fn init(program_id: Pubkey, price_account_pubkey: Pubkey) -> Instruction {
    let data = MockPythPullInstruction::Init.try_to_vec().unwrap();
    Instruction {
        program_id,
        accounts: vec![AccountMeta::new(price_account_pubkey, false)],
        data,
    }
}

pub fn set_price(
    program_id: Pubkey,
    price_account_pubkey: Pubkey,
    price: i64,
    conf: u64,
    expo: i32,
    ema_price: i64,
    ema_conf: u64,
) -> Instruction {
    let data = MockPythPullInstruction::SetPrice {
        price,
        conf,
        expo,
        ema_price,
        ema_conf,
    }
    .try_to_vec()
    .unwrap();
    Instruction {
        program_id,
        accounts: vec![AccountMeta::new(price_account_pubkey, false)],
        data,
    }
}

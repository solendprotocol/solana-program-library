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

use switchboard_on_demand::PullFeedAccountData;

use borsh::{BorshDeserialize, BorshSerialize};
use spl_token::solana_program::{account_info::next_account_info, program_error::ProgramError};
use thiserror::Error;

#[derive(BorshSerialize, BorshDeserialize)]
pub enum MockSwitchboardPullInstruction {
    /// Accounts:
    /// 0: AggregatorAccount
    InitSwitchboard,

    /// Accounts:
    /// 0: AggregatorAccount
    SetSwitchboardPrice { price: i64, expo: i32 },
}

pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    Processor::process(program_id, accounts, instruction_data)
}

pub struct Processor;
impl Processor {
    pub fn process(
        _program_id: &Pubkey,
        accounts: &[AccountInfo],
        instruction_data: &[u8],
    ) -> ProgramResult {
        let instruction = MockSwitchboardPullInstruction::try_from_slice(instruction_data)?;
        let account_info_iter = &mut accounts.iter().peekable();

        match instruction {
            MockSwitchboardPullInstruction::InitSwitchboard => {
                msg!("Mock Switchboard Pull: Init Switchboard");
                let switchboard_feed = next_account_info(account_info_iter)?;
                let mut data = switchboard_feed.try_borrow_mut_data()?;

                data[0..8].copy_from_slice(&PullFeedAccountData::discriminator());

                Ok(())
            }
            MockSwitchboardPullInstruction::SetSwitchboardPrice { price, expo } => {
                msg!("Mock Switchboard Pull: Set Switchboard price");
                let switchboard_feed = next_account_info(account_info_iter)?;

                let mut data = switchboard_feed.try_borrow_mut_data()?;

                let scaled = (price as i128) * 10i128.pow((18 + expo) as u32);

                let result_offset = 8 + 2256;
                data[result_offset..(result_offset + 16)].copy_from_slice(&scaled.to_le_bytes());
                data[(result_offset + 104)..(result_offset + 112)]
                    .copy_from_slice(&Clock::get()?.slot.to_le_bytes());

                Ok(())
            }
        }
    }
}

#[derive(Error, Debug, Copy, Clone)]
pub enum MockSwitchboardPullError {
    /// Invalid instruction
    #[error("Invalid Instruction")]
    InvalidInstruction,
    #[error("The account is not currently owned by the program")]
    IncorrectProgramId,
    #[error("Failed to deserialize")]
    FailedToDeserialize,
}

impl From<MockSwitchboardPullError> for ProgramError {
    fn from(e: MockSwitchboardPullError) -> Self {
        ProgramError::Custom(e as u32)
    }
}

pub fn set_switchboard_price(
    program_id: Pubkey,
    switchboard_feed: Pubkey,
    price: i64,
    expo: i32,
) -> Instruction {
    let data = MockSwitchboardPullInstruction::SetSwitchboardPrice { price, expo }
        .try_to_vec()
        .unwrap();
    Instruction {
        program_id,
        accounts: vec![AccountMeta::new(switchboard_feed, false)],
        data,
    }
}

pub fn init_switchboard(program_id: Pubkey, switchboard_feed: Pubkey) -> Instruction {
    let data = MockSwitchboardPullInstruction::InitSwitchboard
        .try_to_vec()
        .unwrap();
    Instruction {
        program_id,
        accounts: vec![AccountMeta::new(switchboard_feed, false)],
        data,
    }
}

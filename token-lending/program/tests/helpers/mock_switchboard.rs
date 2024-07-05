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
use std::cell::RefMut;
use switchboard_on_demand::OracleSubmission;
use switchboard_v2::{AggregatorAccountData, SwitchboardDecimal};

use borsh::{BorshDeserialize, BorshSerialize};
use spl_token::solana_program::{account_info::next_account_info, program_error::ProgramError};
use thiserror::Error;

#[derive(BorshSerialize, BorshDeserialize)]
pub enum MockSwitchboardInstruction {
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
        let instruction = MockSwitchboardInstruction::try_from_slice(instruction_data)?;
        let account_info_iter = &mut accounts.iter().peekable();

        match instruction {
            MockSwitchboardInstruction::InitSwitchboard => {
                msg!("Mock Switchboard: Init Switchboard");
                let switchboard_feed = next_account_info(account_info_iter)?;
                let mut data = switchboard_feed.try_borrow_mut_data()?;

                let discriminator = [217, 230, 65, 101, 201, 162, 27, 125];
                data[0..8].copy_from_slice(&discriminator);
                Ok(())
            }
            MockSwitchboardInstruction::SetSwitchboardPrice { price, expo } => {
                msg!("Mock Switchboard: Set Switchboard price");
                let switchboard_feed = next_account_info(account_info_iter)?;
                let data = switchboard_feed.try_borrow_mut_data()?;

                let mut aggregator_account: RefMut<AggregatorAccountData> =
                    RefMut::map(data, |data| {
                        bytemuck::from_bytes_mut(
                            &mut data[8..std::mem::size_of::<AggregatorAccountData>() + 8],
                        )
                    });

                aggregator_account.min_oracle_results = 1;
                aggregator_account.latest_confirmed_round.num_success = 1;
                aggregator_account.latest_confirmed_round.result = SwitchboardDecimal {
                    mantissa: price as i128,
                    scale: expo as u32,
                };
                aggregator_account.latest_confirmed_round.round_open_slot = Clock::get()?.slot;

                Ok(())
            }
        }
    }
}

#[derive(Error, Debug, Copy, Clone)]
pub enum MockSwitchboardError {
    /// Invalid instruction
    #[error("Invalid Instruction")]
    InvalidInstruction,
    #[error("The account is not currently owned by the program")]
    IncorrectProgramId,
    #[error("Failed to deserialize")]
    FailedToDeserialize,
}

impl From<MockSwitchboardError> for ProgramError {
    fn from(e: MockSwitchboardError) -> Self {
        ProgramError::Custom(e as u32)
    }
}

pub fn set_switchboard_price(
    program_id: Pubkey,
    switchboard_feed: Pubkey,
    price: i64,
    expo: i32,
) -> Instruction {
    let data = MockSwitchboardInstruction::SetSwitchboardPrice { price, expo }
        .try_to_vec()
        .unwrap();
    Instruction {
        program_id,
        accounts: vec![AccountMeta::new(switchboard_feed, false)],
        data,
    }
}

pub fn init_switchboard(program_id: Pubkey, switchboard_feed: Pubkey) -> Instruction {
    let data = MockSwitchboardInstruction::InitSwitchboard
        .try_to_vec()
        .unwrap();
    Instruction {
        program_id,
        accounts: vec![AccountMeta::new(switchboard_feed, false)],
        data,
    }
}

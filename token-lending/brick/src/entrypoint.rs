//! Program entrypoint definitions

#![cfg(all(target_arch = "bpf", not(feature = "no-entrypoint")))]

use solana_program::{
    account_info::AccountInfo, entrypoint, entrypoint::ProgramResult, msg,
    program_error::PrintProgramError, program_error::ProgramError, pubkey::Pubkey,
};

entrypoint!(process_instruction);
fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    msg!("got me feeling bricked up");

    Err(ProgramError::InvalidArgument)
}

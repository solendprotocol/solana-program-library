// #[cfg(feature = "test-bpf")]

mod helpers;

use helpers::*;
use solana_program_test::*;
use solana_sdk::{
    account::Account,
    instruction::InstructionError,
    pubkey::{Pubkey, PUBKEY_BYTES},
    signature::{Keypair, Signer},
    system_instruction::create_account,
    transaction::{Transaction, TransactionError},
};
use solend_program::{
    error::LendingError,
    instruction::flash_borrow_reserve_liquidity,
    instruction::{
        borrow_obligation_liquidity, deposit_obligation_collateral, flash_repay_reserve_liquidity,
        init_obligation, refresh_obligation, refresh_reserve, repay_obligation_liquidity,
        withdraw_obligation_collateral,
    },
    math::Decimal,
    processor::process_instruction,
    state::{Obligation, INITIAL_COLLATERAL_RATIO},
};
use spl_token::{instruction::approve, solana_program::program_pack::Pack};

#[tokio::test]
async fn test_success() {
    let mut test = ProgramTest::new(
        "solend_program",
        solend_program::id(),
        processor!(process_instruction),
    );

    // limit to track compute unit increase
    test.set_bpf_compute_max_units(45_000);

    const FLASH_LOAN_AMOUNT: u64 = 1_000 * FRACTIONAL_TO_USDC;
    const FEE_AMOUNT: u64 = 3_000_000;
    const HOST_FEE_AMOUNT: u64 = 600_000;

    let user_accounts_owner = Keypair::new();
    let lending_market = add_lending_market(&mut test);

    let mut reserve_config = test_reserve_config();
    reserve_config.fees.host_fee_percentage = 20;
    reserve_config.fees.flash_loan_fee_wad = 3_000_000_000_000_000;

    let usdc_mint = add_usdc_mint(&mut test);
    let usdc_oracle = add_usdc_oracle(&mut test);
    let usdc_test_reserve = add_reserve(
        &mut test,
        &lending_market,
        &usdc_oracle,
        &user_accounts_owner,
        AddReserveArgs {
            user_liquidity_amount: FEE_AMOUNT,
            liquidity_amount: FLASH_LOAN_AMOUNT,
            liquidity_mint_pubkey: usdc_mint.pubkey,
            liquidity_mint_decimals: usdc_mint.decimals,
            config: reserve_config,
            ..AddReserveArgs::default()
        },
    );

    let (mut banks_client, payer, recent_blockhash) = test.start().await;
    let mut transaction = Transaction::new_with_payer(
        &[
            flash_borrow_reserve_liquidity(
                solend_program::id(),
                FLASH_LOAN_AMOUNT,
                usdc_test_reserve.liquidity_supply_pubkey,
                usdc_test_reserve.user_liquidity_pubkey,
                usdc_test_reserve.pubkey,
                lending_market.pubkey,
            ),
            flash_repay_reserve_liquidity(
                solend_program::id(),
                FLASH_LOAN_AMOUNT,
                usdc_test_reserve.user_liquidity_pubkey,
                usdc_test_reserve.liquidity_supply_pubkey,
                usdc_test_reserve.config.fee_receiver,
                usdc_test_reserve.liquidity_host_pubkey,
                usdc_test_reserve.pubkey,
                lending_market.pubkey,
                user_accounts_owner.pubkey(),
            ),
        ],
        Some(&payer.pubkey()),
    );

    transaction.sign(&[&payer, &user_accounts_owner], recent_blockhash);
    assert!(banks_client.process_transaction(transaction).await.is_ok());

    let usdc_reserve = usdc_test_reserve.get_state(&mut banks_client).await;
    assert_eq!(usdc_reserve.liquidity.available_amount, FLASH_LOAN_AMOUNT);

    let liquidity_supply =
        get_token_balance(&mut banks_client, usdc_test_reserve.liquidity_supply_pubkey).await;
    assert_eq!(liquidity_supply, FLASH_LOAN_AMOUNT);

    let token_balance =
        get_token_balance(&mut banks_client, usdc_test_reserve.user_liquidity_pubkey).await;
    assert_eq!(token_balance, 0);

    let fee_balance =
        get_token_balance(&mut banks_client, usdc_test_reserve.config.fee_receiver).await;
    assert_eq!(fee_balance, FEE_AMOUNT - HOST_FEE_AMOUNT);

    let host_fee_balance =
        get_token_balance(&mut banks_client, usdc_test_reserve.liquidity_host_pubkey).await;
    assert_eq!(host_fee_balance, HOST_FEE_AMOUNT);
}

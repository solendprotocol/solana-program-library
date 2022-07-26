// #[cfg(feature = "test-bpf")]

mod helpers;

use helpers::*;
use solana_program_test::*;
use solana_sdk::{
    instruction::InstructionError,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::{Transaction, TransactionError},
};
use solend_program::{
    error::LendingError,
    instruction::{flash_borrow_reserve_liquidity, flash_repay_reserve_liquidity},
    processor::process_instruction,
};

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

#[tokio::test]
async fn test_fail_disable_flash_loans() {
    let mut test = ProgramTest::new(
        "solend_program",
        solend_program::id(),
        processor!(process_instruction),
    );

    // limit to track compute unit increase
    test.set_bpf_compute_max_units(60_000);

    const FLASH_LOAN_AMOUNT: u64 = 3_000_000;
    const LIQUIDITY_AMOUNT: u64 = 1_000 * FRACTIONAL_TO_USDC;
    const FEE_AMOUNT: u64 = 3_000_000;

    let user_accounts_owner = Keypair::new();
    let lending_market = add_lending_market(&mut test);

    let mut reserve_config = test_reserve_config();
    reserve_config.fees.flash_loan_fee_wad = u64::MAX;

    let usdc_mint = add_usdc_mint(&mut test);
    let usdc_oracle = add_usdc_oracle(&mut test);
    let usdc_test_reserve = add_reserve(
        &mut test,
        &lending_market,
        &usdc_oracle,
        &user_accounts_owner,
        AddReserveArgs {
            user_liquidity_amount: FEE_AMOUNT,
            liquidity_amount: LIQUIDITY_AMOUNT,
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

    assert_eq!(
        banks_client
            .process_transaction(transaction)
            .await
            .unwrap_err()
            .unwrap(),
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(LendingError::FlashLoansDisabled as u32)
        )
    );
}

#[tokio::test]
async fn test_fail_double_borrow() {
    let mut test = ProgramTest::new(
        "solend_program",
        solend_program::id(),
        processor!(process_instruction),
    );

    // limit to track compute unit increase
    test.set_bpf_compute_max_units(60_000);

    const FLASH_LOAN_AMOUNT: u64 = 3_000_000;
    const LIQUIDITY_AMOUNT: u64 = 1_000 * FRACTIONAL_TO_USDC;
    const FEE_AMOUNT: u64 = 3_000_000;

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
            liquidity_amount: LIQUIDITY_AMOUNT,
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

    assert_eq!(
        banks_client
            .process_transaction(transaction)
            .await
            .unwrap_err()
            .unwrap(),
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(LendingError::MultipleFlashBorrows as u32)
        )
    );
}

/// idk why anyone would do this but w/e
#[tokio::test]
async fn test_fail_double_repay() {
    let mut test = ProgramTest::new(
        "solend_program",
        solend_program::id(),
        processor!(process_instruction),
    );

    // limit to track compute unit increase
    test.set_bpf_compute_max_units(60_000);

    const FLASH_LOAN_AMOUNT: u64 = 3_000_000;
    const LIQUIDITY_AMOUNT: u64 = 1_000 * FRACTIONAL_TO_USDC;
    const FEE_AMOUNT: u64 = 3_000_000;

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
            liquidity_amount: LIQUIDITY_AMOUNT,
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
            flash_repay_reserve_liquidity(
                solend_program::id(),
                0,
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

    assert_eq!(
        banks_client
            .process_transaction(transaction)
            .await
            .unwrap_err()
            .unwrap(),
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(LendingError::MultipleFlashBorrows as u32)
        )
    );
}

#[tokio::test]
async fn test_fail_only_one_flash_ix_pair_per_tx() {
    let mut test = ProgramTest::new(
        "solend_program",
        solend_program::id(),
        processor!(process_instruction),
    );

    // limit to track compute unit increase
    test.set_bpf_compute_max_units(60_000);

    const FLASH_LOAN_AMOUNT: u64 = 3_000_000;
    const LIQUIDITY_AMOUNT: u64 = 1_000 * FRACTIONAL_TO_USDC;
    const FEE_AMOUNT: u64 = 3_000_000;

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
            liquidity_amount: LIQUIDITY_AMOUNT,
            liquidity_mint_pubkey: usdc_mint.pubkey,
            liquidity_mint_decimals: usdc_mint.decimals,
            config: reserve_config,
            ..AddReserveArgs::default()
        },
    );

    // eventually this will be valid. but for v1 implementation, we only let 1 flash ix pair per tx
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

    assert_eq!(
        banks_client
            .process_transaction(transaction)
            .await
            .unwrap_err()
            .unwrap(),
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(LendingError::MultipleFlashBorrows as u32)
        )
    );
}

#[tokio::test]
async fn test_fail_invalid_repay_ix() {
    let mut test = ProgramTest::new(
        "solend_program",
        solend_program::id(),
        processor!(process_instruction),
    );

    let proxy_program_id = Pubkey::new_unique();
    test.prefer_bpf(false);
    test.add_program(
        "flash_loan_proxy",
        proxy_program_id,
        processor!(helpers::flash_loan_proxy::process_instruction),
    );

    const FLASH_LOAN_AMOUNT: u64 = 3_000_000;
    const LIQUIDITY_AMOUNT: u64 = 1_000_000 * FRACTIONAL_TO_USDC;
    const FEE_AMOUNT: u64 = 3_000_000;

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
            liquidity_amount: LIQUIDITY_AMOUNT,
            liquidity_mint_pubkey: usdc_mint.pubkey,
            liquidity_mint_decimals: usdc_mint.decimals,
            config: reserve_config,
            ..AddReserveArgs::default()
        },
    );

    let (mut banks_client, payer, recent_blockhash) = test.start().await;

    // case 1: invalid reserve in repay
    {
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
                    Pubkey::new_unique(),
                    lending_market.pubkey,
                    user_accounts_owner.pubkey(),
                ),
            ],
            Some(&payer.pubkey()),
        );
        transaction.sign(&[&payer, &user_accounts_owner], recent_blockhash);

        assert_eq!(
            banks_client
                .process_transaction(transaction)
                .await
                .unwrap_err()
                .unwrap(),
            TransactionError::InstructionError(
                0,
                InstructionError::Custom(LendingError::InvalidFlashRepay as u32)
            )
        );
    }

    // case 2: invalid liquidity amount
    {
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
                    FLASH_LOAN_AMOUNT - 1,
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

        assert_eq!(
            banks_client
                .process_transaction(transaction)
                .await
                .unwrap_err()
                .unwrap(),
            TransactionError::InstructionError(
                0,
                InstructionError::Custom(LendingError::InvalidFlashRepay as u32)
            )
        );
    }

    // case 3: no repay
    {
        let mut transaction = Transaction::new_with_payer(
            &[flash_borrow_reserve_liquidity(
                solend_program::id(),
                FLASH_LOAN_AMOUNT,
                usdc_test_reserve.liquidity_supply_pubkey,
                usdc_test_reserve.user_liquidity_pubkey,
                usdc_test_reserve.pubkey,
                lending_market.pubkey,
            )],
            Some(&payer.pubkey()),
        );

        transaction.sign(&[&payer], recent_blockhash);

        assert_eq!(
            banks_client
                .process_transaction(transaction)
                .await
                .unwrap_err()
                .unwrap(),
            TransactionError::InstructionError(
                0,
                InstructionError::Custom(LendingError::NoFlashRepayFound as u32)
            )
        );
    }

    // case 4: cpi repay
    {
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
                helpers::flash_loan_proxy::repay_proxy(
                    proxy_program_id,
                    FLASH_LOAN_AMOUNT,
                    usdc_test_reserve.user_liquidity_pubkey,
                    usdc_test_reserve.liquidity_supply_pubkey,
                    usdc_test_reserve.config.fee_receiver,
                    usdc_test_reserve.liquidity_host_pubkey,
                    usdc_test_reserve.pubkey,
                    solend_program::id(),
                    lending_market.pubkey,
                    user_accounts_owner.pubkey(),
                ),
            ],
            Some(&payer.pubkey()),
        );
        transaction.sign(&[&payer, &user_accounts_owner], recent_blockhash);

        assert_eq!(
            banks_client
                .process_transaction(transaction)
                .await
                .unwrap_err()
                .unwrap(),
            TransactionError::InstructionError(
                0,
                InstructionError::Custom(LendingError::NoFlashRepayFound as u32)
            )
        );
    }

    // case 5: insufficient funds to pay fees on repay. FEE_AMOUNT was calculated using
    // FLASH_LOAN_AMOUNT, not LIQUIDITY_AMOUNT.
    {
        let mut transaction = Transaction::new_with_payer(
            &[
                flash_borrow_reserve_liquidity(
                    solend_program::id(),
                    LIQUIDITY_AMOUNT,
                    usdc_test_reserve.liquidity_supply_pubkey,
                    usdc_test_reserve.user_liquidity_pubkey,
                    usdc_test_reserve.pubkey,
                    lending_market.pubkey,
                ),
                flash_repay_reserve_liquidity(
                    solend_program::id(),
                    LIQUIDITY_AMOUNT,
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

        assert_eq!(
            banks_client
                .process_transaction(transaction)
                .await
                .unwrap_err()
                .unwrap(),
            TransactionError::InstructionError(
                1,
                InstructionError::Custom(spl_token::error::TokenError::InsufficientFunds as u32),
            )
        );
    }
}

#[tokio::test]
async fn test_fail_insufficient_liquidity_for_borrow() {
    let mut test = ProgramTest::new(
        "solend_program",
        solend_program::id(),
        processor!(process_instruction),
    );

    // limit to track compute unit increase
    test.set_bpf_compute_max_units(60_000);

    const LIQUIDITY_AMOUNT: u64 = 1_000 * FRACTIONAL_TO_USDC;
    const FEE_AMOUNT: u64 = 3_000_000;

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
            liquidity_amount: LIQUIDITY_AMOUNT,
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
                LIQUIDITY_AMOUNT + 1,
                usdc_test_reserve.liquidity_supply_pubkey,
                usdc_test_reserve.user_liquidity_pubkey,
                usdc_test_reserve.pubkey,
                lending_market.pubkey,
            ),
            flash_repay_reserve_liquidity(
                solend_program::id(),
                LIQUIDITY_AMOUNT + 1,
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

    assert_eq!(
        banks_client
            .process_transaction(transaction)
            .await
            .unwrap_err()
            .unwrap(),
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(LendingError::InsufficientLiquidity as u32)
        )
    );
}

#[tokio::test]
async fn test_fail_cpi_borrow() {
    let mut test = ProgramTest::new(
        "solend_program",
        solend_program::id(),
        processor!(process_instruction),
    );

    let proxy_program_id = Pubkey::new_unique();
    test.prefer_bpf(false);
    test.add_program(
        "flash_loan_proxy",
        proxy_program_id,
        processor!(helpers::flash_loan_proxy::process_instruction),
    );

    // limit to track compute unit increase
    test.set_bpf_compute_max_units(60_000);

    const FLASH_LOAN_AMOUNT: u64 = 3_000_000;
    const LIQUIDITY_AMOUNT: u64 = 1_000 * FRACTIONAL_TO_USDC;
    const FEE_AMOUNT: u64 = 3_000_000;

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
            liquidity_amount: LIQUIDITY_AMOUNT,
            liquidity_mint_pubkey: usdc_mint.pubkey,
            liquidity_mint_decimals: usdc_mint.decimals,
            config: reserve_config,
            ..AddReserveArgs::default()
        },
    );

    let (mut banks_client, payer, recent_blockhash) = test.start().await;
    let mut transaction = Transaction::new_with_payer(
        &[helpers::flash_loan_proxy::borrow_proxy(
            proxy_program_id,
            FLASH_LOAN_AMOUNT,
            usdc_test_reserve.liquidity_supply_pubkey,
            usdc_test_reserve.user_liquidity_pubkey,
            usdc_test_reserve.pubkey,
            solend_program::id(),
            lending_market.pubkey,
            lending_market.authority,
        )],
        Some(&payer.pubkey()),
    );
    transaction.sign(&[&payer], recent_blockhash);

    assert_eq!(
        banks_client
            .process_transaction(transaction)
            .await
            .unwrap_err()
            .unwrap(),
        TransactionError::InstructionError(
            0,
            InstructionError::Custom(LendingError::FlashBorrowCpi as u32)
        )
    );
}

#![cfg(feature = "test-bpf")]

mod helpers;

use std::collections::HashSet;

use helpers::solend_program_test::{
    setup_world, BalanceChecker, Info, SolendProgramTest, TokenBalanceChange, User,
};
use helpers::test_reserve_config;

use pretty_assertions::assert_eq;
use solana_program::instruction::InstructionError;
use solana_program_test::*;
use solana_sdk::signature::Keypair;
use solana_sdk::transaction::TransactionError;
use solend_program::math::Decimal;
use solend_program::state::{
    LastUpdate, LendingMarket, Obligation, ObligationCollateral, PoolRewardManager, PositionKind,
    Reserve, UserRewardManager, UserRewardManagers,
};

async fn setup() -> (
    SolendProgramTest,
    Info<LendingMarket>,
    Info<Reserve>,
    User,
    Info<Obligation>,
) {
    let (mut test, lending_market, usdc_reserve, _, _, user) =
        setup_world(&test_reserve_config(), &test_reserve_config()).await;

    let obligation = lending_market
        .init_obligation(&mut test, Keypair::new(), &user)
        .await
        .expect("This should succeed");

    lending_market
        .deposit(&mut test, &usdc_reserve, &user, 1_000_000)
        .await
        .expect("This should succeed");

    let usdc_reserve = test.load_account(usdc_reserve.pubkey).await;

    (test, lending_market, usdc_reserve, user, obligation)
}

#[tokio::test]
async fn test_success() {
    let (mut test, lending_market, usdc_reserve, user, obligation) = setup().await;

    let balance_checker = BalanceChecker::start(&mut test, &[&usdc_reserve, &user]).await;

    let deposit_amount = 1_000_000u64;

    lending_market
        .deposit_obligation_collateral(&mut test, &usdc_reserve, &obligation, &user, deposit_amount)
        .await
        .expect("This should succeed");

    // check balance changes
    let (balance_changes, mint_supply_changes) =
        balance_checker.find_balance_changes(&mut test).await;
    let expected_balance_changes = HashSet::from([
        TokenBalanceChange {
            token_account: user
                .get_account(&usdc_reserve.account.collateral.mint_pubkey)
                .unwrap(),
            mint: usdc_reserve.account.collateral.mint_pubkey,
            diff: -(deposit_amount as i128),
        },
        TokenBalanceChange {
            token_account: usdc_reserve.account.collateral.supply_pubkey,
            mint: usdc_reserve.account.collateral.mint_pubkey,
            diff: deposit_amount as _,
        },
    ]);

    assert_eq!(balance_changes, expected_balance_changes);
    assert_eq!(mint_supply_changes, HashSet::new());

    // check program state changes
    let lending_market_post = test.load_account(lending_market.pubkey).await;
    assert_eq!(lending_market, lending_market_post);

    let usdc_reserve_post = test.load_account::<Reserve>(usdc_reserve.pubkey).await;
    let last_update_time_secs = usdc_reserve_post
        .account
        .deposits_pool_reward_manager
        .last_update_time_secs;
    assert_ne!(last_update_time_secs, 0);

    assert_eq!(
        usdc_reserve_post.account,
        Reserve {
            deposits_pool_reward_manager: Box::new(PoolRewardManager {
                total_shares: deposit_amount,
                last_update_time_secs,
                ..*usdc_reserve.account.deposits_pool_reward_manager
            }),
            ..usdc_reserve.account
        }
    );

    let obligation_post = test.load_obligation(obligation.pubkey).await;
    assert_eq!(
        obligation_post.account,
        Obligation {
            last_update: LastUpdate {
                slot: 1000,
                stale: true,
            },
            deposits: vec![ObligationCollateral {
                deposit_reserve: usdc_reserve.pubkey,
                deposited_amount: deposit_amount,
                market_value: Decimal::zero(), // this field only gets updated on a refresh
                attributed_borrow_value: Decimal::zero()
            }],
            user_reward_managers: UserRewardManagers(vec![UserRewardManager {
                reserve: usdc_reserve.pubkey,
                position_kind: PositionKind::Deposit,
                share: deposit_amount,
                last_update_time_secs,
                rewards: Vec::new(),
            }]),
            ..obligation.account
        }
    );
}

#[tokio::test]
async fn test_fail_deposit_too_much() {
    let (mut test, lending_market, usdc_reserve, user, obligation) = setup().await;

    let res = lending_market
        .deposit_obligation_collateral(&mut test, &usdc_reserve, &obligation, &user, 1_000_001)
        .await
        .err()
        .unwrap()
        .unwrap();

    // ix 0 is CU budget, ix 1 is transfer to obligation for realloc, ix 2 is deposit
    const EXPECTED_IX: u8 = 2;
    match res {
        // InsufficientFunds
        TransactionError::InstructionError(EXPECTED_IX, InstructionError::Custom(1)) => (),
        // LendingError::TokenTransferFailed
        TransactionError::InstructionError(EXPECTED_IX, InstructionError::Custom(17)) => (),
        e => panic!("unexpected error: {:#?}", e),
    };
}

#![cfg(feature = "test-bpf")]

mod helpers;

use std::collections::HashSet;

use helpers::solend_program_test::{
    setup_world, BalanceChecker, LiqMiningReward, TokenAccount, TokenBalanceChange,
};
use helpers::test_reserve_config;

use pretty_assertions::assert_eq;
use solana_program_test::*;
use solana_sdk::account::Account;
use solana_sdk::instruction::InstructionError;
use solana_sdk::signature::{Keypair, Signer};
use solana_sdk::transaction::TransactionError;
use solend_program::{
    math::Decimal,
    state::{PoolRewardId, PoolRewardManager, PositionKind, Reserve, UserRewardManager},
};
use solend_sdk::error::LendingError;
use solend_sdk::math::TryMul;
use solend_sdk::state::{Obligation, PoolReward, PoolRewardEntry, UserReward};

#[tokio::test]
async fn test_claim_pool_reward_for_deposit() {
    test_(PositionKind::Deposit).await;
}

#[tokio::test]
async fn test_claim_pool_reward_for_borrow() {
    test_(PositionKind::Borrow).await;
}

async fn test_(position_kind: PositionKind) {
    let (mut test, lending_market, usdc_reserve, wsol_reserve, mut lending_market_owner, mut user) =
        setup_world(&test_reserve_config(), &test_reserve_config()).await;

    let reward_mint = test.create_mint_as_test_authority().await;
    let reward_vault = Keypair::new();
    let duration_secs = 3_600;
    let total_rewards = 1_000_000;
    let current_time = test.get_clock().await.unix_timestamp as u64;
    let initial_time = current_time;
    let reward = LiqMiningReward {
        mint: reward_mint,
        vault: reward_vault.insecure_clone(),
    };

    lending_market
        .add_pool_reward(
            &mut test,
            &usdc_reserve,
            &mut lending_market_owner,
            &reward,
            position_kind,
            current_time,
            current_time + duration_secs as u64,
            total_rewards,
        )
        .await
        .expect("Should add pool reward");

    let obligation = lending_market
        .init_obligation(&mut test, Keypair::new(), &user)
        .await
        .expect("Should init obligation");

    let expected_share = match position_kind {
        PositionKind::Deposit => {
            let deposit_amount = 1_000_000;
            lending_market
                .deposit_reserve_liquidity_and_obligation_collateral(
                    &mut test,
                    &usdc_reserve,
                    &obligation,
                    &user,
                    deposit_amount,
                )
                .await
                .expect("Should deposit $USDC");

            deposit_amount
        }
        PositionKind::Borrow => {
            lending_market
                .deposit_reserve_liquidity_and_obligation_collateral(
                    &mut test,
                    &wsol_reserve,
                    &obligation,
                    &user,
                    420_000_000,
                )
                .await
                .expect("Should deposit $wSOL");

            lending_market
                .borrow_obligation_liquidity(
                    &mut test,
                    &usdc_reserve,
                    &obligation,
                    &user,
                    None,
                    690,
                )
                .await
                .expect("Should borrow $USDC");

            690
        }
    };

    let current_time = test
        .advance_clock_by_slots_and_secs(1, duration_secs as u64 / 2)
        .await;

    // user must have a token account to deposit rewards into ahead of time
    user.create_associated_token_account(&reward.mint, &mut test)
        .await;

    let balance_checker =
        BalanceChecker::start(&mut test, &[&TokenAccount(reward.vault.pubkey()), &user]).await;

    let err = lending_market
        .claim_pool_reward(
            &mut test,
            &obligation,
            &usdc_reserve,
            &user,
            &reward,
            position_kind,
            None,
        )
        .await
        .expect_err("Cannot claim reward before it ends unless owner");

    match err.unwrap() {
        TransactionError::InstructionError(_, InstructionError::Custom(err_code)) => {
            assert_eq!(err_code, LendingError::InvalidSigner as u32);
        }
        _ => panic!("Expected LendingError::InvalidSigner, got: {:?}", err),
    };

    lending_market
        .claim_pool_reward(
            &mut test,
            &obligation,
            &usdc_reserve,
            &user,
            &reward,
            position_kind,
            Some(&user),
        )
        .await
        .expect("Should claim reward");

    let (balance_changes, _) = balance_checker.find_balance_changes(&mut test).await;

    let diff = (total_rewards as i128) / 2
        - match position_kind {
            PositionKind::Deposit => 0,
            PositionKind::Borrow => 1, // integer division rounds down
        };
    let expected_balance_changes = HashSet::from([
        TokenBalanceChange {
            token_account: user.get_account(&reward.mint).unwrap(),
            mint: reward.mint,
            diff,
        },
        TokenBalanceChange {
            token_account: reward.vault.pubkey(),
            mint: reward.mint,
            diff: -diff,
        },
    ]);
    assert_eq!(balance_changes, expected_balance_changes);

    let usdc_reserve_post = test.load_account::<Reserve>(usdc_reserve.pubkey).await;

    let cumulative_rewards_per_share = match position_kind {
        PositionKind::Deposit => Decimal::from_scaled_val(500000000000000000),
        PositionKind::Borrow => Decimal::from_scaled_val(724637681159420289855),
    };

    let expected_reward_manager = PoolRewardManager {
        total_shares: expected_share,
        last_update_time_secs: current_time,
        pool_rewards: {
            let mut og = PoolRewardManager::default().pool_rewards;

            og[0] = PoolRewardEntry::Occupied(Box::new(PoolReward {
                id: PoolRewardId(1),
                vault: reward_vault.pubkey(),
                start_time_secs: initial_time,
                duration_secs,
                total_rewards,
                num_user_reward_managers: 1,
                cumulative_rewards_per_share,
            }));

            og
        },
    };

    assert_eq!(
        usdc_reserve_post.account.pool_reward_manager(position_kind),
        &expected_reward_manager
    );

    let obligation_post = test.load_obligation(obligation.pubkey).await;

    let earned_rewards = match position_kind {
        PositionKind::Deposit => {
            // on deposit there's no division involved and so it ends up being
            // nice whole number
            Decimal::zero()
        }
        PositionKind::Borrow => {
            // on borrow we have some precision loss and so the one extra
            // _almost_ token stays in the user's account
            Decimal::from_scaled_val(999999999999999950)
        }
    };
    // we don't withdraw fractions of a token but keep them around for future claims
    assert_eq!(earned_rewards.try_floor_u64().unwrap(), 0);

    assert_eq!(
        obligation_post.account.user_reward_managers.last().unwrap(),
        &UserRewardManager {
            reserve: usdc_reserve.pubkey,
            position_kind,
            share: expected_share,
            last_update_time_secs: current_time,
            rewards: vec![UserReward {
                pool_reward_index: 0,
                pool_reward_id: PoolRewardId(1),
                earned_rewards,
                cumulative_rewards_per_share
            }],
        }
    );

    // move time forward so that all rewards can be claimed

    let current_time = test
        .advance_clock_by_slots_and_secs(1, duration_secs as _)
        .await;

    lending_market
        .claim_pool_reward(
            &mut test,
            &obligation,
            &usdc_reserve,
            &user,
            &reward,
            position_kind,
            None,
        )
        .await
        .expect("Should claim reward");

    // reserve should have no user reward managers

    let usdc_reserve_final = test.load_account::<Reserve>(usdc_reserve.pubkey).await;
    let pool_reward_manager = usdc_reserve_final
        .account
        .pool_reward_manager(position_kind);

    assert_eq!(pool_reward_manager.last_update_time_secs, current_time);

    assert_eq!(
        pool_reward_manager.pool_rewards[0],
        PoolRewardEntry::Occupied(Box::new(PoolReward {
            id: PoolRewardId(1),
            vault: reward_vault.pubkey(),
            start_time_secs: initial_time,
            duration_secs,
            total_rewards,
            num_user_reward_managers: 0,
            cumulative_rewards_per_share: cumulative_rewards_per_share
                .try_mul(Decimal::from(2u64))
                .unwrap()
        }))
    );

    // obligation should no longer track this reward

    let obligation_final = test.load_obligation(obligation.pubkey).await;

    assert_eq!(
        obligation_final
            .account
            .user_reward_managers
            .last()
            .unwrap()
            .rewards,
        vec![],
    );
}

#[tokio::test]
async fn test_cannot_claim_into_wrong_destination() {
    let (mut test, lending_market, usdc_reserve, _, mut lending_market_owner, user) =
        setup_world(&test_reserve_config(), &test_reserve_config()).await;

    let reward_mint = test.create_mint_as_test_authority().await;
    let reward_vault = Keypair::new();
    let duration_secs = 3_600;
    let total_rewards = 1_000_000;
    let initial_time = test.get_clock().await.unix_timestamp as u64;
    let reward = LiqMiningReward {
        mint: reward_mint,
        vault: reward_vault.insecure_clone(),
    };

    lending_market
        .add_pool_reward(
            &mut test,
            &usdc_reserve,
            &mut lending_market_owner,
            &reward,
            PositionKind::Deposit,
            initial_time,
            initial_time + duration_secs as u64,
            total_rewards,
        )
        .await
        .expect("Should add pool reward");

    let obligation = lending_market
        .init_obligation(&mut test, Keypair::new(), &user)
        .await
        .expect("Should init obligation");

    lending_market
        .deposit_reserve_liquidity_and_obligation_collateral(
            &mut test,
            &usdc_reserve,
            &obligation,
            &user,
            1,
        )
        .await
        .expect("Should deposit $USDC");

    // let's use a token account of a wrong user
    lending_market_owner
        .create_token_account(&reward.mint, &mut test)
        .await;

    let err = lending_market
        .claim_pool_reward(
            &mut test,
            &obligation,
            &usdc_reserve,
            &lending_market_owner, // ! wrong
            &reward,
            PositionKind::Deposit,
            None,
        )
        .await
        .expect_err("Cannot steal user reward");

    assert_eq!(
        err.unwrap(),
        TransactionError::InstructionError(
            1,
            InstructionError::Custom(LendingError::InvalidAccountInput as _)
        )
    );
}

#[tokio::test]
async fn test_migrate_obligation() {
    let (mut test, lending_market, usdc_reserve, _, mut lending_market_owner, mut user) =
        setup_world(&test_reserve_config(), &test_reserve_config()).await;

    let obligation = lending_market
        .init_obligation(&mut test, Keypair::new(), &user)
        .await
        .expect("Should init obligation");

    lending_market
        .deposit_reserve_liquidity_and_obligation_collateral(
            &mut test,
            &usdc_reserve,
            &obligation,
            &user,
            1,
        )
        .await
        .expect("Should deposit $USDC");

    {
        // The call above set up the obligation with a user reward manager.
        // We'll now truncate the liq. mining data to simulate an obligation in
        // the old format.
        // However, that will leave the reserve in an invalid state as it will
        // have already the user shares set up.
        // That's ok, let's just ignore that in this test.

        let mut new_raw_obligation = Account {
            data: vec![0; Obligation::MIN_LEN],
            ..test
                .context
                .banks_client
                .get_account(obligation.pubkey)
                .await
                .expect("Should access obligation account")
                .expect("Obligation account should exist")
        };

        Obligation::pack(
            {
                let mut obligation = test.load_obligation(obligation.pubkey).await;
                obligation.account.user_reward_managers.clear();
                obligation.account
            },
            &mut new_raw_obligation.data,
        )
        .expect("Should pack obligation");

        test.context
            .set_account(&obligation.pubkey, &new_raw_obligation.into());
    }

    let reward_mint = test.create_mint_as_test_authority().await;
    let reward_vault = Keypair::new();
    let duration_secs = 3_600;
    let total_rewards = 1_000_000;
    let initial_time = test.get_clock().await.unix_timestamp as u64;
    let reward = LiqMiningReward {
        mint: reward_mint,
        vault: reward_vault.insecure_clone(),
    };

    lending_market
        .add_pool_reward(
            &mut test,
            &usdc_reserve,
            &mut lending_market_owner,
            &reward,
            PositionKind::Deposit,
            initial_time,
            initial_time + duration_secs as u64,
            total_rewards,
        )
        .await
        .expect("Should add pool reward");

    let current_time = test
        .advance_clock_by_slots_and_secs(1, duration_secs as u64 / 2)
        .await;

    // user must have a token account to deposit rewards into ahead of time
    user.create_associated_token_account(&reward.mint, &mut test)
        .await;

    let balance_checker = BalanceChecker::start(&mut test, &[&user]).await;

    // At this point the user did not have any shares in the obligation and so
    // they cannot claim anything.
    // However, we migrate the obligation so that next time they claim they do
    // get something.

    lending_market
        .claim_pool_reward(
            &mut test,
            &obligation,
            &usdc_reserve,
            &user,
            &reward,
            PositionKind::Deposit,
            None,
        )
        .await
        .expect("Should claim reward");

    let usdc_reserve_post = test.load_account::<Reserve>(usdc_reserve.pubkey).await;

    assert_eq!(
        usdc_reserve_post
            .account
            .deposits_pool_reward_manager
            .total_shares,
        2 // 1 from the old obligation and 1 from the new one
    );

    let obligation_post = test.load_obligation(obligation.pubkey).await;

    assert_eq!(
        obligation_post.account.user_reward_managers[0],
        UserRewardManager {
            reserve: usdc_reserve.pubkey,
            position_kind: PositionKind::Deposit,
            share: 1,
            last_update_time_secs: current_time,
            rewards: vec![UserReward {
                pool_reward_index: 0,
                pool_reward_id: PoolRewardId(1),
                earned_rewards: Decimal::zero(),
                cumulative_rewards_per_share: Decimal::from_scaled_val(500000_000000000000000000),
            }],
        }
    );

    let (balance_changes, _) = balance_checker.find_balance_changes(&mut test).await;
    assert_eq!(balance_changes, HashSet::new());

    let current_time = test.advance_clock_by_slots_and_secs(1, duration_secs).await;

    // now they should be able to claim rewards

    lending_market
        .claim_pool_reward(
            &mut test,
            &obligation,
            &usdc_reserve,
            &user,
            &reward,
            PositionKind::Deposit,
            None,
        )
        .await
        .expect("Should claim reward");

    let obligation_final = test.load_obligation(obligation.pubkey).await;

    assert_eq!(
        obligation_final.account.user_reward_managers[0],
        UserRewardManager {
            reserve: usdc_reserve.pubkey,
            position_kind: PositionKind::Deposit,
            share: 1,
            last_update_time_secs: current_time,
            rewards: vec![],
        }
    );

    let (balance_changes, _) = balance_checker.find_balance_changes(&mut test).await;
    assert_eq!(
        balance_changes,
        HashSet::from([TokenBalanceChange {
            token_account: user.get_account(&reward.mint).unwrap(),
            mint: reward.mint,
            // There are 2 shares and we're accruing rewards for half the time.
            // There are 2 shares bcs we reset the obligation and "register" it
            // a second time in this test.
            diff: (total_rewards as i128) / 4,
        }])
    );
}

# Liquidity Mining

## Overview

The liquidity mining feature models the same feature implemented in [Suilend][suilend-lm].
In a gist we track deposits and borrows for each reserve in two structures: pool reward manager that exist on each reserve and user reward manager that exist on linked obligations.

Deposits increase total pool shares and user shares by the exact amount of collateral token deposited into an obligation.
Collateral token that is _not_ deposited into any obligation does not count toward the total pool shares.

Conversely, withdraws decrease total pool shares and user shares by the exact amount of collateral token withdrawn from an obligation.

Similarly, borrows increase total pool shares and user shares.
However, the amount of shares is determined by "liability shares".
Liability shares are calculated for an obligation as `(borrow_amount / cumulative_borrow_rate)`.

Conversely, repays decrease total pool shares and user shares.

When a user deposits, withdraws, borrows or repays, we calculate their effective shares and then set them rather than incrementing/decrementing them.

An obligation can also be liquidated which is a process of repaying and withdrawing.
This adequately updates the pool reward manager and user reward manager deposit shares for the withdraw reserve and liability shares for the repay reserve.

An obligation's debt can also be forgiven.
This is an act of repaying and the liability shares are updated accordingly.

## Differences to Suilend

In Suilend a reserve can have at most 50 rewards.
However, Sui dynamic object model let's us store more data easily.
In Save we're storing the data on the reserve and this means packing and
unpacking it frequently which negatively impacts CU limits.
We lower the number of rewards to 30.
In Save, if we want to add new rewards we will crank old ones to make space
in the reserve if there isn't any.

In Suilend we store the amount of rewards that have been made available to users already.
We keep adding `(total_rewards * time_passed) / (total_time)` every time someone interacts with the manager.
This value is used to transfer the unallocated rewards to the admin.
However, this can be calculated dynamically which avoids storing an extra packed decimal (16 bytes) on each reserve's pool reward (30).

## New ixs

There's a common concept of reward vault and reward vault authority across the ixs.
A reward vault is a token account that stores reward tokens for a specific pool reward.
A reward vault authority is a PDA that is used to sign CPIs into the token program for the reward vault.

```rust
// the seeds for the reward vault authority
[
    b"RewardVaultAuthority",
    lending_market_key,
    vault_token_account_key,
]
```

### `add_pool_reward`

Admin only ix that adds a new pool reward to a reserve's reward manager, either a deposit or a borrow one.
This ix will fail if all slots are occupied.

There's a minimum reward period of 1 hour, no short rewards are allowed.

Each pool reward has a unique vault that holds the reward tokens.
This vault account must be created for the token program before calling this ix.
In this ix we initialize the account as token account and transfer the reward tokens to it from the admin's token account.

### `edit_pool_reward`

Both extending and shortening calculate the difference between total rewards linearly.
Users will still be able to claim rewards they accrued until this point.

#### Cancel

Cancelling a pool reward can be done by setting the end time to 0.
Note that only rewards longer than `solend_sdk::MIN_REWARD_PERIOD_SECS` can be cancelled.
In this case we transfer tokens from the reward vault to the lending market reward token account.

#### Shorten

If the new endtime is in the future, larger than start time and smaller than previous end time
then we shorten the reward period, refunding the unallocated rewards to the lending market
reward token account.

#### Extend

If the new endtime is in the future, larger than start time and larger than previous end time
then we extend the reward period, taking more tokens from the lending market reward token
account.

### `claim_pool_reward`

Permission-less way to claim allocated user liquidity mining rewards.

It finds the UserRewardManager for the reserve and obligation and withdraws
all eligible rewards from it.
The eligible rewards are then transferred to the obligation owners's token account.

Anyone can call this ix which is useful for cranking.

Alternatively, if the obligation is not yet migrated, this does the migration for the obligation as well.
See [Migrations](#migrations) section for more details.

### `close_pool_reward`

Closes a pool reward, making its slot vacant and ready for a new reward.

Before closing a pool reward that pool reward must first be cancelled and all rewards must be claimed by the users.

### `upgrade_reserve`

Temporary ix to upgrade a reserve to LM feature added in @v2.0.2.
Fails if reserve was not sized as @v2.0.2 (ie. has been upgraded or created with @v2.1.00).

Until this ix is called for a Reserve account, all other ixs that try to unpack the Reserve will fail due to size mismatch.

## Changes

This section is partly relevant also to client implementations.
There are breaking changes introduced with this version.

### First byte of each account is discriminator

In @v2.0.2 the first byte of any _initialized_ account was set to the program version, ie. `0x01`.
Once any account is mutably packed in @v2.1.0, the first byte will be set to the account discriminator:

```rust
/// Match the first byte of an account data against this enum to determine
/// the account type.
///
/// # Note
///
/// In versions before @v2.1.0 this byte represented program version.
/// That's why we skip value `1u8`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AccountDiscriminator {
    /// Account is not initialized yet.
    #[default]
    Uninitialized = 0,
    /// [crate::state::LendingMarket]
    LendingMarket = 2,
    /// [crate::state::Reserve]
    Reserve = 3,
    /// [crate::state::Obligation]
    Obligation = 4,
}
```

### Obligations need more rent

Because obligations track user rewards that depend on the number of reserves and the rewards that those reserves have, we now dynamically reallocate size of obligation accounts.

This means that sometimes obligations will need more rent than before and this rent must be (`system_program`) transferred to the obligation account before any interaction with the borrow-lending program.

The calculation for an upper-bound of an obligation size from which rent-exempt balance is calculated:

```math
\overline{s} = 1301 + \sum_{i=0}^{n} 50 + 37 * m_{i}
```

Where $`n`$ is the number of reserves in the obligation and $`m_{i}`$ is the number of pool rewards in obligation's reserve manager $`i`$.

This is an upper-bound because some of those pool rewards might be over and therefore wouldn't be copied to the obligation.

The particular obligation's reserve manager depends on whether the obligation is a borrow or deposit obligation.

### Reserve size increased

Migrated reserve accounts are sized at 5451 bytes.

### CUs increased for all reserve/obligation related ixs

We increase the reserve size and the obligation size which costs more compute when (un)packing.
Additionally, we now write to the reserve account on withdrawal to update the total shares.

All this means more CUs are needed for ixs to succeed.
Additionally, the CUs increase linearly with the number of rewards in each involved reserve.

> TBD: Let's review together the limits used in the present client implementation.

### Reserve account in processor is protected by runtime borrow checker

In @v2.0.2 access to reserve account followed a pattern of unpacking an immutable reference to a cloned memory location, working with it and then mutably packing it back to the original location.
This introduced extra up(pack)ing operations and was prone to double spend bugs.

In this version we're leveraging the `solana_program` framework's usage of `Cell` container.
We keep a `Ref`/`RefMut` around in a wrapper struct along with the unpacked reserve struct and automatically pack it back to the original location when `RefMut` is dropped.
This way we guarantee at runtime that only one mutable reference to the reserve exists at any time.

## Migrations

### `Reserve`

There's a CLI command for `UpgradeReserveToV2_1_0` ix to permission-lessly upgrade a reserve account.
Once upgraded any subsequent calls to this ix for the specific reserve will fail.
The upgrade requires 4832 extra bytes which amounts to ~0.035 $SOL.
Some reserves have extra rent and won't require the full amount.
The `UpgradeReserveToV2_1_0` ix can be delete as soon as all reserves are migrated.

### `Obligation`

To start tracking rewards for an obligation we need to set its shares to the appropriate amount.
They are at 0 before the obligation is fully migrated.

We can call `claim_pool_reward` ix to do this, or any deposit/withdraw/repay/borrow ix.

> Prior to version @2.1.0 there was no concept of liq. mining.
> That means user shares are going to be 0 even if they have a borrow or deposit.
> This ix can be used to start tracking obligation's rewards.

The obligation will be reallocated if it needs more space to add extra rewards.
Client must ensure that the obligation has enough rent-exempt balance.
All obligations would benefit from a extra airdropped rent about `1 + 50 * obligation_reserves` lamports.

### `LendingMarket`

The lending market account is not changed in this version except for the first byte discriminator.

A lending market will be automatically upgraded on the first mutable ix.

## Outstanding work

- [x] Review feature parity with Suilend
  - Looped rewards are not implemented but that's ok
- [x] Consider changing the reward vault authority seed
- [ ] Consider having another admin account to manage the rewards
- [x] Consider spending some rent to the obligations from the reclaimed merkle-tree reward distributor
  - We will fund the obligations to support some of the extra rent
- [x] Discuss CU limits with the Save client team

<!-- List of References -->

[suilend-lm]: https://github.com/solendprotocol/suilend/blob/dc53150416f352053ac3acbb320ee143409c4a5d/contracts/suilend/sources/liquidity_mining.move#L2

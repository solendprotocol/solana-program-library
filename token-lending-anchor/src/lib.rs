use anchor_lang::{
    prelude::*,
    solana_program::{self, entrypoint::ProgramResult},
    Accounts, Key, ToAccountInfos,
};
use anchor_spl::token::Token;
use token_lending_common::state::ReserveConfig;

solana_program::declare_id!("So1endDq2YkqhipRh3WViPa8hdiSpxWy6z3Z6tMCpAo");

#[derive(Accounts)]
pub struct InitLendingMarket<'info> {
    pub lending_market: AccountInfo<'info>,
    pub rent: AccountInfo<'info>,
    pub token_program_id: Program<'info, Token>,
    pub oracle_program_id: AccountInfo<'info>,
    pub switchboard_oracle_program_id: AccountInfo<'info>,
}

pub fn init_lending_market<'a, 'b, 'c, 'info>(
    ctx: CpiContext<'a, 'b, 'c, 'info, InitLendingMarket<'info>>,
    owner: Pubkey,
    quote_currency: [u8; 32],
) -> Result<()> {
    let ix = token_lending_common::instruction::init_lending_market(
        ctx.program.key(),
        owner,
        quote_currency,
        ctx.accounts.lending_market.key(),
        ctx.accounts.oracle_program_id.key(),
        ctx.accounts.switchboard_oracle_program_id.key(),
    );
    solana_program::program::invoke_signed(
        &ix,
        &ctx.accounts.to_account_infos(),
        ctx.signer_seeds,
    )?;
    Ok(())
}

#[derive(Accounts)]
pub struct SetLendingMarketOwner<'info> {
    pub lending_market: AccountInfo<'info>,
    pub lending_market_owner: Signer<'info>,
}

pub fn set_lending_market_owner<'a, 'b, 'c, 'info>(
    ctx: CpiContext<'a, 'b, 'c, 'info, SetLendingMarketOwner<'info>>,
    new_owner: Pubkey,
) -> Result<()> {
    let ix = token_lending_common::instruction::set_lending_market_owner(
        ID,
        ctx.accounts.lending_market.key(),
        ctx.accounts.lending_market_owner.key(),
        new_owner,
    );
    solana_program::program::invoke_signed(&ix, &ctx.accounts.to_account_infos(), ctx.signer_seeds)
        .map_err(Into::into)
}

#[derive(Accounts)]
pub struct InitReserve<'info> {
    pub source_liquidity: AccountInfo<'info>,
    pub destination_collateral: AccountInfo<'info>,
    pub reserve: AccountInfo<'info>,
    pub reserve_liquidity_mint: AccountInfo<'info>,
    pub reserve_liquidity_supply: AccountInfo<'info>,
    pub reserve_liquidity_fee_receiver: AccountInfo<'info>,
    pub reserve_collateral_mint: AccountInfo<'info>,
    pub reserve_collateral_supply: AccountInfo<'info>,
    pub pyth_product: AccountInfo<'info>,
    pub pyth_price: AccountInfo<'info>,
    pub switchboard_feed: AccountInfo<'info>,
    pub lending_market: AccountInfo<'info>,
    pub lending_market_authority: AccountInfo<'info>,
    pub lending_market_owner: AccountInfo<'info>,
    pub user_transfer_authority: AccountInfo<'info>,
    pub token_program: Program<'info, Token>,
    pub clock: AccountInfo<'info>,
    pub rent: AccountInfo<'info>,
}

pub fn init_reserve<'a, 'b, 'c, 'info>(
    ctx: CpiContext<'a, 'b, 'c, 'info, InitReserve<'info>>,
    liquidity_amount: u64,
    config: ReserveConfig,
) -> Result<()> {
    let ix = token_lending_common::instruction::init_reserve(
        ctx.program.key(),
        liquidity_amount,
        config,
        ctx.accounts.source_liquidity.key(),
        ctx.accounts.destination_collateral.key(),
        ctx.accounts.reserve.key(),
        ctx.accounts.reserve_liquidity_mint.key(),
        ctx.accounts.reserve_liquidity_supply.key(),
        ctx.accounts.reserve_collateral_mint.key(),
        ctx.accounts.reserve_collateral_supply.key(),
        ctx.accounts.pyth_product.key(),
        ctx.accounts.pyth_price.key(),
        ctx.accounts.switchboard_feed.key(),
        ctx.accounts.lending_market.key(),
        ctx.accounts.lending_market_owner.key(),
        ctx.accounts.user_transfer_authority.key(),
    );
    solana_program::program::invoke_signed(
        &ix,
        &ctx.accounts.to_account_infos(),
        ctx.signer_seeds,
    )?;
    Ok(())
}

#[derive(Accounts)]
pub struct RefreshReserveAccounts<'info> {
    pub reserve: AccountInfo<'info>,
    pub pyth_price: AccountInfo<'info>,
    pub switchboard_feed: AccountInfo<'info>,
    pub clock_sysvar: Sysvar<'info, Clock>,
}

pub fn refresh_reserve<'a, 'b, 'c, 'info>(
    ctx: CpiContext<'a, 'b, 'c, 'info, RefreshReserveAccounts<'info>>,
) -> Result<()> {
    let ix = token_lending_common::instruction::refresh_reserve(
        ID,
        ctx.accounts.reserve.key(),
        ctx.accounts.pyth_price.key(),
        ctx.accounts.switchboard_feed.key(),
    );
    solana_program::program::invoke_signed(
        &ix,
        &ctx.accounts.to_account_infos(),
        ctx.signer_seeds,
    )?;
    Ok(())
}

#[derive(Accounts)]
pub struct DepositReserveLiquidityAccounts<'info> {
    pub source_liquidity: AccountInfo<'info>,
    pub destination_collateral: AccountInfo<'info>,
    pub reserve: AccountInfo<'info>,
    pub reserve_liquidity_supply: AccountInfo<'info>,
    pub reserve_collateral_mint: AccountInfo<'info>,
    pub lending_market: AccountInfo<'info>,
    pub lending_market_authority: AccountInfo<'info>,
    pub user_transfer_authority: Signer<'info>,
    pub clock_sysvar: AccountInfo<'info>,
    pub token_program: AccountInfo<'info>,
}

pub fn deposit_reserve_liquidity<'a, 'b, 'c, 'info>(
    ctx: CpiContext<'a, 'b, 'c, 'info, DepositReserveLiquidityAccounts<'info>>,
    liquidity_amount: u64,
) -> Result<()> {
    let ix = token_lending_common::instruction::deposit_reserve_liquidity(
        ID,
        liquidity_amount,
        ctx.accounts.source_liquidity.key(),
        ctx.accounts.destination_collateral.key(),
        ctx.accounts.reserve.key(),
        ctx.accounts.reserve_liquidity_supply.key(),
        ctx.accounts.reserve_collateral_mint.key(),
        ctx.accounts.lending_market.key(),
        ctx.accounts.user_transfer_authority.key(),
    );
    solana_program::program::invoke_signed(
        &ix,
        &ctx.accounts.to_account_infos(),
        ctx.signer_seeds,
    )?;
    Ok(())
}

#[derive(Accounts)]
pub struct RedeemReserveCollateralAccounts<'info> {
    pub source_collateral: AccountInfo<'info>,
    pub destination_liquidity: AccountInfo<'info>,
    pub reserve: AccountInfo<'info>,
    pub reserve_collateral_mint: AccountInfo<'info>,
    pub reserve_liquidity_supply: AccountInfo<'info>,
    pub lending_market: AccountInfo<'info>,
    pub user_transfer_authority: Signer<'info>,
    pub clock_sysvar: AccountInfo<'info>,
    pub token_program: AccountInfo<'info>,
}

pub fn redeem_reserve_collateral<'a, 'b, 'c, 'info>(
    ctx: CpiContext<'a, 'b, 'c, 'info, RedeemReserveCollateralAccounts<'info>>,
    collateral_amount: u64,
) -> Result<()> {
    let ix = token_lending_common::instruction::redeem_reserve_collateral(
        ID,
        collateral_amount,
        ctx.accounts.source_collateral.key(),
        ctx.accounts.destination_liquidity.key(),
        ctx.accounts.reserve.key(),
        ctx.accounts.reserve_collateral_mint.key(),
        ctx.accounts.reserve_liquidity_supply.key(),
        ctx.accounts.lending_market.key(),
        ctx.accounts.user_transfer_authority.key(),
    );
    solana_program::program::invoke_signed(
        &ix,
        &ctx.accounts.to_account_infos(),
        ctx.signer_seeds,
    )?;

    Ok(())
}

#[derive(Accounts)]
pub struct InitObligationAccounts<'info> {
    pub obligation: AccountInfo<'info>,
    pub lending_market: AccountInfo<'info>,
    pub obligation_owner: Signer<'info>,
    pub clock_sysvar: AccountInfo<'info>,
    pub rent_sysvar: AccountInfo<'info>,
    pub token_program: AccountInfo<'info>,
}

pub fn init_obligation<'a, 'b, 'c, 'info>(
    ctx: CpiContext<'a, 'b, 'c, 'info, InitObligationAccounts<'info>>,
) -> Result<()> {
    let ix = token_lending_common::instruction::init_obligation(
        ID,
        ctx.accounts.obligation.key(),
        ctx.accounts.lending_market.key(),
        ctx.accounts.obligation_owner.key(),
    );
    solana_program::program::invoke_signed(&ix, &ctx.accounts.to_account_infos(), ctx.signer_seeds)
        .map_err(Into::into)
}

#[derive(Accounts)]
pub struct RefreshObligationAccounts<'info> {
    pub obligation: AccountInfo<'info>,
    pub clock_sysvar: Sysvar<'info, Clock>,
}

pub fn refresh_obligation<'a, 'b, 'c, 'info>(
    ctx: CpiContext<'a, 'b, 'c, 'info, RefreshObligationAccounts<'info>>,
) -> ProgramResult {
    let ix = token_lending_common::instruction::refresh_obligation(
        ID,
        ctx.accounts.obligation.key(),
        ctx.remaining_accounts.iter().map(|k| k.key()).collect(),
    );

    solana_program::program::invoke_signed(&ix, &ctx.accounts.to_account_infos(), ctx.signer_seeds)
        .map_err(Into::into)
}

#[derive(Accounts)]
pub struct DepositObligationCollateralAccounts<'info> {
    pub source_collateral: AccountInfo<'info>,
    pub destination_collateral: AccountInfo<'info>,
    pub deposit_reserve: AccountInfo<'info>,
    pub obligation: AccountInfo<'info>,
    pub lending_market: AccountInfo<'info>,
    pub obligation_owner: AccountInfo<'info>,
    pub user_transfer_authority: Signer<'info>,
    pub clock_sysvar: AccountInfo<'info>,
    pub token_program: AccountInfo<'info>,
}

pub fn deposit_obligation_collateral<'a, 'b, 'c, 'info>(
    ctx: CpiContext<'a, 'b, 'c, 'info, DepositObligationCollateralAccounts<'info>>,
    collateral_amount: u64,
) -> Result<()> {
    let ix = token_lending_common::instruction::deposit_obligation_collateral(
        ID,
        collateral_amount,
        ctx.accounts.source_collateral.key(),
        ctx.accounts.destination_collateral.key(),
        ctx.accounts.deposit_reserve.key(),
        ctx.accounts.obligation.key(),
        ctx.accounts.lending_market.key(),
        ctx.accounts.obligation_owner.key(),
        ctx.accounts.user_transfer_authority.key(),
    );
    solana_program::program::invoke_signed(&ix, &ctx.accounts.to_account_infos(), ctx.signer_seeds)
        .map_err(Into::into)
}

#[derive(Accounts)]
pub struct WithdrawObligationCollateralAccounts<'info> {
    pub source_collateral: AccountInfo<'info>,
    pub destination_collateral: AccountInfo<'info>,
    pub withdraw_reserve: AccountInfo<'info>,
    pub obligation: AccountInfo<'info>,
    pub lending_market: AccountInfo<'info>,
    pub lending_market_authority: AccountInfo<'info>,
    pub obligation_owner: Signer<'info>,
    pub clock_sysvar: AccountInfo<'info>,
    pub token_program: AccountInfo<'info>,
}

pub fn withdraw_obligation_collateral<'a, 'b, 'c, 'info>(
    ctx: CpiContext<'a, 'b, 'c, 'info, WithdrawObligationCollateralAccounts<'info>>,
    collateral_amount: u64,
) -> ProgramResult {
    let ix = token_lending_common::instruction::withdraw_obligation_collateral(
        ID,
        collateral_amount,
        ctx.accounts.source_collateral.key(),
        ctx.accounts.destination_collateral.key(),
        ctx.accounts.withdraw_reserve.key(),
        ctx.accounts.obligation.key(),
        ctx.accounts.lending_market.key(),
        ctx.accounts.obligation_owner.key(),
    );

    solana_program::program::invoke_signed(&ix, &ctx.accounts.to_account_infos(), ctx.signer_seeds)
        .map_err(Into::into)
}

#[derive(Accounts)]
pub struct BorrowObligationLiquidityAccounts<'info> {
    pub source_liquidity: AccountInfo<'info>,
    pub destination_liquidity: AccountInfo<'info>,
    pub borrow_reserve: AccountInfo<'info>,
    pub borrow_reserve_liquidity_fee_receiver: AccountInfo<'info>,
    pub obligation: AccountInfo<'info>,
    pub lending_market: AccountInfo<'info>,
    pub lending_market_authority: AccountInfo<'info>,
    pub obligation_owner: Signer<'info>,
    pub clock_sysvar: AccountInfo<'info>,
    pub token_program: AccountInfo<'info>,
}

pub fn borrow_obligation_liquidity<'a, 'b, 'c, 'info>(
    ctx: CpiContext<'a, 'b, 'c, 'info, BorrowObligationLiquidityAccounts<'info>>,
    liquidity_amount: u64,
) -> Result<()> {
    let host_fee_receiver = ctx.remaining_accounts.get(0);
    let ix = token_lending_common::instruction::borrow_obligation_liquidity(
        ID,
        liquidity_amount,
        ctx.accounts.source_liquidity.key(),
        ctx.accounts.destination_liquidity.key(),
        ctx.accounts.borrow_reserve.key(),
        ctx.accounts.borrow_reserve_liquidity_fee_receiver.key(),
        ctx.accounts.obligation.key(),
        ctx.accounts.lending_market.key(),
        ctx.accounts.obligation_owner.key(),
        host_fee_receiver.map(|k| k.key()),
    );

    solana_program::program::invoke_signed(&ix, &ctx.accounts.to_account_infos(), ctx.signer_seeds)
        .map_err(Into::into)
}

#[derive(Accounts)]
pub struct RepayObligationLiquidityAccounts<'info> {
    pub source_liquidity: AccountInfo<'info>,
    pub destination_liquidity: AccountInfo<'info>,
    pub repay_reserve: AccountInfo<'info>,
    pub obligation: AccountInfo<'info>,
    pub lending_market: AccountInfo<'info>,
    pub user_transfer_authority: Signer<'info>,
    pub clock_sysvar: Sysvar<'info, Clock>,
    pub token_program: Program<'info, Token>,
}

pub fn repay_obligation_liquidity<'a, 'b, 'c, 'info>(
    ctx: CpiContext<'a, 'b, 'c, 'info, RepayObligationLiquidityAccounts<'info>>,
    liquidity_amount: u64,
) -> ProgramResult {
    let ix = token_lending_common::instruction::repay_obligation_liquidity(
        ID,
        liquidity_amount,
        ctx.accounts.source_liquidity.key(),
        ctx.accounts.destination_liquidity.key(),
        ctx.accounts.repay_reserve.key(),
        ctx.accounts.obligation.key(),
        ctx.accounts.lending_market.key(),
        ctx.accounts.user_transfer_authority.key(),
    );
    solana_program::program::invoke_signed(&ix, &ctx.accounts.to_account_infos(), ctx.signer_seeds)
        .map_err(Into::into)
}

#[derive(Accounts)]
pub struct LiquidateObligationAccounts<'info> {
    pub source_liquidity: AccountInfo<'info>,
    pub destination_collateral: AccountInfo<'info>,
    pub repay_reserve: AccountInfo<'info>,
    pub repay_reserve_liquidity_supply: AccountInfo<'info>,
    pub withdraw_reserve: AccountInfo<'info>,
    pub withdraw_reserve_collateral_supply: AccountInfo<'info>,
    pub obligation: AccountInfo<'info>,
    pub lending_market: AccountInfo<'info>,
    pub lending_market_authority: AccountInfo<'info>,
    pub user_transfer_authority: Signer<'info>,
    pub clock_sysvar: Sysvar<'info, Clock>,
    pub token_program: Program<'info, Token>,
}

pub fn liquidate_obligation<'a, 'b, 'c, 'info>(
    ctx: CpiContext<'a, 'b, 'c, 'info, LiquidateObligationAccounts<'info>>,
    liquidity_amount: u64,
) -> ProgramResult {
    let ix = token_lending_common::instruction::liquidate_obligation(
        ID,
        liquidity_amount,
        ctx.accounts.source_liquidity.key(),
        ctx.accounts.destination_collateral.key(),
        ctx.accounts.repay_reserve.key(),
        ctx.accounts.repay_reserve_liquidity_supply.key(),
        ctx.accounts.withdraw_reserve.key(),
        ctx.accounts.withdraw_reserve_collateral_supply.key(),
        ctx.accounts.obligation.key(),
        ctx.accounts.lending_market.key(),
        ctx.accounts.user_transfer_authority.key(),
    );

    solana_program::program::invoke_signed(&ix, &ctx.accounts.to_account_infos(), ctx.signer_seeds)
        .map_err(Into::into)
}

#[derive(Accounts)]
pub struct DepositReserveLiquidityAndObligationCollateralAccounts<'info> {
    pub source_liquidity: AccountInfo<'info>,
    pub user_collateral: AccountInfo<'info>,
    pub reserve: AccountInfo<'info>,
    pub reserve_liquidity_supply: AccountInfo<'info>,
    pub reserve_collateral_mint: AccountInfo<'info>,
    pub lending_market: AccountInfo<'info>,
    pub lending_market_authority: AccountInfo<'info>,
    pub destination_deposit_collateral: AccountInfo<'info>,
    pub obligation: AccountInfo<'info>,
    pub obligation_owner: Signer<'info>,
    pub reserve_liquidity_pyth_oracle: AccountInfo<'info>,
    pub reserve_liquidity_switchboard_oracle: AccountInfo<'info>,
    pub user_transfer_authority: Signer<'info>,
    pub clock_sysvar: Sysvar<'info, Clock>,
    pub token_program: Program<'info, Token>,
}

pub fn deposit_reserve_liquidity_and_obligation_collateral<'a, 'b, 'c, 'info>(
    ctx: CpiContext<
        'a,
        'b,
        'c,
        'info,
        DepositReserveLiquidityAndObligationCollateralAccounts<'info>,
    >,
    liquidity_amount: u64,
) -> ProgramResult {
    let ix = token_lending_common::instruction::deposit_reserve_liquidity_and_obligation_collateral(
        ID,
        liquidity_amount,
        ctx.accounts.source_liquidity.key(),
        ctx.accounts.user_collateral.key(),
        ctx.accounts.reserve.key(),
        ctx.accounts.reserve_liquidity_supply.key(),
        ctx.accounts.reserve_collateral_mint.key(),
        ctx.accounts.lending_market.key(),
        ctx.accounts.destination_deposit_collateral.key(),
        ctx.accounts.obligation.key(),
        ctx.accounts.obligation_owner.key(),
        ctx.accounts.reserve_liquidity_pyth_oracle.key(),
        ctx.accounts.reserve_liquidity_switchboard_oracle.key(),
        ctx.accounts.user_transfer_authority.key(),
    );
    solana_program::program::invoke_signed(&ix, &ctx.accounts.to_account_infos(), ctx.signer_seeds)
        .map_err(Into::into)
}

#[derive(Accounts)]
pub struct WithdrawObligationCollateralAndRedeemReserveCollateralAccounts<'info> {
    pub source_collateral: AccountInfo<'info>,
    pub destination_collateral: AccountInfo<'info>,
    pub withdraw_reserve: AccountInfo<'info>,
    pub obligation: AccountInfo<'info>,
    pub lending_market: AccountInfo<'info>,
    pub lending_market_authority: AccountInfo<'info>,
    pub destination_liquidity: AccountInfo<'info>,
    pub reserve_collateral_mint: AccountInfo<'info>,
    pub reserve_liquidity_supply: AccountInfo<'info>,
    pub obligation_owner: Signer<'info>,
    pub user_transfer_authority: Signer<'info>,
    pub clock_sysvar: AccountInfo<'info>,
    pub token_program: AccountInfo<'info>,
}

pub fn withdraw_obligation_collateral_and_redeem_reserve_collateral<'a, 'b, 'c, 'info>(
    ctx: CpiContext<
        'a,
        'b,
        'c,
        'info,
        WithdrawObligationCollateralAndRedeemReserveCollateralAccounts<'info>,
    >,
    collateral_amount: u64,
) -> ProgramResult {
    let ix =
        token_lending_common::instruction::withdraw_obligation_collateral_and_redeem_reserve_collateral(
            ID,
            collateral_amount,
            ctx.accounts.source_collateral.key(),
            ctx.accounts.destination_collateral.key(),
            ctx.accounts.withdraw_reserve.key(),
            ctx.accounts.obligation.key(),
            ctx.accounts.lending_market.key(),
            ctx.accounts.destination_liquidity.key(),
            ctx.accounts.reserve_collateral_mint.key(),
            ctx.accounts.reserve_liquidity_supply.key(),
            ctx.accounts.obligation_owner.key(),
            ctx.accounts.user_transfer_authority.key(),
        );

    solana_program::program::invoke_signed(&ix, &ctx.accounts.to_account_infos(), ctx.signer_seeds)
        .map_err(Into::into)
}

#[derive(Accounts)]
pub struct UpdateReserveConfig<'info> {
    pub reserve: AccountInfo<'info>,
    pub lending_market: AccountInfo<'info>,
    pub lending_market_authority: AccountInfo<'info>,
    pub lending_market_owner: Signer<'info>,
    pub pyth_product: AccountInfo<'info>,
    pub pyth_price: AccountInfo<'info>,
    pub switchboard_feed: AccountInfo<'info>,
}

pub fn update_reserve_config<'a, 'b, 'c, 'info>(
    ctx: CpiContext<'a, 'b, 'c, 'info, UpdateReserveConfig<'info>>,
    config: ReserveConfig,
) -> Result<()> {
    let ix = token_lending_common::instruction::update_reserve_config(
        ID,
        config,
        ctx.accounts.reserve.key(),
        ctx.accounts.lending_market.key(),
        ctx.accounts.lending_market_owner.key(),
        ctx.accounts.pyth_product.key(),
        ctx.accounts.pyth_price.key(),
        ctx.accounts.switchboard_feed.key(),
    );
    solana_program::program::invoke_signed(&ix, &ctx.accounts.to_account_infos(), ctx.signer_seeds)
        .map_err(Into::into)
}

#[derive(Accounts)]
pub struct LiquidateObligationAndRedeemReserveCollateralAccounts<'info> {
    pub source_liquidity: AccountInfo<'info>,
    pub destination_collateral: AccountInfo<'info>,
    pub destination_liquidity: AccountInfo<'info>,
    pub repay_reserve: AccountInfo<'info>,
    pub repay_reserve_liquidity_supply: AccountInfo<'info>,
    pub withdraw_reserve: AccountInfo<'info>,
    pub withdraw_reserve_collateral_mint: AccountInfo<'info>,
    pub withdraw_reserve_collateral_supply: AccountInfo<'info>,
    pub withdraw_reserve_liquidity_supply: AccountInfo<'info>,
    pub withdraw_reserve_liquidity_fee_receiver: AccountInfo<'info>,
    pub obligation: AccountInfo<'info>,
    pub lending_market: AccountInfo<'info>,
    pub lending_market_authority: AccountInfo<'info>,
    pub user_transfer_authority: Signer<'info>,
    pub token_program: Program<'info, Token>,
}

pub fn liquidate_obligation_and_redeem_reserve_collateral<'a, 'b, 'c, 'info>(
    ctx: CpiContext<
        'a,
        'b,
        'c,
        'info,
        LiquidateObligationAndRedeemReserveCollateralAccounts<'info>,
    >,
    liquidity_amount: u64,
) -> ProgramResult {
    let ix = token_lending_common::instruction::liquidate_obligation_and_redeem_reserve_collateral(
        ID,
        liquidity_amount,
        ctx.accounts.source_liquidity.key(),
        ctx.accounts.destination_collateral.key(),
        ctx.accounts.destination_liquidity.key(),
        ctx.accounts.repay_reserve.key(),
        ctx.accounts.repay_reserve_liquidity_supply.key(),
        ctx.accounts.withdraw_reserve.key(),
        ctx.accounts.withdraw_reserve_collateral_mint.key(),
        ctx.accounts.withdraw_reserve_collateral_supply.key(),
        ctx.accounts.withdraw_reserve_liquidity_supply.key(),
        ctx.accounts.withdraw_reserve_liquidity_fee_receiver.key(),
        ctx.accounts.obligation.key(),
        ctx.accounts.lending_market.key(),
        ctx.accounts.user_transfer_authority.key(),
    );

    solana_program::program::invoke_signed(&ix, &ctx.accounts.to_account_infos(), ctx.signer_seeds)
        .map_err(Into::into)
}

#[derive(Accounts)]
pub struct RedeemFees<'info> {
    pub reserve: AccountInfo<'info>,
    pub reserve_liquidity_fee_receiver: AccountInfo<'info>,
    pub reserve_liquidity_supply: AccountInfo<'info>,
    pub lending_market: AccountInfo<'info>,
    pub lending_market_authority: AccountInfo<'info>,
    pub token_program: Program<'info, Token>,
}

pub fn redeem_fees<'a, 'b, 'c, 'info>(
    ctx: CpiContext<'a, 'b, 'c, 'info, RedeemFees<'info>>,
) -> Result<()> {
    let ix = token_lending_common::instruction::redeem_fees(
        ID,
        ctx.accounts.reserve.key(),
        ctx.accounts.reserve_liquidity_fee_receiver.key(),
        ctx.accounts.reserve_liquidity_supply.key(),
        ctx.accounts.lending_market.key(),
    );
    solana_program::program::invoke_signed(&ix, &ctx.accounts.to_account_infos(), ctx.signer_seeds)
        .map_err(Into::into)
}

#[derive(Clone)]
pub struct Solend;

impl anchor_lang::Id for Solend {
    fn id() -> Pubkey {
        ID
    }
}

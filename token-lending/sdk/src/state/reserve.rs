use super::*;
use crate::{
    error::LendingError,
    math::{Decimal, Rate, TryAdd, TryDiv, TryMul, TrySub},
};
use arrayref::{array_mut_ref, array_ref, array_refs, mut_array_refs};
use num_derive::FromPrimitive;
use num_traits::FromPrimitive;
use solana_program::{
    clock::Slot,
    entrypoint::ProgramResult,
    msg,
    program_error::ProgramError,
    program_pack::{IsInitialized, Pack, Sealed},
    pubkey::{Pubkey, PUBKEY_BYTES},
};
use std::{
    cmp::{max, min, Ordering},
    convert::{TryFrom, TryInto},
};

/// Percentage of an obligation that can be repaid during each liquidation call
pub const LIQUIDATION_CLOSE_FACTOR: u8 = 20;

/// Obligation borrow amount that is small enough to close out
pub const LIQUIDATION_CLOSE_AMOUNT: u64 = 2;

/// Maximum quote currency value that can be liquidated in 1 liquidate_obligation call
pub const MAX_LIQUIDATABLE_VALUE_AT_ONCE: u64 = 500_000;

/// Lending market reserve state
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reserve {
    /// Version of the struct
    pub version: u8,
    /// Last slot when supply and rates updated
    pub last_update: LastUpdate,
    /// Lending market address
    pub lending_market: Pubkey,
    /// Reserve liquidity
    pub liquidity: ReserveLiquidity,
    /// Reserve collateral
    pub collateral: ReserveCollateral,
    /// Reserve configuration values
    pub config: ReserveConfig,
    /// Outflow Rate Limiter (denominated in tokens)
    pub rate_limiter: RateLimiter,
}

impl Reserve {
    /// Create a new reserve
    pub fn new(params: InitReserveParams) -> Self {
        let mut reserve = Self::default();
        Self::init(&mut reserve, params);
        reserve
    }

    /// Initialize a reserve
    pub fn init(&mut self, params: InitReserveParams) {
        self.version = PROGRAM_VERSION;
        self.last_update = LastUpdate::new(params.current_slot);
        self.lending_market = params.lending_market;
        self.liquidity = params.liquidity;
        self.collateral = params.collateral;
        self.config = params.config;
        self.rate_limiter = RateLimiter::new(params.rate_limiter_config, params.current_slot);
    }

    /// get borrow weight. Guaranteed to be greater than 1
    pub fn borrow_weight(&self) -> Decimal {
        Decimal::one()
            .try_add(Decimal::from_bps(self.config.added_borrow_weight_bps))
            .unwrap()
    }

    /// get loan to value ratio as a Rate
    pub fn loan_to_value_ratio(&self) -> Rate {
        Rate::from_percent(self.config.loan_to_value_ratio)
    }

    /// find current market value of tokens
    pub fn market_value(&self, liquidity_amount: Decimal) -> Result<Decimal, ProgramError> {
        self.liquidity
            .market_price
            .try_mul(liquidity_amount)?
            .try_div(Decimal::from(
                (10u128)
                    .checked_pow(self.liquidity.mint_decimals as u32)
                    .ok_or(LendingError::MathOverflow)?,
            ))
    }

    /// find the current upper bound market value of tokens.
    /// ie max(market_price, smoothed_market_price) * liquidity_amount
    pub fn market_value_upper_bound(
        &self,
        liquidity_amount: Decimal,
    ) -> Result<Decimal, ProgramError> {
        let price_upper_bound = std::cmp::max(
            self.liquidity.market_price,
            self.liquidity.smoothed_market_price,
        );

        price_upper_bound
            .try_mul(liquidity_amount)?
            .try_div(Decimal::from(
                (10u128)
                    .checked_pow(self.liquidity.mint_decimals as u32)
                    .ok_or(LendingError::MathOverflow)?,
            ))
    }

    /// find the current lower bound market value of tokens.
    /// ie min(market_price, smoothed_market_price) * liquidity_amount
    pub fn market_value_lower_bound(
        &self,
        liquidity_amount: Decimal,
    ) -> Result<Decimal, ProgramError> {
        let price_lower_bound = std::cmp::min(
            self.liquidity.market_price,
            self.liquidity.smoothed_market_price,
        );

        price_lower_bound
            .try_mul(liquidity_amount)?
            .try_div(Decimal::from(
                (10u128)
                    .checked_pow(self.liquidity.mint_decimals as u32)
                    .ok_or(LendingError::MathOverflow)?,
            ))
    }

    /// Record deposited liquidity and return amount of collateral tokens to mint
    pub fn deposit_liquidity(&mut self, liquidity_amount: u64) -> Result<u64, ProgramError> {
        let collateral_amount = self
            .collateral_exchange_rate()?
            .liquidity_to_collateral(liquidity_amount)?;

        self.liquidity.deposit(liquidity_amount)?;
        self.collateral.mint(collateral_amount)?;

        Ok(collateral_amount)
    }

    /// Record redeemed collateral and return amount of liquidity to withdraw
    pub fn redeem_collateral(&mut self, collateral_amount: u64) -> Result<u64, ProgramError> {
        let collateral_exchange_rate = self.collateral_exchange_rate()?;
        let liquidity_amount =
            collateral_exchange_rate.collateral_to_liquidity(collateral_amount)?;

        self.collateral.burn(collateral_amount)?;
        self.liquidity.withdraw(liquidity_amount)?;

        Ok(liquidity_amount)
    }

    /// Calculate the current borrow rate
    pub fn current_borrow_rate(&self) -> Result<Rate, ProgramError> {
        let utilization_rate = min(self.liquidity.utilization_rate()?, Rate::one());
        let optimal_utilization_rate = Rate::from_percent(self.config.optimal_utilization_rate);
        let unhealthy_utilization_rate = Rate::from_percent(self.config.unhealthy_utilization_rate);
        if utilization_rate <= optimal_utilization_rate {
            let min_rate = Rate::from_percent(self.config.min_borrow_rate);

            if optimal_utilization_rate == Rate::zero() {
                return Ok(min_rate);
            }

            let normalized_rate = utilization_rate.try_div(optimal_utilization_rate)?;
            let rate_range = Rate::from_percent(
                self.config
                    .optimal_borrow_rate
                    .checked_sub(self.config.min_borrow_rate)
                    .ok_or(LendingError::MathOverflow)?,
            );

            Ok(normalized_rate.try_mul(rate_range)?.try_add(min_rate)?)
        } else if utilization_rate <= unhealthy_utilization_rate {
            let weight = utilization_rate
                .try_sub(optimal_utilization_rate)?
                .try_div(unhealthy_utilization_rate.try_sub(optimal_utilization_rate)?)?;

            let optimal_borrow_rate = Rate::from_percent(self.config.optimal_borrow_rate);
            let max_borrow_rate = Rate::from_percent(self.config.max_borrow_rate);
            let rate_range = max_borrow_rate.try_sub(optimal_borrow_rate)?;

            weight.try_mul(rate_range)?.try_add(optimal_borrow_rate)
        } else {
            let weight: Decimal = utilization_rate
                .try_sub(unhealthy_utilization_rate)?
                .try_div(Rate::from_percent(
                    100u8
                        .checked_sub(self.config.unhealthy_utilization_rate)
                        .ok_or(LendingError::MathOverflow)?,
                ))?
                .into();

            let max_borrow_rate = Rate::from_percent(self.config.max_borrow_rate);
            let super_max_borrow_rate = Rate::from_percent_u64(self.config.super_max_borrow_rate);
            let rate_range: Decimal = super_max_borrow_rate.try_sub(max_borrow_rate)?.into();

            // if done with just Rates, this computation can overflow. so we temporarily convert to Decimal
            // and back to Rate
            weight
                .try_mul(rate_range)?
                .try_add(max_borrow_rate.into())?
                .try_into()
        }
    }

    /// Collateral exchange rate
    pub fn collateral_exchange_rate(&self) -> Result<CollateralExchangeRate, ProgramError> {
        let total_liquidity = self.liquidity.total_supply()?;
        self.collateral.exchange_rate(total_liquidity)
    }

    /// Update borrow rate and accrue interest
    pub fn accrue_interest(&mut self, current_slot: Slot) -> ProgramResult {
        let slots_elapsed = self.last_update.slots_elapsed(current_slot)?;
        if slots_elapsed > 0 {
            let current_borrow_rate = self.current_borrow_rate()?;
            let take_rate = Rate::from_percent(self.config.protocol_take_rate);
            self.liquidity
                .compound_interest(current_borrow_rate, slots_elapsed, take_rate)?;
        }
        Ok(())
    }

    /// Borrow liquidity up to a maximum market value
    pub fn calculate_borrow(
        &self,
        amount_to_borrow: u64,
        max_borrow_value: Decimal,
        remaining_reserve_borrow: Decimal,
    ) -> Result<CalculateBorrowResult, ProgramError> {
        // @TODO: add lookup table https://git.io/JOCYq
        let decimals = 10u64
            .checked_pow(self.liquidity.mint_decimals as u32)
            .ok_or(LendingError::MathOverflow)?;
        if amount_to_borrow == u64::MAX {
            let borrow_amount = max_borrow_value
                .try_mul(decimals)?
                .try_div(max(
                    self.liquidity.market_price,
                    self.liquidity.smoothed_market_price,
                ))?
                .try_div(self.borrow_weight())?
                .min(remaining_reserve_borrow)
                .min(self.liquidity.available_amount.into());
            let (borrow_fee, host_fee) = self
                .config
                .fees
                .calculate_borrow_fees(borrow_amount, FeeCalculation::Inclusive)?;
            let receive_amount = borrow_amount
                .try_floor_u64()?
                .checked_sub(borrow_fee)
                .ok_or(LendingError::MathOverflow)?;

            Ok(CalculateBorrowResult {
                borrow_amount,
                receive_amount,
                borrow_fee,
                host_fee,
            })
        } else {
            let receive_amount = amount_to_borrow;
            let borrow_amount = Decimal::from(receive_amount);
            let (borrow_fee, host_fee) = self
                .config
                .fees
                .calculate_borrow_fees(borrow_amount, FeeCalculation::Exclusive)?;

            let borrow_amount = borrow_amount.try_add(borrow_fee.into())?;
            let borrow_value = self
                .market_value_upper_bound(borrow_amount)?
                .try_mul(self.borrow_weight())?;
            if borrow_value > max_borrow_value {
                msg!("Borrow value cannot exceed maximum borrow value");
                return Err(LendingError::BorrowTooLarge.into());
            }

            Ok(CalculateBorrowResult {
                borrow_amount,
                receive_amount,
                borrow_fee,
                host_fee,
            })
        }
    }

    /// Repay liquidity up to the borrowed amount
    pub fn calculate_repay(
        &self,
        amount_to_repay: u64,
        borrowed_amount: Decimal,
    ) -> Result<CalculateRepayResult, ProgramError> {
        let settle_amount = if amount_to_repay == u64::MAX {
            borrowed_amount
        } else {
            Decimal::from(amount_to_repay).min(borrowed_amount)
        };
        let repay_amount = settle_amount.try_ceil_u64()?;

        Ok(CalculateRepayResult {
            settle_amount,
            repay_amount,
        })
    }

    /// Liquidate some or all of an unhealthy obligation
    pub fn calculate_liquidation(
        &self,
        amount_to_liquidate: u64,
        obligation: &Obligation,
        liquidity: &ObligationLiquidity,
        collateral: &ObligationCollateral,
    ) -> Result<CalculateLiquidationResult, ProgramError> {
        let bonus_rate = Rate::from_percent(self.config.liquidation_bonus).try_add(Rate::one())?;

        let max_amount = if amount_to_liquidate == u64::MAX {
            liquidity.borrowed_amount_wads
        } else {
            Decimal::from(amount_to_liquidate).min(liquidity.borrowed_amount_wads)
        };

        let settle_amount;
        let repay_amount;
        let withdraw_amount;

        // do a full liquidation if the market value of the borrow is less than one.
        if liquidity.market_value <= Decimal::one() {
            let liquidation_value = liquidity.market_value.try_mul(bonus_rate)?;
            match liquidation_value.cmp(&collateral.market_value) {
                Ordering::Greater => {
                    let repay_pct = collateral.market_value.try_div(liquidation_value)?;
                    settle_amount = liquidity.borrowed_amount_wads.try_mul(repay_pct)?;
                    repay_amount = settle_amount.try_ceil_u64()?;
                    withdraw_amount = collateral.deposited_amount;
                }
                Ordering::Equal => {
                    settle_amount = liquidity.borrowed_amount_wads;
                    repay_amount = settle_amount.try_ceil_u64()?;
                    withdraw_amount = collateral.deposited_amount;
                }
                Ordering::Less => {
                    let withdraw_pct = liquidation_value.try_div(collateral.market_value)?;

                    settle_amount = liquidity.borrowed_amount_wads;
                    repay_amount = settle_amount.try_ceil_u64()?;
                    if repay_amount == 0 {
                        msg!("repay amount is zero");
                        return Err(LendingError::LiquidationTooSmall.into());
                    }

                    withdraw_amount = max(
                        Decimal::from(collateral.deposited_amount)
                            .try_mul(withdraw_pct)?
                            .try_floor_u64()?,
                        // if withdraw_amount gets floored to zero and repay amount is non-zero,
                        // we set the withdraw_amount to 1. We do this so dust obligations get
                        // cleaned up.
                        //
                        // safety: technically this gives the liquidator more of a bonus, but this
                        // can happen at most once per ObligationLiquidity so I don't think this
                        // can be exploited to cause bad debt or anything.
                        1,
                    );
                }
            }
        } else {
            // partial liquidation
            // calculate settle_amount and withdraw_amount, repay_amount is settle_amount rounded
            let liquidation_amount = obligation
                .max_liquidation_amount(liquidity)?
                .min(max_amount);
            let liquidation_pct = liquidation_amount.try_div(liquidity.borrowed_amount_wads)?;
            let liquidation_value = liquidity
                .market_value
                .try_mul(liquidation_pct)?
                .try_mul(bonus_rate)?;

            match liquidation_value.cmp(&collateral.market_value) {
                Ordering::Greater => {
                    let repay_pct = collateral.market_value.try_div(liquidation_value)?;
                    settle_amount = liquidation_amount.try_mul(repay_pct)?;
                    repay_amount = settle_amount.try_ceil_u64()?;
                    withdraw_amount = collateral.deposited_amount;
                }
                Ordering::Equal => {
                    settle_amount = liquidation_amount;
                    repay_amount = settle_amount.try_ceil_u64()?;
                    withdraw_amount = collateral.deposited_amount;
                }
                Ordering::Less => {
                    let withdraw_pct = liquidation_value.try_div(collateral.market_value)?;
                    settle_amount = liquidation_amount;
                    repay_amount = settle_amount.try_ceil_u64()?;
                    withdraw_amount = Decimal::from(collateral.deposited_amount)
                        .try_mul(withdraw_pct)?
                        .try_floor_u64()?;
                }
            }
        }

        Ok(CalculateLiquidationResult {
            settle_amount,
            repay_amount,
            withdraw_amount,
        })
    }

    /// Calculate protocol cut of liquidation bonus always at least 1 lamport
    pub fn calculate_protocol_liquidation_fee(
        &self,
        amount_liquidated: u64,
    ) -> Result<u64, ProgramError> {
        let bonus_rate = Rate::from_percent(self.config.liquidation_bonus).try_add(Rate::one())?;
        let amount_liquidated_wads = Decimal::from(amount_liquidated);

        let bonus = amount_liquidated_wads.try_sub(amount_liquidated_wads.try_div(bonus_rate)?)?;

        // After deploying must update all reserves to set liquidation fee then redeploy with this line instead of hardcode
        let protocol_fee = std::cmp::max(
            bonus
                .try_mul(Rate::from_percent(self.config.protocol_liquidation_fee))?
                .try_ceil_u64()?,
            1,
        );
        Ok(protocol_fee)
    }

    /// Calculate protocol fee redemption accounting for availible liquidity and accumulated fees
    pub fn calculate_redeem_fees(&self) -> Result<u64, ProgramError> {
        Ok(min(
            self.liquidity.available_amount,
            self.liquidity
                .accumulated_protocol_fees_wads
                .try_floor_u64()?,
        ))
    }
}

/// Initialize a reserve
pub struct InitReserveParams {
    /// Last slot when supply and rates updated
    pub current_slot: Slot,
    /// Lending market address
    pub lending_market: Pubkey,
    /// Reserve liquidity
    pub liquidity: ReserveLiquidity,
    /// Reserve collateral
    pub collateral: ReserveCollateral,
    /// Reserve configuration values
    pub config: ReserveConfig,
    /// rate limiter config
    pub rate_limiter_config: RateLimiterConfig,
}

/// Calculate borrow result
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalculateBorrowResult {
    /// Total amount of borrow including fees
    pub borrow_amount: Decimal,
    /// Borrow amount portion of total amount
    pub receive_amount: u64,
    /// Loan origination fee
    pub borrow_fee: u64,
    /// Host fee portion of origination fee
    pub host_fee: u64,
}

/// Calculate repay result
#[derive(Debug)]
pub struct CalculateRepayResult {
    /// Amount of liquidity that is settled from the obligation.
    pub settle_amount: Decimal,
    /// Amount that will be repaid as u64
    pub repay_amount: u64,
}

/// Calculate liquidation result
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalculateLiquidationResult {
    /// Amount of liquidity that is settled from the obligation. It includes
    /// the amount of loan that was defaulted if collateral is depleted.
    pub settle_amount: Decimal,
    /// Amount that will be repaid as u64
    pub repay_amount: u64,
    /// Amount of collateral to withdraw in exchange for repay amount
    pub withdraw_amount: u64,
}

/// Reserve liquidity
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReserveLiquidity {
    /// Reserve liquidity mint address
    pub mint_pubkey: Pubkey,
    /// Reserve liquidity mint decimals
    pub mint_decimals: u8,
    /// Reserve liquidity supply address
    pub supply_pubkey: Pubkey,
    /// Reserve liquidity pyth oracle account
    pub pyth_oracle_pubkey: Pubkey,
    /// Reserve liquidity switchboard oracle account
    pub switchboard_oracle_pubkey: Pubkey,
    /// Reserve liquidity available
    pub available_amount: u64,
    /// Reserve liquidity borrowed
    pub borrowed_amount_wads: Decimal,
    /// Reserve liquidity cumulative borrow rate
    pub cumulative_borrow_rate_wads: Decimal,
    /// Reserve cumulative protocol fees
    pub accumulated_protocol_fees_wads: Decimal,
    /// Reserve liquidity market price in quote currency
    pub market_price: Decimal,
    /// Smoothed reserve liquidity market price for the liquidity (eg TWAP, VWAP, EMA)
    pub smoothed_market_price: Decimal,
}

impl ReserveLiquidity {
    /// Create a new reserve liquidity
    pub fn new(params: NewReserveLiquidityParams) -> Self {
        Self {
            mint_pubkey: params.mint_pubkey,
            mint_decimals: params.mint_decimals,
            supply_pubkey: params.supply_pubkey,
            pyth_oracle_pubkey: params.pyth_oracle_pubkey,
            switchboard_oracle_pubkey: params.switchboard_oracle_pubkey,
            available_amount: 0,
            borrowed_amount_wads: Decimal::zero(),
            cumulative_borrow_rate_wads: Decimal::one(),
            accumulated_protocol_fees_wads: Decimal::zero(),
            market_price: params.market_price,
            smoothed_market_price: params.smoothed_market_price,
        }
    }

    /// Calculate the total reserve supply including active loans
    pub fn total_supply(&self) -> Result<Decimal, ProgramError> {
        Decimal::from(self.available_amount)
            .try_add(self.borrowed_amount_wads)?
            .try_sub(self.accumulated_protocol_fees_wads)
    }

    /// Add liquidity to available amount
    pub fn deposit(&mut self, liquidity_amount: u64) -> ProgramResult {
        self.available_amount = self
            .available_amount
            .checked_add(liquidity_amount)
            .ok_or(LendingError::MathOverflow)?;
        Ok(())
    }

    /// Remove liquidity from available amount
    pub fn withdraw(&mut self, liquidity_amount: u64) -> ProgramResult {
        if liquidity_amount > self.available_amount {
            msg!("Withdraw amount cannot exceed available amount");
            return Err(LendingError::InsufficientLiquidity.into());
        }
        self.available_amount = self
            .available_amount
            .checked_sub(liquidity_amount)
            .ok_or(LendingError::MathOverflow)?;
        Ok(())
    }

    /// Subtract borrow amount from available liquidity and add to borrows
    pub fn borrow(&mut self, borrow_decimal: Decimal) -> ProgramResult {
        let borrow_amount = borrow_decimal.try_floor_u64()?;
        if borrow_amount > self.available_amount {
            msg!("Borrow amount cannot exceed available amount");
            return Err(LendingError::InsufficientLiquidity.into());
        }

        self.available_amount = self
            .available_amount
            .checked_sub(borrow_amount)
            .ok_or(LendingError::MathOverflow)?;
        self.borrowed_amount_wads = self.borrowed_amount_wads.try_add(borrow_decimal)?;

        Ok(())
    }

    /// Add repay amount to available liquidity and subtract settle amount from total borrows
    pub fn repay(&mut self, repay_amount: u64, settle_amount: Decimal) -> ProgramResult {
        self.available_amount = self
            .available_amount
            .checked_add(repay_amount)
            .ok_or(LendingError::MathOverflow)?;
        let safe_settle_amount = settle_amount.min(self.borrowed_amount_wads);
        self.borrowed_amount_wads = self.borrowed_amount_wads.try_sub(safe_settle_amount)?;

        Ok(())
    }

    /// Forgive bad debt. This essentially socializes the loss across all ctoken holders of
    /// this reserve.
    pub fn forgive_debt(&mut self, liquidity_amount: Decimal) -> ProgramResult {
        self.borrowed_amount_wads = self.borrowed_amount_wads.try_sub(liquidity_amount)?;

        Ok(())
    }

    /// Subtract settle amount from accumulated_protocol_fees_wads and withdraw_amount from available liquidity
    pub fn redeem_fees(&mut self, withdraw_amount: u64) -> ProgramResult {
        self.available_amount = self
            .available_amount
            .checked_sub(withdraw_amount)
            .ok_or(LendingError::MathOverflow)?;
        self.accumulated_protocol_fees_wads = self
            .accumulated_protocol_fees_wads
            .try_sub(Decimal::from(withdraw_amount))?;

        Ok(())
    }

    /// Calculate the liquidity utilization rate of the reserve
    pub fn utilization_rate(&self) -> Result<Rate, ProgramError> {
        let total_supply = self.total_supply()?;
        if total_supply == Decimal::zero() || self.borrowed_amount_wads == Decimal::zero() {
            return Ok(Rate::zero());
        }
        let denominator = self
            .borrowed_amount_wads
            .try_add(Decimal::from(self.available_amount))?;
        self.borrowed_amount_wads.try_div(denominator)?.try_into()
    }

    /// Compound current borrow rate over elapsed slots
    fn compound_interest(
        &mut self,
        current_borrow_rate: Rate,
        slots_elapsed: u64,
        take_rate: Rate,
    ) -> ProgramResult {
        let slot_interest_rate = current_borrow_rate.try_div(SLOTS_PER_YEAR)?;
        let compounded_interest_rate = Rate::one()
            .try_add(slot_interest_rate)?
            .try_pow(slots_elapsed)?;
        self.cumulative_borrow_rate_wads = self
            .cumulative_borrow_rate_wads
            .try_mul(compounded_interest_rate)?;

        let net_new_debt = self
            .borrowed_amount_wads
            .try_mul(compounded_interest_rate)?
            .try_sub(self.borrowed_amount_wads)?;

        self.accumulated_protocol_fees_wads = net_new_debt
            .try_mul(take_rate)?
            .try_add(self.accumulated_protocol_fees_wads)?;

        self.borrowed_amount_wads = self.borrowed_amount_wads.try_add(net_new_debt)?;
        Ok(())
    }
}

/// Create a new reserve liquidity
pub struct NewReserveLiquidityParams {
    /// Reserve liquidity mint address
    pub mint_pubkey: Pubkey,
    /// Reserve liquidity mint decimals
    pub mint_decimals: u8,
    /// Reserve liquidity supply address
    pub supply_pubkey: Pubkey,
    /// Reserve liquidity pyth oracle account
    pub pyth_oracle_pubkey: Pubkey,
    /// Reserve liquidity switchboard oracle account
    pub switchboard_oracle_pubkey: Pubkey,
    /// Reserve liquidity market price in quote currency
    pub market_price: Decimal,
    /// Smoothed reserve liquidity market price in quote currency
    pub smoothed_market_price: Decimal,
}

/// Reserve collateral
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReserveCollateral {
    /// Reserve collateral mint address
    pub mint_pubkey: Pubkey,
    /// Reserve collateral mint supply, used for exchange rate
    pub mint_total_supply: u64,
    /// Reserve collateral supply address
    pub supply_pubkey: Pubkey,
}

impl ReserveCollateral {
    /// Create a new reserve collateral
    pub fn new(params: NewReserveCollateralParams) -> Self {
        Self {
            mint_pubkey: params.mint_pubkey,
            mint_total_supply: 0,
            supply_pubkey: params.supply_pubkey,
        }
    }

    /// Add collateral to total supply
    pub fn mint(&mut self, collateral_amount: u64) -> ProgramResult {
        self.mint_total_supply = self
            .mint_total_supply
            .checked_add(collateral_amount)
            .ok_or(LendingError::MathOverflow)?;
        Ok(())
    }

    /// Remove collateral from total supply
    pub fn burn(&mut self, collateral_amount: u64) -> ProgramResult {
        self.mint_total_supply = self
            .mint_total_supply
            .checked_sub(collateral_amount)
            .ok_or(LendingError::MathOverflow)?;
        Ok(())
    }

    /// Return the current collateral exchange rate.
    fn exchange_rate(
        &self,
        total_liquidity: Decimal,
    ) -> Result<CollateralExchangeRate, ProgramError> {
        let rate = if self.mint_total_supply == 0 || total_liquidity == Decimal::zero() {
            Rate::from_scaled_val(INITIAL_COLLATERAL_RATE)
        } else {
            let mint_total_supply = Decimal::from(self.mint_total_supply);
            Rate::try_from(mint_total_supply.try_div(total_liquidity)?)?
        };

        Ok(CollateralExchangeRate(rate))
    }
}

/// Create a new reserve collateral
pub struct NewReserveCollateralParams {
    /// Reserve collateral mint address
    pub mint_pubkey: Pubkey,
    /// Reserve collateral supply address
    pub supply_pubkey: Pubkey,
}

/// Collateral exchange rate
#[derive(Clone, Copy, Debug)]
pub struct CollateralExchangeRate(Rate);

impl CollateralExchangeRate {
    /// Convert reserve collateral to liquidity
    pub fn collateral_to_liquidity(&self, collateral_amount: u64) -> Result<u64, ProgramError> {
        self.decimal_collateral_to_liquidity(collateral_amount.into())?
            .try_floor_u64()
    }

    /// Convert reserve collateral to liquidity
    pub fn decimal_collateral_to_liquidity(
        &self,
        collateral_amount: Decimal,
    ) -> Result<Decimal, ProgramError> {
        collateral_amount.try_div(self.0)
    }

    /// Convert reserve liquidity to collateral
    pub fn liquidity_to_collateral(&self, liquidity_amount: u64) -> Result<u64, ProgramError> {
        self.decimal_liquidity_to_collateral(liquidity_amount.into())?
            .try_floor_u64()
    }

    /// Convert reserve liquidity to collateral
    pub fn decimal_liquidity_to_collateral(
        &self,
        liquidity_amount: Decimal,
    ) -> Result<Decimal, ProgramError> {
        liquidity_amount.try_mul(self.0)
    }
}

impl From<CollateralExchangeRate> for Rate {
    fn from(exchange_rate: CollateralExchangeRate) -> Self {
        exchange_rate.0
    }
}

/// Reserve configuration values
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReserveConfig {
    /// Optimal utilization rate, as a percentage
    pub optimal_utilization_rate: u8,
    /// Unhealthy utilization rate, as a percentage
    pub unhealthy_utilization_rate: u8,
    /// Target ratio of the value of borrows to deposits, as a percentage
    /// 0 if use as collateral is disabled
    pub loan_to_value_ratio: u8,
    /// Bonus a liquidator gets when repaying part of an unhealthy obligation, as a percentage
    pub liquidation_bonus: u8,
    /// Loan to value ratio at which an obligation can be liquidated, as a percentage
    pub liquidation_threshold: u8,
    /// Min borrow APY
    pub min_borrow_rate: u8,
    /// Optimal (utilization) borrow APY
    pub optimal_borrow_rate: u8,
    /// Max borrow APY
    pub max_borrow_rate: u8,
    /// Supermax borrow APY
    pub super_max_borrow_rate: u64,
    /// Program owner fees assessed, separate from gains due to interest accrual
    pub fees: ReserveFees,
    /// Maximum deposit limit of liquidity in native units, u64::MAX for inf
    pub deposit_limit: u64,
    /// Borrows disabled
    pub borrow_limit: u64,
    /// Reserve liquidity fee receiver address
    pub fee_receiver: Pubkey,
    /// Cut of the liquidation bonus that the protocol receives, as a percentage
    pub protocol_liquidation_fee: u8,
    /// Protocol take rate is the amount borrowed interest protocol recieves, as a percentage  
    pub protocol_take_rate: u8,
    /// Added borrow weight in basis points. THIS FIELD SHOULD NEVER BE USED DIRECTLY. Always use
    /// borrow_weight()
    pub added_borrow_weight_bps: u64,
    /// Type of the reserve (Regular, Isolated)
    pub reserve_type: ReserveType,
}

/// validates reserve configs
#[inline(always)]
pub fn validate_reserve_config(config: ReserveConfig) -> ProgramResult {
    if config.optimal_utilization_rate > 100 {
        msg!("Optimal utilization rate must be in range [0, 100]");
        return Err(LendingError::InvalidConfig.into());
    }
    if config.unhealthy_utilization_rate < config.optimal_utilization_rate
        || config.unhealthy_utilization_rate > 100
    {
        msg!("Unhealthy utilization rate must be in range [optimal_utilization_rate, 100]");
        return Err(LendingError::InvalidConfig.into());
    }
    if config.loan_to_value_ratio >= 100 {
        msg!("Loan to value ratio must be in range [0, 100)");
        return Err(LendingError::InvalidConfig.into());
    }
    if config.liquidation_bonus > 100 {
        msg!("Liquidation bonus must be in range [0, 100]");
        return Err(LendingError::InvalidConfig.into());
    }
    if config.liquidation_threshold < config.loan_to_value_ratio
        || config.liquidation_threshold > 100
    {
        msg!("Liquidation threshold must be in range [LTV, 100]");
        return Err(LendingError::InvalidConfig.into());
    }
    if config.optimal_borrow_rate < config.min_borrow_rate {
        msg!("Optimal borrow rate must be >= min borrow rate");
        return Err(LendingError::InvalidConfig.into());
    }
    if config.optimal_borrow_rate > config.max_borrow_rate {
        msg!("Optimal borrow rate must be <= max borrow rate");
        return Err(LendingError::InvalidConfig.into());
    }
    if config.super_max_borrow_rate < config.max_borrow_rate as u64 {
        msg!("Super max borrow rate must be >= max borrow rate");
        return Err(LendingError::InvalidConfig.into());
    }
    if config.fees.borrow_fee_wad >= WAD {
        msg!("Borrow fee must be in range [0, 1_000_000_000_000_000_000)");
        return Err(LendingError::InvalidConfig.into());
    }
    if config.fees.host_fee_percentage > 100 {
        msg!("Host fee percentage must be in range [0, 100]");
        return Err(LendingError::InvalidConfig.into());
    }
    if config.protocol_liquidation_fee > 100 {
        msg!("Protocol liquidation fee must be in range [0, 100]");
        return Err(LendingError::InvalidConfig.into());
    }
    if config.protocol_take_rate > 100 {
        msg!("Protocol take rate must be in range [0, 100]");
        return Err(LendingError::InvalidConfig.into());
    }

    if config.reserve_type == ReserveType::Isolated
        && !(config.loan_to_value_ratio == 0 && config.liquidation_threshold == 0)
    {
        msg!("open/close LTV must be 0 for isolated reserves");
        return Err(LendingError::InvalidConfig.into());
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, FromPrimitive)]
/// Asset Type of the reserve
pub enum ReserveType {
    #[default]
    /// this asset can be used as collateral
    Regular = 0,
    /// this asset cannot be used as collateral and can only be borrowed in isolation
    Isolated = 1,
}

/// Additional fee information on a reserve
///
/// These exist separately from interest accrual fees, and are specifically for the program owner
/// and frontend host. The fees are paid out as a percentage of liquidity token amounts during
/// repayments and liquidations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReserveFees {
    /// Fee assessed on `BorrowObligationLiquidity`, expressed as a Wad.
    /// Must be between 0 and 10^18, such that 10^18 = 1.  A few examples for
    /// clarity:
    /// 1% = 10_000_000_000_000_000
    /// 0.01% (1 basis point) = 100_000_000_000_000
    /// 0.00001% (Aave borrow fee) = 100_000_000_000
    pub borrow_fee_wad: u64,
    /// Fee for flash loan, expressed as a Wad.
    /// 0.3% (Aave flash loan fee) = 3_000_000_000_000_000
    pub flash_loan_fee_wad: u64,
    /// Amount of fee going to host account, if provided in liquidate and repay
    pub host_fee_percentage: u8,
}

impl ReserveFees {
    /// Calculate the owner and host fees on borrow
    pub fn calculate_borrow_fees(
        &self,
        borrow_amount: Decimal,
        fee_calculation: FeeCalculation,
    ) -> Result<(u64, u64), ProgramError> {
        self.calculate_fees(borrow_amount, self.borrow_fee_wad, fee_calculation)
    }

    /// Calculate the owner and host fees on flash loan
    pub fn calculate_flash_loan_fees(
        &self,
        flash_loan_amount: Decimal,
    ) -> Result<(u64, u64), ProgramError> {
        let (total_fees, host_fee) = self.calculate_fees(
            flash_loan_amount,
            self.flash_loan_fee_wad,
            FeeCalculation::Exclusive,
        )?;

        let origination_fee = total_fees
            .checked_sub(host_fee)
            .ok_or(LendingError::MathOverflow)?;
        Ok((origination_fee, host_fee))
    }

    fn calculate_fees(
        &self,
        amount: Decimal,
        fee_wad: u64,
        fee_calculation: FeeCalculation,
    ) -> Result<(u64, u64), ProgramError> {
        let borrow_fee_rate = Rate::from_scaled_val(fee_wad);
        let host_fee_rate = Rate::from_percent(self.host_fee_percentage);
        if borrow_fee_rate > Rate::zero() && amount > Decimal::zero() {
            let need_to_assess_host_fee = host_fee_rate > Rate::zero();
            let minimum_fee = if need_to_assess_host_fee {
                2u64 // 1 token to owner, 1 to host
            } else {
                1u64 // 1 token to owner, nothing else
            };

            let borrow_fee_amount = match fee_calculation {
                // Calculate fee to be added to borrow: fee = amount * rate
                FeeCalculation::Exclusive => amount.try_mul(borrow_fee_rate)?,
                // Calculate fee to be subtracted from borrow: fee = amount * (rate / (rate + 1))
                FeeCalculation::Inclusive => {
                    let borrow_fee_rate =
                        borrow_fee_rate.try_div(borrow_fee_rate.try_add(Rate::one())?)?;
                    amount.try_mul(borrow_fee_rate)?
                }
            };

            let borrow_fee_decimal = borrow_fee_amount.max(minimum_fee.into());
            if borrow_fee_decimal >= amount {
                msg!("Borrow amount is too small to receive liquidity after fees");
                return Err(LendingError::BorrowTooSmall.into());
            }

            let borrow_fee = borrow_fee_decimal.try_round_u64()?;
            let host_fee = if need_to_assess_host_fee {
                borrow_fee_decimal
                    .try_mul(host_fee_rate)?
                    .try_round_u64()?
                    .max(1u64)
            } else {
                0
            };

            Ok((borrow_fee, host_fee))
        } else {
            Ok((0, 0))
        }
    }
}

/// Calculate fees exlusive or inclusive of an amount
pub enum FeeCalculation {
    /// Fee added to amount: fee = rate * amount
    Exclusive,
    /// Fee included in amount: fee = (rate / (1 + rate)) * amount
    Inclusive,
}

impl Sealed for Reserve {}
impl IsInitialized for Reserve {
    fn is_initialized(&self) -> bool {
        self.version != UNINITIALIZED_VERSION
    }
}

const RESERVE_LEN: usize = 619; // 1 + 8 + 1 + 32 + 32 + 1 + 32 + 32 + 32 + 8 + 16 + 16 + 16 + 32 + 8 + 32 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 8 + 8 + 1 + 8 + 8 + 32 + 1 + 1 + 16 + 230
impl Pack for Reserve {
    const LEN: usize = RESERVE_LEN;

    // @TODO: break this up by reserve / liquidity / collateral / config https://git.io/JOCca
    fn pack_into_slice(&self, output: &mut [u8]) {
        let output = array_mut_ref![output, 0, RESERVE_LEN];
        #[allow(clippy::ptr_offset_with_cast)]
        let (
            version,
            last_update_slot,
            last_update_stale,
            lending_market,
            liquidity_mint_pubkey,
            liquidity_mint_decimals,
            liquidity_supply_pubkey,
            liquidity_pyth_oracle_pubkey,
            liquidity_switchboard_oracle_pubkey,
            liquidity_available_amount,
            liquidity_borrowed_amount_wads,
            liquidity_cumulative_borrow_rate_wads,
            liquidity_market_price,
            collateral_mint_pubkey,
            collateral_mint_total_supply,
            collateral_supply_pubkey,
            config_optimal_utilization_rate,
            config_loan_to_value_ratio,
            config_liquidation_bonus,
            config_liquidation_threshold,
            config_min_borrow_rate,
            config_optimal_borrow_rate,
            config_max_borrow_rate,
            config_fees_borrow_fee_wad,
            config_fees_flash_loan_fee_wad,
            config_fees_host_fee_percentage,
            config_deposit_limit,
            config_borrow_limit,
            config_fee_receiver,
            config_protocol_liquidation_fee,
            config_protocol_take_rate,
            liquidity_accumulated_protocol_fees_wads,
            rate_limiter,
            config_added_borrow_weight_bps,
            liquidity_smoothed_market_price,
            config_asset_type,
            config_unhealthy_utilization_rate,
            config_super_max_borrow_rate,
            _padding,
        ) = mut_array_refs![
            output,
            1,
            8,
            1,
            PUBKEY_BYTES,
            PUBKEY_BYTES,
            1,
            PUBKEY_BYTES,
            PUBKEY_BYTES,
            PUBKEY_BYTES,
            8,
            16,
            16,
            16,
            PUBKEY_BYTES,
            8,
            PUBKEY_BYTES,
            1,
            1,
            1,
            1,
            1,
            1,
            1,
            8,
            8,
            1,
            8,
            8,
            PUBKEY_BYTES,
            1,
            1,
            16,
            RATE_LIMITER_LEN,
            8,
            16,
            1,
            1,
            8,
            140
        ];

        // reserve
        *version = self.version.to_le_bytes();
        *last_update_slot = self.last_update.slot.to_le_bytes();
        pack_bool(self.last_update.stale, last_update_stale);
        lending_market.copy_from_slice(self.lending_market.as_ref());

        // liquidity
        liquidity_mint_pubkey.copy_from_slice(self.liquidity.mint_pubkey.as_ref());
        *liquidity_mint_decimals = self.liquidity.mint_decimals.to_le_bytes();
        liquidity_supply_pubkey.copy_from_slice(self.liquidity.supply_pubkey.as_ref());
        liquidity_pyth_oracle_pubkey.copy_from_slice(self.liquidity.pyth_oracle_pubkey.as_ref());
        liquidity_switchboard_oracle_pubkey
            .copy_from_slice(self.liquidity.switchboard_oracle_pubkey.as_ref());
        *liquidity_available_amount = self.liquidity.available_amount.to_le_bytes();
        pack_decimal(
            self.liquidity.borrowed_amount_wads,
            liquidity_borrowed_amount_wads,
        );
        pack_decimal(
            self.liquidity.cumulative_borrow_rate_wads,
            liquidity_cumulative_borrow_rate_wads,
        );
        pack_decimal(
            self.liquidity.accumulated_protocol_fees_wads,
            liquidity_accumulated_protocol_fees_wads,
        );
        pack_decimal(self.liquidity.market_price, liquidity_market_price);
        pack_decimal(
            self.liquidity.smoothed_market_price,
            liquidity_smoothed_market_price,
        );

        // collateral
        collateral_mint_pubkey.copy_from_slice(self.collateral.mint_pubkey.as_ref());
        *collateral_mint_total_supply = self.collateral.mint_total_supply.to_le_bytes();
        collateral_supply_pubkey.copy_from_slice(self.collateral.supply_pubkey.as_ref());

        // config
        *config_optimal_utilization_rate = self.config.optimal_utilization_rate.to_le_bytes();
        *config_unhealthy_utilization_rate = self.config.unhealthy_utilization_rate.to_le_bytes();
        *config_loan_to_value_ratio = self.config.loan_to_value_ratio.to_le_bytes();
        *config_liquidation_bonus = self.config.liquidation_bonus.to_le_bytes();
        *config_liquidation_threshold = self.config.liquidation_threshold.to_le_bytes();
        *config_min_borrow_rate = self.config.min_borrow_rate.to_le_bytes();
        *config_optimal_borrow_rate = self.config.optimal_borrow_rate.to_le_bytes();
        *config_max_borrow_rate = self.config.max_borrow_rate.to_le_bytes();
        *config_super_max_borrow_rate = self.config.super_max_borrow_rate.to_le_bytes();
        *config_fees_borrow_fee_wad = self.config.fees.borrow_fee_wad.to_le_bytes();
        *config_fees_flash_loan_fee_wad = self.config.fees.flash_loan_fee_wad.to_le_bytes();
        *config_fees_host_fee_percentage = self.config.fees.host_fee_percentage.to_le_bytes();
        *config_deposit_limit = self.config.deposit_limit.to_le_bytes();
        *config_borrow_limit = self.config.borrow_limit.to_le_bytes();
        config_fee_receiver.copy_from_slice(self.config.fee_receiver.as_ref());
        *config_protocol_liquidation_fee = self.config.protocol_liquidation_fee.to_le_bytes();
        *config_protocol_take_rate = self.config.protocol_take_rate.to_le_bytes();
        *config_asset_type = (self.config.reserve_type as u8).to_le_bytes();

        self.rate_limiter.pack_into_slice(rate_limiter);

        *config_added_borrow_weight_bps = self.config.added_borrow_weight_bps.to_le_bytes();
    }

    /// Unpacks a byte buffer into a [ReserveInfo](struct.ReserveInfo.html).
    fn unpack_from_slice(input: &[u8]) -> Result<Self, ProgramError> {
        let input = array_ref![input, 0, RESERVE_LEN];
        #[allow(clippy::ptr_offset_with_cast)]
        let (
            version,
            last_update_slot,
            last_update_stale,
            lending_market,
            liquidity_mint_pubkey,
            liquidity_mint_decimals,
            liquidity_supply_pubkey,
            liquidity_pyth_oracle_pubkey,
            liquidity_switchboard_oracle_pubkey,
            liquidity_available_amount,
            liquidity_borrowed_amount_wads,
            liquidity_cumulative_borrow_rate_wads,
            liquidity_market_price,
            collateral_mint_pubkey,
            collateral_mint_total_supply,
            collateral_supply_pubkey,
            config_optimal_utilization_rate,
            config_loan_to_value_ratio,
            config_liquidation_bonus,
            config_liquidation_threshold,
            config_min_borrow_rate,
            config_optimal_borrow_rate,
            config_max_borrow_rate,
            config_fees_borrow_fee_wad,
            config_fees_flash_loan_fee_wad,
            config_fees_host_fee_percentage,
            config_deposit_limit,
            config_borrow_limit,
            config_fee_receiver,
            config_protocol_liquidation_fee,
            config_protocol_take_rate,
            liquidity_accumulated_protocol_fees_wads,
            rate_limiter,
            config_added_borrow_weight_bps,
            liquidity_smoothed_market_price,
            config_asset_type,
            config_unhealthy_utilization_rate,
            config_super_max_borrow_rate,
            _padding,
        ) = array_refs![
            input,
            1,
            8,
            1,
            PUBKEY_BYTES,
            PUBKEY_BYTES,
            1,
            PUBKEY_BYTES,
            PUBKEY_BYTES,
            PUBKEY_BYTES,
            8,
            16,
            16,
            16,
            PUBKEY_BYTES,
            8,
            PUBKEY_BYTES,
            1,
            1,
            1,
            1,
            1,
            1,
            1,
            8,
            8,
            1,
            8,
            8,
            PUBKEY_BYTES,
            1,
            1,
            16,
            RATE_LIMITER_LEN,
            8,
            16,
            1,
            1,
            8,
            140
        ];

        let version = u8::from_le_bytes(*version);
        if version > PROGRAM_VERSION {
            msg!("Reserve version does not match lending program version");
            return Err(ProgramError::InvalidAccountData);
        }

        let optimal_utilization_rate = u8::from_le_bytes(*config_optimal_utilization_rate);
        let max_borrow_rate = u8::from_le_bytes(*config_max_borrow_rate);

        Ok(Self {
            version,
            last_update: LastUpdate {
                slot: u64::from_le_bytes(*last_update_slot),
                stale: unpack_bool(last_update_stale)?,
            },
            lending_market: Pubkey::new_from_array(*lending_market),
            liquidity: ReserveLiquidity {
                mint_pubkey: Pubkey::new_from_array(*liquidity_mint_pubkey),
                mint_decimals: u8::from_le_bytes(*liquidity_mint_decimals),
                supply_pubkey: Pubkey::new_from_array(*liquidity_supply_pubkey),
                pyth_oracle_pubkey: Pubkey::new_from_array(*liquidity_pyth_oracle_pubkey),
                switchboard_oracle_pubkey: Pubkey::new_from_array(
                    *liquidity_switchboard_oracle_pubkey,
                ),
                available_amount: u64::from_le_bytes(*liquidity_available_amount),
                borrowed_amount_wads: unpack_decimal(liquidity_borrowed_amount_wads),
                cumulative_borrow_rate_wads: unpack_decimal(liquidity_cumulative_borrow_rate_wads),
                accumulated_protocol_fees_wads: unpack_decimal(
                    liquidity_accumulated_protocol_fees_wads,
                ),
                market_price: unpack_decimal(liquidity_market_price),
                smoothed_market_price: unpack_decimal(liquidity_smoothed_market_price),
            },
            collateral: ReserveCollateral {
                mint_pubkey: Pubkey::new_from_array(*collateral_mint_pubkey),
                mint_total_supply: u64::from_le_bytes(*collateral_mint_total_supply),
                supply_pubkey: Pubkey::new_from_array(*collateral_supply_pubkey),
            },
            config: ReserveConfig {
                optimal_utilization_rate,
                unhealthy_utilization_rate: max(
                    optimal_utilization_rate,
                    u8::from_le_bytes(*config_unhealthy_utilization_rate),
                ),
                loan_to_value_ratio: u8::from_le_bytes(*config_loan_to_value_ratio),
                liquidation_bonus: u8::from_le_bytes(*config_liquidation_bonus),
                liquidation_threshold: u8::from_le_bytes(*config_liquidation_threshold),
                min_borrow_rate: u8::from_le_bytes(*config_min_borrow_rate),
                optimal_borrow_rate: u8::from_le_bytes(*config_optimal_borrow_rate),
                max_borrow_rate,
                super_max_borrow_rate: max(
                    max_borrow_rate as u64,
                    u64::from_le_bytes(*config_super_max_borrow_rate),
                ),
                fees: ReserveFees {
                    borrow_fee_wad: u64::from_le_bytes(*config_fees_borrow_fee_wad),
                    flash_loan_fee_wad: u64::from_le_bytes(*config_fees_flash_loan_fee_wad),
                    host_fee_percentage: u8::from_le_bytes(*config_fees_host_fee_percentage),
                },
                deposit_limit: u64::from_le_bytes(*config_deposit_limit),
                borrow_limit: u64::from_le_bytes(*config_borrow_limit),
                fee_receiver: Pubkey::new_from_array(*config_fee_receiver),
                protocol_liquidation_fee: u8::from_le_bytes(*config_protocol_liquidation_fee),
                protocol_take_rate: u8::from_le_bytes(*config_protocol_take_rate),
                added_borrow_weight_bps: u64::from_le_bytes(*config_added_borrow_weight_bps),
                reserve_type: ReserveType::from_u8(config_asset_type[0]).unwrap(),
            },
            rate_limiter: RateLimiter::unpack_from_slice(rate_limiter)?,
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::math::{PERCENT_SCALER, WAD};
    use proptest::prelude::*;
    use rand::Rng;
    use solana_program::native_token::LAMPORTS_PER_SOL;

    use std::default::Default;

    fn rand_decimal() -> Decimal {
        Decimal::from_scaled_val(rand::thread_rng().gen())
    }

    #[test]
    fn pack_and_unpack_reserve() {
        let mut rng = rand::thread_rng();
        for _ in 0..100 {
            let optimal_utilization_rate = rng.gen();
            let reserve = Reserve {
                version: PROGRAM_VERSION,
                last_update: LastUpdate {
                    slot: rng.gen(),
                    stale: rng.gen(),
                },
                lending_market: Pubkey::new_unique(),
                liquidity: ReserveLiquidity {
                    mint_pubkey: Pubkey::new_unique(),
                    mint_decimals: rng.gen(),
                    supply_pubkey: Pubkey::new_unique(),
                    pyth_oracle_pubkey: Pubkey::new_unique(),
                    switchboard_oracle_pubkey: Pubkey::new_unique(),
                    available_amount: rng.gen(),
                    borrowed_amount_wads: rand_decimal(),
                    cumulative_borrow_rate_wads: rand_decimal(),
                    accumulated_protocol_fees_wads: rand_decimal(),
                    market_price: rand_decimal(),
                    smoothed_market_price: rand_decimal(),
                },
                collateral: ReserveCollateral {
                    mint_pubkey: Pubkey::new_unique(),
                    mint_total_supply: rng.gen(),
                    supply_pubkey: Pubkey::new_unique(),
                },
                config: ReserveConfig {
                    optimal_utilization_rate,
                    unhealthy_utilization_rate: max(optimal_utilization_rate, rng.gen()),
                    loan_to_value_ratio: rng.gen(),
                    liquidation_bonus: rng.gen(),
                    liquidation_threshold: rng.gen(),
                    min_borrow_rate: rng.gen(),
                    optimal_borrow_rate: rng.gen(),
                    max_borrow_rate: rng.gen(),
                    super_max_borrow_rate: max(u8::MAX as u64, rng.gen()),
                    fees: ReserveFees {
                        borrow_fee_wad: rng.gen(),
                        flash_loan_fee_wad: rng.gen(),
                        host_fee_percentage: rng.gen(),
                    },
                    deposit_limit: rng.gen(),
                    borrow_limit: rng.gen(),
                    fee_receiver: Pubkey::new_unique(),
                    protocol_liquidation_fee: rng.gen(),
                    protocol_take_rate: rng.gen(),
                    added_borrow_weight_bps: rng.gen(),
                    reserve_type: ReserveType::from_u8(rng.gen::<u8>() % 2).unwrap(),
                },
                rate_limiter: rand_rate_limiter(),
            };

            let mut packed = [0u8; Reserve::LEN];
            Reserve::pack(reserve.clone(), &mut packed).unwrap();
            let unpacked = Reserve::unpack(&packed).unwrap();
            assert_eq!(reserve, unpacked);
        }
    }

    const MAX_LIQUIDITY: u64 = u64::MAX / 5;

    fn utilizations() -> impl Strategy<Value = (u8, u8)> {
        // Generate a sorted vector of u8 values with length 4.
        proptest::collection::vec(0..=100u8, 2).prop_map(|mut vec| {
            // Sort the vector.
            vec.sort_unstable();
            // Convert the first four elements of the vector into a tuple.
            (vec[0], vec[1])
        })
    }

    fn borrow_rates() -> impl Strategy<Value = (u8, u8, u8, u64)> {
        prop::collection::vec(0u8..=255u8, 3)
            // Generate a u64 element in the range [0, u64::MAX].
            .prop_flat_map(|vec| {
                let max_u8 = *vec.iter().max().unwrap() as u64;
                (Just(vec), max_u8..=u64::MAX as u64)
            })
            .prop_map(|(mut vec, d)| {
                // Sort the vector to ensure the first three elements are in non-decreasing order.
                vec.sort();
                // Convert the sorted vector and the fourth element into a tuple.
                (vec[0], vec[1], vec[2], d)
            })
    }

    // Creates rates (threshold, ltv) where 2 <= threshold <= 100 and threshold <= ltv <= 1,000%
    prop_compose! {
        fn unhealthy_rates()(threshold in 2..=100u8)(
            ltv_rate in threshold as u64..=1000u64,
            threshold in Just(threshold),
        ) -> (Decimal, u8) {
            (Decimal::from_scaled_val(ltv_rate as u128 * PERCENT_SCALER as u128), threshold)
        }
    }

    // Creates a range of reasonable token conversion rates
    prop_compose! {
        fn token_conversion_rate()(
            conversion_rate in 1..=u16::MAX,
            invert_conversion_rate: bool,
        ) -> Decimal {
            let conversion_rate = Decimal::from(conversion_rate as u64);
            if invert_conversion_rate {
                Decimal::one().try_div(conversion_rate).unwrap()
            } else {
                conversion_rate
            }
        }
    }

    // Creates a range of reasonable collateral exchange rates
    prop_compose! {
        fn collateral_exchange_rate_range()(percent in 1..=500u64) -> CollateralExchangeRate {
            CollateralExchangeRate(Rate::from_scaled_val(percent * PERCENT_SCALER))
        }
    }

    proptest! {
        #[test]
        fn current_borrow_rate(
            total_liquidity in 0..=MAX_LIQUIDITY,
            borrowed_percent in 0..=WAD,
            (optimal_utilization_rate, unhealthy_utilization_rate) in utilizations(),
            (min_borrow_rate, optimal_borrow_rate, max_borrow_rate, super_max_borrow_rate) in borrow_rates(),
        ) {
            let borrowed_amount_wads = Decimal::from(total_liquidity)
                .try_mul(Rate::from_scaled_val(borrowed_percent))?;
            let reserve = Reserve {
                liquidity: ReserveLiquidity {
                    borrowed_amount_wads,
                    available_amount: total_liquidity - borrowed_amount_wads.try_round_u64()?,
                    ..ReserveLiquidity::default()
                },
                config: ReserveConfig {
                    optimal_utilization_rate,
                    unhealthy_utilization_rate,
                    min_borrow_rate,
                    optimal_borrow_rate,
                    max_borrow_rate,
                    super_max_borrow_rate: super_max_borrow_rate as u64,
                    ..ReserveConfig::default()
                },
                ..Reserve::default()
            };

            let current_borrow_rate = reserve.current_borrow_rate()?;
            assert!(current_borrow_rate >= Rate::from_percent(min_borrow_rate));
            assert!(current_borrow_rate <= Rate::from_percent_u64(super_max_borrow_rate),
                "current_borrow_rate: {}, super_max_borrow_rate: {}",
                current_borrow_rate, super_max_borrow_rate);

            let current_rate = reserve.liquidity.utilization_rate()?;
            if current_rate <= Rate::from_percent(optimal_utilization_rate) {
                assert!(current_borrow_rate <= Rate::from_percent(optimal_borrow_rate));
            } else if current_rate <= Rate::from_percent(unhealthy_utilization_rate) {
                assert!(current_borrow_rate >= Rate::from_percent(optimal_borrow_rate));
                assert!(current_borrow_rate <= Rate::from_percent(max_borrow_rate));
            } else {
                assert!(current_borrow_rate >= Rate::from_percent(max_borrow_rate));
                assert!(current_borrow_rate <= Rate::from_percent_u64(super_max_borrow_rate));
            }
        }

        #[test]
        fn current_utilization_rate(
            total_liquidity in 0..=MAX_LIQUIDITY,
            borrowed_percent in 0..=WAD,
        ) {
            let borrowed_amount_wads = Decimal::from(total_liquidity).try_mul(Rate::from_scaled_val(borrowed_percent))?;
            let liquidity = ReserveLiquidity {
                borrowed_amount_wads,
                available_amount: total_liquidity - borrowed_amount_wads.try_round_u64()?,
                ..ReserveLiquidity::default()
            };

            let current_rate = liquidity.utilization_rate()?;
            assert!(current_rate <= Rate::one());
        }

        #[test]
        fn collateral_exchange_rate(
            total_liquidity in 0..=MAX_LIQUIDITY,
            borrowed_percent in 0..=WAD,
            collateral_multiplier in 0..=(5*WAD),
            borrow_rate in 0..=u8::MAX,
        ) {
            let borrowed_liquidity_wads = Decimal::from(total_liquidity).try_mul(Rate::from_scaled_val(borrowed_percent))?;
            let available_liquidity = total_liquidity - borrowed_liquidity_wads.try_round_u64()?;
            let mint_total_supply = Decimal::from(total_liquidity).try_mul(Rate::from_scaled_val(collateral_multiplier))?.try_round_u64()?;

            let mut reserve = Reserve {
                collateral: ReserveCollateral {
                    mint_total_supply,
                    ..ReserveCollateral::default()
                },
                liquidity: ReserveLiquidity {
                    borrowed_amount_wads: borrowed_liquidity_wads,
                    available_amount: available_liquidity,
                    ..ReserveLiquidity::default()
                },
                config: ReserveConfig {
                    min_borrow_rate: borrow_rate,
                    optimal_borrow_rate: borrow_rate,
                    optimal_utilization_rate: 100,
                    ..ReserveConfig::default()
                },
                ..Reserve::default()
            };

            let exchange_rate = reserve.collateral_exchange_rate()?;
            assert!(exchange_rate.0.to_scaled_val() <= 5u128 * WAD as u128);

            // After interest accrual, total liquidity increases and collateral are worth more
            reserve.accrue_interest(1)?;

            let new_exchange_rate = reserve.collateral_exchange_rate()?;
            if borrow_rate > 0 && total_liquidity > 0 && borrowed_percent > 0 {
                assert!(new_exchange_rate.0 < exchange_rate.0);
            } else {
                assert_eq!(new_exchange_rate.0, exchange_rate.0);
            }
        }

        #[test]
        fn compound_interest(
            slots_elapsed in 0..=SLOTS_PER_YEAR,
            borrow_rate in 0..=u8::MAX,
            take_rate in 0..=100u8,
        ) {
            let mut reserve = Reserve::default();
            let borrow_rate = Rate::from_percent(borrow_rate);
            let take_rate = Rate::from_percent(take_rate);

            // Simulate running for max 1000 years, assuming that interest is
            // compounded at least once a year
            for _ in 0..1000 {
                reserve.liquidity.compound_interest(borrow_rate, slots_elapsed, take_rate)?;
                reserve.liquidity.cumulative_borrow_rate_wads.to_scaled_val()?;
                reserve.liquidity.accumulated_protocol_fees_wads.to_scaled_val()?;
            }
        }

        #[test]
        fn reserve_accrue_interest(
            slots_elapsed in 0..=SLOTS_PER_YEAR,
            borrowed_liquidity in 0..=u64::MAX,
            borrow_rate in 0..=u8::MAX,
        ) {
            let borrowed_amount_wads = Decimal::from(borrowed_liquidity);
            let mut reserve = Reserve {
                liquidity: ReserveLiquidity {
                    borrowed_amount_wads,
                    ..ReserveLiquidity::default()
                },
                config: ReserveConfig {
                    min_borrow_rate: borrow_rate,
                    optimal_borrow_rate: borrow_rate,
                    max_borrow_rate: borrow_rate,
                    super_max_borrow_rate: borrow_rate as u64,
                    ..ReserveConfig::default()
                },
                ..Reserve::default()
            };

            reserve.accrue_interest(slots_elapsed)?;

            if borrow_rate > 0 && slots_elapsed > 0 {
                assert!(reserve.liquidity.borrowed_amount_wads > borrowed_amount_wads);
            } else {
                assert!(reserve.liquidity.borrowed_amount_wads == borrowed_amount_wads);
            }
        }

        #[test]
        fn borrow_fee_calculation(
            borrow_fee_wad in 0..WAD, // at WAD, fee == borrow amount, which fails
            flash_loan_fee_wad in 0..WAD, // at WAD, fee == borrow amount, which fails
            host_fee_percentage in 0..=100u8,
            borrow_amount in 3..=u64::MAX, // start at 3 to ensure calculation success
                                           // 0, 1, and 2 are covered in the minimum tests
                                           // @FIXME: ^ no longer true
        ) {
            let fees = ReserveFees {
                borrow_fee_wad,
                flash_loan_fee_wad,
                host_fee_percentage,
            };
            let (total_fee, host_fee) = fees.calculate_borrow_fees(Decimal::from(borrow_amount), FeeCalculation::Exclusive)?;

            // The total fee can't be greater than the amount borrowed, as long
            // as amount borrowed is greater than 2.
            // At a borrow amount of 2, we can get a total fee of 2 if a host
            // fee is also specified.
            assert!(total_fee <= borrow_amount);

            // the host fee can't be greater than the total fee
            assert!(host_fee <= total_fee);

            // for all fee rates greater than 0, we must have some fee
            if borrow_fee_wad > 0 {
                assert!(total_fee > 0);
            }

            if host_fee_percentage == 100 {
                // if the host fee percentage is maxed at 100%, it should get all the fee
                assert_eq!(host_fee, total_fee);
            }

            // if there's a host fee and some borrow fee, host fee must be greater than 0
            if host_fee_percentage > 0 && borrow_fee_wad > 0 {
                assert!(host_fee > 0);
            } else {
                assert_eq!(host_fee, 0);
            }
        }

        #[test]
        fn flash_loan_fee_calculation(
            borrow_fee_wad in 0..WAD, // at WAD, fee == borrow amount, which fails
            flash_loan_fee_wad in 0..WAD, // at WAD, fee == borrow amount, which fails
            host_fee_percentage in 0..=100u8,
            borrow_amount in 3..=u64::MAX, // start at 3 to ensure calculation success
                                           // 0, 1, and 2 are covered in the minimum tests
                                           // @FIXME: ^ no longer true
        ) {
            let fees = ReserveFees {
                borrow_fee_wad,
                flash_loan_fee_wad,
                host_fee_percentage,
            };
            let (origination_fee, host_fee) = fees.calculate_flash_loan_fees(Decimal::from(borrow_amount))?;

            // The total fee can't be greater than the amount borrowed, as long
            // as amount borrowed is greater than 2.
            // At a borrow amount of 2, we can get a total fee of 2 if a host
            // fee is also specified.
            assert!(origination_fee + host_fee <= borrow_amount);

            // for all fee rates greater than 0, we must have some fee
            if borrow_fee_wad > 0 {
                assert!(origination_fee + host_fee > 0);
            }

            if host_fee_percentage == 100 {
                // if the host fee percentage is maxed at 100%, it should get all the fee
                assert_eq!(origination_fee, 0);
            }

            // if there's a host fee and some borrow fee, host fee must be greater than 0
            if host_fee_percentage > 0 && borrow_fee_wad > 0 {
                assert!(host_fee > 0);
            } else {
                assert_eq!(host_fee, 0);
            }
        }
    }

    #[test]
    fn borrow_fee_calculation_min_host() {
        let fees = ReserveFees {
            borrow_fee_wad: 10_000_000_000_000_000, // 1%
            flash_loan_fee_wad: 0,
            host_fee_percentage: 20,
        };

        // only 2 tokens borrowed, get error
        let err = fees
            .calculate_borrow_fees(Decimal::from(2u64), FeeCalculation::Exclusive)
            .unwrap_err();
        assert_eq!(err, LendingError::BorrowTooSmall.into()); // minimum of 3 tokens

        // only 1 token borrowed, get error
        let err = fees
            .calculate_borrow_fees(Decimal::one(), FeeCalculation::Exclusive)
            .unwrap_err();
        assert_eq!(err, LendingError::BorrowTooSmall.into());

        // 0 amount borrowed, 0 fee
        let (total_fee, host_fee) = fees
            .calculate_borrow_fees(Decimal::zero(), FeeCalculation::Exclusive)
            .unwrap();
        assert_eq!(total_fee, 0);
        assert_eq!(host_fee, 0);
    }

    #[test]
    fn borrow_fee_calculation_min_no_host() {
        let fees = ReserveFees {
            borrow_fee_wad: 10_000_000_000_000_000, // 1%
            flash_loan_fee_wad: 0,
            host_fee_percentage: 0,
        };

        // only 2 tokens borrowed, ok
        let (total_fee, host_fee) = fees
            .calculate_borrow_fees(Decimal::from(2u64), FeeCalculation::Exclusive)
            .unwrap();
        assert_eq!(total_fee, 1);
        assert_eq!(host_fee, 0);

        // only 1 token borrowed, get error
        let err = fees
            .calculate_borrow_fees(Decimal::one(), FeeCalculation::Exclusive)
            .unwrap_err();
        assert_eq!(err, LendingError::BorrowTooSmall.into()); // minimum of 2 tokens

        // 0 amount borrowed, 0 fee
        let (total_fee, host_fee) = fees
            .calculate_borrow_fees(Decimal::zero(), FeeCalculation::Exclusive)
            .unwrap();
        assert_eq!(total_fee, 0);
        assert_eq!(host_fee, 0);
    }

    #[test]
    fn borrow_fee_calculation_host() {
        let fees = ReserveFees {
            borrow_fee_wad: 10_000_000_000_000_000, // 1%
            flash_loan_fee_wad: 0,
            host_fee_percentage: 20,
        };

        let (total_fee, host_fee) = fees
            .calculate_borrow_fees(Decimal::from(1000u64), FeeCalculation::Exclusive)
            .unwrap();

        assert_eq!(total_fee, 10); // 1% of 1000
        assert_eq!(host_fee, 2); // 20% of 10
    }

    #[test]
    fn borrow_fee_calculation_no_host() {
        let fees = ReserveFees {
            borrow_fee_wad: 10_000_000_000_000_000, // 1%
            flash_loan_fee_wad: 0,
            host_fee_percentage: 0,
        };

        let (total_fee, host_fee) = fees
            .calculate_borrow_fees(Decimal::from(1000u64), FeeCalculation::Exclusive)
            .unwrap();

        assert_eq!(total_fee, 10); // 1% of 1000
        assert_eq!(host_fee, 0); // 0 host fee
    }

    #[test]
    fn market_value() {
        let reserve = Reserve {
            liquidity: ReserveLiquidity {
                mint_decimals: 9,
                market_price: Decimal::from(25u64),
                smoothed_market_price: Decimal::from(50u64),
                ..ReserveLiquidity::default()
            },
            ..Reserve::default()
        };

        assert_eq!(
            reserve.market_value(Decimal::from(1u64)).unwrap(),
            Decimal::from(25u64).try_div(1e9 as u64).unwrap()
        );
        assert_eq!(
            reserve
                .market_value(Decimal::from(10 * LAMPORTS_PER_SOL))
                .unwrap(),
            Decimal::from(250u64)
        );

        assert_eq!(
            reserve
                .market_value_lower_bound(Decimal::from(10 * LAMPORTS_PER_SOL))
                .unwrap(),
            Decimal::from(250u64)
        );

        assert_eq!(
            reserve
                .market_value_upper_bound(Decimal::from(10 * LAMPORTS_PER_SOL))
                .unwrap(),
            Decimal::from(500u64)
        );
    }

    #[derive(Debug, Clone)]
    struct ReserveConfigTestCase {
        config: ReserveConfig,
        result: Result<(), ProgramError>,
    }

    fn reserve_config_test_cases() -> impl Strategy<Value = ReserveConfigTestCase> {
        prop_oneof![
            Just(ReserveConfigTestCase {
                config: ReserveConfig {
                    reserve_type: ReserveType::Isolated,
                    loan_to_value_ratio: 1,
                    ..ReserveConfig::default()
                },
                result: Err(LendingError::InvalidConfig.into()),
            }),
            Just(ReserveConfigTestCase {
                config: ReserveConfig {
                    reserve_type: ReserveType::Isolated,
                    liquidation_threshold: 1,
                    ..ReserveConfig::default()
                },
                result: Err(LendingError::InvalidConfig.into()),
            }),
            Just(ReserveConfigTestCase {
                config: ReserveConfig {
                    reserve_type: ReserveType::Isolated,
                    loan_to_value_ratio: 0,
                    liquidation_threshold: 0,
                    ..ReserveConfig::default()
                },
                result: Ok(()),
            })
        ]
    }

    proptest! {
        #[test]
        fn test_validate_reserve_config(test_case in reserve_config_test_cases()) {
            assert_eq!(validate_reserve_config(test_case.config), test_case.result);
        }
    }

    #[derive(Debug, Clone)]
    struct LiquidationTestCase {
        deposit_amount: u64,
        deposit_market_value: Decimal,
        borrow_amount: u64,
        borrow_market_value: Decimal,
        liquidation_result: CalculateLiquidationResult,
    }

    fn calculate_liquidation_test_cases() -> impl Strategy<Value = LiquidationTestCase> {
        let close_factor: Decimal = Rate::from_percent(LIQUIDATION_CLOSE_FACTOR)
            .try_into()
            .unwrap();
        let liquidation_bonus: Decimal = Rate::from_percent(5)
            .try_add(Rate::one())
            .unwrap()
            .try_into()
            .unwrap();

        prop_oneof![
            // collateral market value > liquidation value
            Just(LiquidationTestCase {
                deposit_amount: 1000,
                deposit_market_value: Decimal::from(100u64),
                borrow_amount: 800,
                borrow_market_value: Decimal::from(80u64),
                liquidation_result: CalculateLiquidationResult {
                    settle_amount: close_factor.try_mul(Decimal::from(800u64)).unwrap(),
                    repay_amount: close_factor
                        .try_mul(Decimal::from(800u64))
                        .unwrap()
                        .try_ceil_u64()
                        .unwrap(),
                    withdraw_amount: close_factor
                        .try_mul(liquidation_bonus)
                        .unwrap()
                        .try_mul(Decimal::from(800u64))
                        .unwrap()
                        .try_floor_u64()
                        .unwrap(),
                },
            }),
            // collateral market value == liquidation_value
            Just(LiquidationTestCase {
                borrow_amount: 8000,
                borrow_market_value: Decimal::from(8000u64),
                deposit_amount: (8000 * LIQUIDATION_CLOSE_FACTOR as u64) * 105 / 10000,
                deposit_market_value: Decimal::from(
                    (8000 * LIQUIDATION_CLOSE_FACTOR as u64) * 105 / 10000
                ),

                liquidation_result: CalculateLiquidationResult {
                    settle_amount: Decimal::from((8000 * LIQUIDATION_CLOSE_FACTOR as u64) / 100),
                    repay_amount: (8000 * LIQUIDATION_CLOSE_FACTOR as u64) / 100,
                    withdraw_amount: (8000 * LIQUIDATION_CLOSE_FACTOR as u64) * 105 / 10000,
                },
            }),
            // collateral market value < liquidation_value
            Just(LiquidationTestCase {
                borrow_amount: 8000,
                borrow_market_value: Decimal::from(8000u64),

                // half of liquidation value
                deposit_amount: (8000 * LIQUIDATION_CLOSE_FACTOR as u64) * 105 / 10000 / 2,
                deposit_market_value: Decimal::from(
                    (8000 * LIQUIDATION_CLOSE_FACTOR as u64) * 105 / 10000 / 2
                ),

                liquidation_result: CalculateLiquidationResult {
                    settle_amount: Decimal::from(
                        (8000 * LIQUIDATION_CLOSE_FACTOR as u64) / 100 / 2
                    ),
                    repay_amount: (8000 * LIQUIDATION_CLOSE_FACTOR as u64) / 100 / 2,
                    withdraw_amount: (8000 * LIQUIDATION_CLOSE_FACTOR as u64) * 105 / 10000 / 2,
                },
            }),
            // dust ObligationLiquidity where collateral market value > liquidation value
            Just(LiquidationTestCase {
                borrow_amount: 100,
                borrow_market_value: Decimal::from_percent(50),
                deposit_amount: 100,
                deposit_market_value: Decimal::from(1u64),

                liquidation_result: CalculateLiquidationResult {
                    settle_amount: Decimal::from(100u64),
                    repay_amount: 100,
                    // $0.5 * 1.05 = $0.525
                    withdraw_amount: 52,
                },
            }),
            // dust ObligationLiquidity where collateral market value == liquidation value
            Just(LiquidationTestCase {
                borrow_amount: 1,
                borrow_market_value: Decimal::from(1u64),
                deposit_amount: 1000,
                deposit_market_value: Decimal::from_percent(105),

                liquidation_result: CalculateLiquidationResult {
                    settle_amount: Decimal::from(1u64),
                    repay_amount: 1,
                    withdraw_amount: 1000,
                },
            }),
            // dust ObligationLiquidity where collateral market value < liquidation value
            Just(LiquidationTestCase {
                borrow_amount: 10,
                borrow_market_value: Decimal::one(),
                deposit_amount: 10,
                deposit_market_value: Decimal::from_bps(5250), // $0.525

                liquidation_result: CalculateLiquidationResult {
                    settle_amount: Decimal::from(5u64),
                    repay_amount: 5,
                    withdraw_amount: 10,
                },
            }),
            // dust ObligationLiquidity where collateral market value > liquidation value and the
            // withdraw amount is 1
            Just(LiquidationTestCase {
                borrow_amount: 1,
                borrow_market_value: Decimal::one(),
                deposit_amount: 1,
                deposit_market_value: Decimal::from(10u64),

                liquidation_result: CalculateLiquidationResult {
                    settle_amount: Decimal::from(1u64),
                    repay_amount: 1,
                    withdraw_amount: 1,
                },
            }),
        ]
    }

    proptest! {
        #[test]
        fn calculate_liquidation(test_case in calculate_liquidation_test_cases()) {
            let reserve = Reserve {
                config: ReserveConfig {
                    liquidation_bonus: 5,
                    ..ReserveConfig::default()
                },
                ..Reserve::default()
            };

            let obligation = Obligation {
                deposits: vec![ObligationCollateral {
                    deposit_reserve: Pubkey::new_unique(),
                    deposited_amount: test_case.deposit_amount,
                    market_value: test_case.deposit_market_value
                }],
                borrows: vec![ObligationLiquidity {
                    borrow_reserve: Pubkey::new_unique(),
                    cumulative_borrow_rate_wads: Decimal::one(),
                    borrowed_amount_wads: Decimal::from(test_case.borrow_amount),
                    market_value: test_case.borrow_market_value,
                }],
                borrowed_value: test_case.borrow_market_value,
                ..Obligation::default()
            };

            assert_eq!(
                reserve.calculate_liquidation(
                    u64::MAX, &obligation, &obligation.borrows[0], &obligation.deposits[0]).unwrap(),
                test_case.liquidation_result);
        }
    }

    #[derive(Debug, Clone)]
    struct CalculateBorrowTestCase {
        // args
        borrow_amount: u64,
        remaining_borrow_value: Decimal,
        remaining_reserve_capacity: Decimal,

        // reserve state
        market_price: Decimal,
        smoothed_market_price: Decimal,
        decimal: u8,
        added_borrow_weight_bps: u64,

        borrow_fee_wad: u64,
        host_fee: u8,

        result: Result<CalculateBorrowResult, ProgramError>,
    }

    fn calculate_borrow_test_cases() -> impl Strategy<Value = CalculateBorrowTestCase> {
        // borrow fee is 1%, host fee is 20% on all test cases
        prop_oneof![
            Just(CalculateBorrowTestCase {
                borrow_amount: LAMPORTS_PER_SOL,
                remaining_borrow_value: Decimal::from(10u64),
                remaining_reserve_capacity: Decimal::from(LAMPORTS_PER_SOL * 10),

                market_price: Decimal::from(1u64),
                smoothed_market_price: Decimal::from(1u64),
                decimal: 9,
                added_borrow_weight_bps: 0,

                borrow_fee_wad: 10_000_000_000_000_000, // 1%
                host_fee: 20,

                result: Ok(CalculateBorrowResult {
                    borrow_amount: Decimal::from(LAMPORTS_PER_SOL * 101 / 100),
                    receive_amount: LAMPORTS_PER_SOL,
                    borrow_fee: LAMPORTS_PER_SOL / 100,
                    host_fee: LAMPORTS_PER_SOL / 100 / 100 * 20
                }),
            }),
            // borrow max
            Just(CalculateBorrowTestCase {
                borrow_amount: u64::MAX,
                remaining_borrow_value: Decimal::from(10u64),
                remaining_reserve_capacity: Decimal::from(LAMPORTS_PER_SOL * 101 / 100),

                market_price: Decimal::from(1u64),
                smoothed_market_price: Decimal::from(1u64),
                decimal: 9,
                added_borrow_weight_bps: 0,

                borrow_fee_wad: 10_000_000_000_000_000, // 1%
                host_fee: 20,

                result: Ok(CalculateBorrowResult {
                    borrow_amount: Decimal::from(LAMPORTS_PER_SOL * 101 / 100),
                    receive_amount: LAMPORTS_PER_SOL,
                    borrow_fee: LAMPORTS_PER_SOL / 100,
                    host_fee: LAMPORTS_PER_SOL / 100 / 100 * 20
                }),
            }),
            // borrow weight is 2, can only borrow 0.5 sol
            Just(CalculateBorrowTestCase {
                borrow_amount: LAMPORTS_PER_SOL / 2,
                remaining_borrow_value: Decimal::from(1u64),
                remaining_reserve_capacity: Decimal::from(LAMPORTS_PER_SOL),

                market_price: Decimal::from(1u64),
                smoothed_market_price: Decimal::from(1u64),
                decimal: 9,
                added_borrow_weight_bps: 10_000,

                borrow_fee_wad: 0,
                host_fee: 0,

                result: Ok(CalculateBorrowResult {
                    borrow_amount: Decimal::from(LAMPORTS_PER_SOL / 2),
                    receive_amount: LAMPORTS_PER_SOL / 2,
                    borrow_fee: 0,
                    host_fee: 0,
                }),
            }),
            // borrow weight is 2, can only max borrow 0.5 sol
            Just(CalculateBorrowTestCase {
                borrow_amount: u64::MAX,
                remaining_borrow_value: Decimal::from(1u64),
                remaining_reserve_capacity: Decimal::from(LAMPORTS_PER_SOL),

                market_price: Decimal::from(1u64),
                smoothed_market_price: Decimal::from(1u64),
                decimal: 9,
                added_borrow_weight_bps: 10_000,

                borrow_fee_wad: 0,
                host_fee: 0,

                result: Ok(CalculateBorrowResult {
                    borrow_amount: Decimal::from(LAMPORTS_PER_SOL / 2),
                    receive_amount: LAMPORTS_PER_SOL / 2,
                    borrow_fee: 0,
                    host_fee: 0,
                }),
            }),
            // borrow max where ema price is 2x the market price
            Just(CalculateBorrowTestCase {
                borrow_amount: u64::MAX,
                remaining_borrow_value: Decimal::from(100u64),
                remaining_reserve_capacity: Decimal::from(100 * LAMPORTS_PER_SOL),

                market_price: Decimal::from(10u64),
                smoothed_market_price: Decimal::from(20u64),
                decimal: 9,
                added_borrow_weight_bps: 0,

                borrow_fee_wad: 0,
                host_fee: 0,

                result: Ok(CalculateBorrowResult {
                    borrow_amount: Decimal::from(5 * LAMPORTS_PER_SOL),
                    receive_amount: 5 * LAMPORTS_PER_SOL,
                    borrow_fee: 0,
                    host_fee: 0
                }),
            }),
            // borrow max where market price is 2x ema price
            Just(CalculateBorrowTestCase {
                borrow_amount: u64::MAX,
                remaining_borrow_value: Decimal::from(100u64),
                remaining_reserve_capacity: Decimal::from(100 * LAMPORTS_PER_SOL),

                market_price: Decimal::from(20u64),
                smoothed_market_price: Decimal::from(10u64),
                decimal: 9,
                added_borrow_weight_bps: 0,

                borrow_fee_wad: 0,
                host_fee: 0,

                result: Ok(CalculateBorrowResult {
                    borrow_amount: Decimal::from(5 * LAMPORTS_PER_SOL),
                    receive_amount: 5 * LAMPORTS_PER_SOL,
                    borrow_fee: 0,
                    host_fee: 0
                }),
            }),
            // borrow enough where it would be fine if we were just using the market price but
            // not fine when using both market and ema price
            Just(CalculateBorrowTestCase {
                borrow_amount: 7 * LAMPORTS_PER_SOL,
                remaining_borrow_value: Decimal::from(100u64),
                remaining_reserve_capacity: Decimal::from(100 * LAMPORTS_PER_SOL),

                market_price: Decimal::from(10u64),
                smoothed_market_price: Decimal::from(20u64),
                decimal: 9,
                added_borrow_weight_bps: 0,

                borrow_fee_wad: 0,
                host_fee: 0,

                result: Err(LendingError::BorrowTooLarge.into()),
            }),
        ]
    }

    proptest! {
        #[test]
        fn calculate_borrow(test_case in calculate_borrow_test_cases()) {
            let reserve = Reserve {
                config: ReserveConfig {
                    added_borrow_weight_bps: test_case.added_borrow_weight_bps,
                    fees: ReserveFees {
                        borrow_fee_wad: test_case.borrow_fee_wad,
                        host_fee_percentage: test_case.host_fee,
                        flash_loan_fee_wad: 0,
                    },
                    ..ReserveConfig::default()
                },
                liquidity: ReserveLiquidity {
                    mint_decimals: test_case.decimal,
                    market_price: test_case.market_price,
                    smoothed_market_price: test_case.smoothed_market_price,
                    available_amount: test_case.remaining_reserve_capacity.to_scaled_val().unwrap() as u64,
                    ..ReserveLiquidity::default()
                },
                ..Reserve::default()
            };
            assert_eq!(reserve.calculate_borrow(
                test_case.borrow_amount,
                test_case.remaining_borrow_value,
                test_case.remaining_reserve_capacity,
            ), test_case.result);
        }
    }
}

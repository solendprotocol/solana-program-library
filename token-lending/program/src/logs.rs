#![allow(missing_docs)]
use crate::math::Decimal;
use solana_program::pubkey::Pubkey;
use std::fmt;

extern crate serde;
extern crate serde_json;

#[derive(Debug, Serialize)]
pub enum LogEventType {
    ObligationStateUpdate,
    ProgramVersion,
    PythError,
    PythOraclePriceUpdate,
    ReserveStateUpdate,
    SwitchboardError,
    SwitchboardV1OraclePriceUpdate,
}

impl fmt::Display for LogEventType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

fn pubkey_serialize<S>(x: &Pubkey, s: S) -> Result<S::Ok, S::Error>
where
    S: serde::ser::Serializer,
{
    s.serialize_str(&x.to_string())
}

#[macro_export]
macro_rules! emit_log_event {
    ($e:expr) => {
        msg!("solend-event-log:");
        msg!(&serde_json::to_string($e).unwrap());
    };
}

#[derive(Serialize)]
pub struct PythOraclePriceUpdate {
    pub event_type: LogEventType,
    #[serde(serialize_with = "pubkey_serialize")]
    pub oracle_pubkey: Pubkey,
    pub price: Decimal,
    pub confidence: u64,
    pub published_slot: u64,
}

#[derive(Serialize)]
pub struct PythError {
    pub event_type: LogEventType,
    #[serde(serialize_with = "pubkey_serialize")]
    pub oracle_pubkey: Pubkey,
    pub error_message: String,
}

#[derive(Serialize)]
pub struct SwitchboardV1OraclePriceUpdate {
    pub event_type: LogEventType,
    #[serde(serialize_with = "pubkey_serialize")]
    pub oracle_pubkey: Pubkey,
    pub price: Decimal,
    pub published_slot: u64,
}

#[derive(Serialize)]
pub struct SwitchboardError {
    pub event_type: LogEventType,
    #[serde(serialize_with = "pubkey_serialize")]
    pub oracle_pubkey: Pubkey,
    pub error_message: String,
}

#[derive(Serialize)]
pub struct ProgramVersion {
    pub event_type: LogEventType,
    pub version: u8,
}

#[derive(Serialize)]
pub struct ReserveStateUpdate {
    pub event_type: LogEventType,
    pub available_amount: u64,
    pub borrowed_amount_wads: Decimal,
    pub cumulative_borrow_rate_wads: Decimal,
    pub collateral_mint_total_supply: u64,
    pub collateral_exchange_rate: String,
}

// ObligationStateUpdate intentionally does not contain the obligation ID
// to save on compute since it is contained in the transaction itself.
#[derive(Serialize)]
pub struct ObligationStateUpdate {
    pub event_type: LogEventType,
    pub allowed_borrow_value: Decimal,
    pub unhealthy_borrow_value: Decimal,
    pub deposits: Vec<DepositLog>,
    pub borrows: Vec<BorrowLog>,
}

#[derive(Serialize)]
pub struct DepositLog {
    pub reserve_id_index: u8,
    pub deposited_amount: u64,
}

#[derive(Serialize)]
pub struct BorrowLog {
    pub reserve_id_index: u8,
    pub borrowed_amount_wads: Decimal,
    pub cumulative_borrow_rate_wads: Decimal,
}

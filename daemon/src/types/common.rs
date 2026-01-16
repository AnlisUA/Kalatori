//! Common types shared across multiple modules

use std::fmt;

use chrono::{
    DateTime,
    Utc,
};
use rust_decimal::Decimal;
use serde::{
    Deserialize,
    Serialize,
};
use sqlx::types::Text;
use sqlx::{
    FromRow,
    Type,
};

pub use kalatori_client::types::ChainType;

/// Initiator type for payouts and refunds
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum InitiatorType {
    System,
    Admin,
}

impl fmt::Display for InitiatorType {
    fn fmt(
        &self,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        match self {
            Self::System => write!(f, "System"),
            Self::Admin => write!(f, "Admin"),
        }
    }
}

impl std::str::FromStr for InitiatorType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "System" => Ok(Self::System),
            "Admin" => Ok(Self::Admin),
            _ => Err(format!("Unknown initiator type: {s}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferInfo {
    pub chain: ChainType,
    pub asset_id: String,
    pub asset_name: String,
    pub amount: Decimal,
    pub source_address: String,
    pub destination_address: String,
}

#[derive(FromRow)]
pub struct TransferInfoRow {
    pub chain: ChainType,
    pub asset_id: String,
    pub asset_name: String,
    pub amount: Text<Decimal>,
    pub source_address: String,
    pub destination_address: String,
}

impl From<TransferInfoRow> for TransferInfo {
    fn from(value: TransferInfoRow) -> Self {
        Self {
            chain: value.chain,
            asset_id: value.asset_id,
            asset_name: value.asset_name,
            amount: value.amount.into_inner(),
            source_address: value.source_address,
            destination_address: value.destination_address,
        }
    }
}

/// Retry metadata for payouts and refunds
#[derive(Debug, Clone, Default, PartialEq, Eq, FromRow, Serialize, Deserialize)]
pub struct RetryMeta {
    pub retry_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_attempt_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_retry_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_message: Option<String>,
}

impl RetryMeta {
    fn retry_delay_secs(&self) -> i64 {
        // TODO: it's simplified strategy. In future might be better
        // to calculate delay based on average block execution time of the chain
        match self.retry_count {
            0 => 60,          // 1 minute
            1 => 5 * 60,      // 5 minutes
            2 => 15 * 60,     // 15 minutes
            3 => 30 * 60,     // 30 minutes
            4 => 60 * 60,     // 1 hour
            _ => 2 * 60 * 60, // 2 hours
        }
    }

    #[expect(clippy::arithmetic_side_effects)]
    pub fn increment_retry(
        &mut self,
        failure_message: String,
    ) {
        let now = Utc::now();
        self.retry_count += 1;
        self.last_attempt_at = Some(now);
        self.next_retry_at = Some(now + chrono::Duration::seconds(self.retry_delay_secs()));
        self.failure_message = Some(failure_message);
    }
}

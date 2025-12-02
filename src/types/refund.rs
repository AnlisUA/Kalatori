use std::fmt;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Type};
use uuid::Uuid;

use super::common::InitiatorType;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum RefundStatus {
    Waiting,
    InProgress,
    Completed,
    Failed,
}

impl fmt::Display for RefundStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Waiting => write!(f, "Waiting"),
            Self::InProgress => write!(f, "InProgress"),
            Self::Completed => write!(f, "Completed"),
            Self::Failed => write!(f, "Failed"),
        }
    }
}

impl std::str::FromStr for RefundStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Waiting" => Ok(Self::Waiting),
            "InProgress" => Ok(Self::InProgress),
            "Completed" => Ok(Self::Completed),
            "Failed" => Ok(Self::Failed),
            _ => Err(format!("Unknown refund status: {s}")),
        }
    }
}

#[expect(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, FromRow)]
pub struct Refund {
    pub id: Uuid,
    pub invoice_id: Uuid,
    pub asset_id: u32,
    pub chain: String,
    pub amount: Decimal,
    pub source_address: String,
    pub destination_address: String,
    pub allow_transfer_all: bool,
    pub initiator_type: InitiatorType,
    pub initiator_id: Option<Uuid>,
    pub status: RefundStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

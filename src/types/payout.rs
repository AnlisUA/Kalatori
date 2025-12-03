//! Payout types for `SQLite` schema

use std::fmt;

use chrono::{
    DateTime,
    Utc,
};
use serde::{
    Deserialize,
    Serialize,
};
use sqlx::{
    FromRow,
    Type,
};
use uuid::Uuid;

use super::common::InitiatorType;
use crate::legacy_types::WithdrawalStatus;

/// Payout status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum PayoutStatus {
    Waiting,
    InProgress,
    Completed,
    Failed,
}

impl fmt::Display for PayoutStatus {
    fn fmt(
        &self,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        match self {
            Self::Waiting => write!(f, "Waiting"),
            Self::InProgress => write!(f, "InProgress"),
            Self::Completed => write!(f, "Completed"),
            Self::Failed => write!(f, "Failed"),
        }
    }
}

impl std::str::FromStr for PayoutStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Waiting" => Ok(Self::Waiting),
            "InProgress" => Ok(Self::InProgress),
            "Completed" => Ok(Self::Completed),
            "Failed" => Ok(Self::Failed),
            _ => Err(format!("Unknown payout status: {s}")),
        }
    }
}

// Convert PayoutStatus to old WithdrawalStatus for backward compatibility
impl From<PayoutStatus> for WithdrawalStatus {
    fn from(status: PayoutStatus) -> Self {
        match status {
            PayoutStatus::Waiting | PayoutStatus::InProgress => Self::Waiting,
            PayoutStatus::Completed => Self::Completed,
            PayoutStatus::Failed => Self::Failed,
        }
    }
}

#[expect(dead_code)]
/// Payout from `SQLite`
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, FromRow)]
pub struct Payout {
    pub id: Uuid,
    pub invoice_id: Uuid,
    pub asset_id: u32,
    pub chain: String,
    pub source_address: String,
    pub destination_address: String,
    pub initiator_type: InitiatorType,
    pub initiator_id: Option<Uuid>,
    pub status: PayoutStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

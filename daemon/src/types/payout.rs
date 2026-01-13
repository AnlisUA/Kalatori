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

use super::{
    InitiatorType,
    RetryMeta,
    TransferInfo,
    TransferInfoRow,
    Invoice,
};

/// Payout status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum PayoutStatus {
    Waiting,
    InProgress,
    Completed,
    FailedRetriable,
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
            Self::FailedRetriable => write!(f, "FailedRetriable"),
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
            "FailedRetriable" => Ok(Self::FailedRetriable),
            "Failed" => Ok(Self::Failed),
            _ => Err(format!("Unknown payout status: {s}")),
        }
    }
}

/// Payout from `SQLite`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Payout {
    pub id: Uuid,
    pub invoice_id: Uuid,
    pub initiator_type: InitiatorType,
    pub initiator_id: Option<Uuid>,
    pub status: PayoutStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(flatten)]
    pub transfer_info: TransferInfo,
    #[serde(flatten)]
    pub retry_meta: RetryMeta,
}

impl Payout {
    pub fn from_invoice(
        invoice: Invoice,
        payout_address: String,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            invoice_id: invoice.id,
            transfer_info: TransferInfo {
                asset_id: invoice.asset_id,
                asset_name: invoice.asset_name,
                chain: invoice.chain,
                source_address: invoice.payment_address,
                destination_address: payout_address,
                amount: invoice.amount,
            },
            initiator_type: InitiatorType::System,
            initiator_id: None,
            status: PayoutStatus::Waiting,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            retry_meta: RetryMeta::default(),
        }
    }
}

#[derive(FromRow)]
pub struct PayoutRow {
    pub id: Uuid,
    pub invoice_id: Uuid,
    pub initiator_type: InitiatorType,
    pub initiator_id: Option<Uuid>,
    pub status: PayoutStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[sqlx(flatten)]
    pub transfer_info: TransferInfoRow,
    #[sqlx(flatten)]
    pub retry_meta: RetryMeta,
}

impl From<PayoutRow> for Payout {
    fn from(value: PayoutRow) -> Self {
        Self {
            id: value.id,
            invoice_id: value.invoice_id,
            transfer_info: value.transfer_info.into(),
            initiator_type: value.initiator_type,
            initiator_id: value.initiator_id,
            status: value.status,
            created_at: value.created_at,
            updated_at: value.updated_at,
            retry_meta: value.retry_meta,
        }
    }
}

#[cfg(test)]
pub fn default_payout(invoice_id: Uuid) -> Payout {
    let transfer_info = TransferInfo {
        asset_id: 1984.to_string(),
        asset_name: "USDT".to_string(),
        chain: super::ChainType::PolkadotAssetHub,
        source_address: "5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY".to_string(),
        destination_address: "1NthTCKurNHLW52mMa6iA8Gz7UFYW5UnM3yTSpVdGu4Th7h".to_string(),
        amount: rust_decimal::Decimal::new(1000, 2),
    };

    Payout {
        id: Uuid::new_v4(),
        invoice_id,
        transfer_info,
        initiator_type: InitiatorType::System,
        initiator_id: None,
        status: PayoutStatus::Waiting,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        retry_meta: RetryMeta::default(),
    }
}

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

use super::common::{
    InitiatorType,
    RetryMeta,
    TransferInfo,
    TransferInfoRow,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum RefundStatus {
    Waiting,
    InProgress,
    Completed,
    Failed,
}

impl fmt::Display for RefundStatus {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refund {
    pub id: Uuid,
    pub invoice_id: Uuid,
    pub initiator_type: InitiatorType,
    pub initiator_id: Option<Uuid>,
    pub status: RefundStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(flatten)]
    pub transfer_info: TransferInfo,
    #[serde(flatten)]
    pub retry_meta: RetryMeta,
}

#[derive(FromRow)]
pub struct RefundRow {
    pub id: Uuid,
    pub invoice_id: Uuid,
    pub initiator_type: InitiatorType,
    pub initiator_id: Option<Uuid>,
    pub status: RefundStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[sqlx(flatten)]
    pub transfer_info: TransferInfoRow,
    #[sqlx(flatten)]
    pub retry_meta: RetryMeta,
}

impl From<RefundRow> for Refund {
    fn from(row: RefundRow) -> Self {
        Self {
            id: row.id,
            invoice_id: row.invoice_id,
            initiator_type: row.initiator_type,
            initiator_id: row.initiator_id,
            status: row.status,
            created_at: row.created_at,
            updated_at: row.updated_at,
            transfer_info: row.transfer_info.into(),
            retry_meta: row.retry_meta,
        }
    }
}

#[cfg(test)]
pub fn default_refund(invoice_id: Uuid) -> Refund {
    let transfer_info = TransferInfo {
        asset_id: 1984.to_string(),
        asset_name: "USDT".to_string(),
        chain: super::ChainType::PolkadotAssetHub,
        amount: rust_decimal::Decimal::new(5000, 2), // 50.00
        source_address: "5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY".to_string(),
        destination_address: "5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty".to_string(),
    };

    Refund {
        id: Uuid::new_v4(),
        invoice_id,
        transfer_info,
        initiator_type: InitiatorType::Admin,
        initiator_id: Some(Uuid::new_v4()),
        status: RefundStatus::Waiting,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        retry_meta: RetryMeta::default(),
    }
}

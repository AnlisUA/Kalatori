//! Transaction types for `SQLite` schema

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
use sqlx::types::{
    Json,
    Text,
};
use sqlx::{
    FromRow,
    Type,
};
use uuid::Uuid;

/// Transaction type (incoming or outgoing)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum TransactionType {
    Incoming,
    Outgoing,
}

impl fmt::Display for TransactionType {
    fn fmt(
        &self,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        match self {
            Self::Incoming => write!(f, "Incoming"),
            Self::Outgoing => write!(f, "Outgoing"),
        }
    }
}

impl std::str::FromStr for TransactionType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Incoming" => Ok(Self::Incoming),
            "Outgoing" => Ok(Self::Outgoing),
            _ => Err(format!("Unknown transaction type: {s}")),
        }
    }
}

/// Transaction status (for new schema)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum TransactionStatus {
    Waiting,
    InProgress,
    Completed,
    Failed,
}

impl fmt::Display for TransactionStatus {
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

impl std::str::FromStr for TransactionStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Waiting" => Ok(Self::Waiting),
            "InProgress" => Ok(Self::InProgress),
            "Completed" => Ok(Self::Completed),
            "Failed" => Ok(Self::Failed),
            _ => Err(format!(
                "Unknown transaction status: {s}"
            )),
        }
    }
}

/// Origin field for transactions (what triggered this transaction)
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransactionOrigin {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refund_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payout_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub internal_transfer_id: Option<Uuid>,
}

/// Metadata for outgoing transactions
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutgoingTransactionMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extrinsic_bytes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub built_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sent_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirmed_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_message: Option<String>,
}

/// Transaction from `SQLite`
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, FromRow)]
pub struct Transaction {
    pub id: Uuid,
    pub invoice_id: Uuid,
    pub asset_id: u32,
    pub chain: String,
    pub amount: Decimal,
    pub sender: String,
    pub recipient: String,
    pub block_number: Option<u32>,
    pub position_in_block: Option<u32>,
    pub tx_hash: Option<String>,
    pub origin: TransactionOrigin,
    pub status: TransactionStatus,
    pub transaction_type: TransactionType,
    pub outgoing_meta: OutgoingTransactionMeta,
    pub created_at: DateTime<Utc>,
    pub transaction_bytes: Option<String>, // Backward compat (hex-encoded extrinsic)
}

#[derive(FromRow)]
pub struct TransactionRow {
    pub id: Uuid,
    pub invoice_id: Uuid,
    pub asset_id: u32,
    pub chain: String,
    pub amount: Text<Decimal>,
    pub sender: String,
    pub recipient: String,
    pub block_number: Option<u32>,
    pub position_in_block: Option<u32>,
    pub tx_hash: Option<String>,
    pub origin: Json<TransactionOrigin>,
    pub status: TransactionStatus,
    pub transaction_type: TransactionType,
    pub outgoing_meta: Json<OutgoingTransactionMeta>,
    pub created_at: DateTime<Utc>,
    pub transaction_bytes: Option<String>,
}

impl From<TransactionRow> for Transaction {
    fn from(row: TransactionRow) -> Self {
        Self {
            id: row.id,
            invoice_id: row.invoice_id,
            asset_id: row.asset_id,
            chain: row.chain,
            amount: row.amount.0,
            sender: row.sender,
            recipient: row.recipient,
            block_number: row.block_number,
            position_in_block: row.position_in_block,
            tx_hash: row.tx_hash,
            origin: row.origin.0,
            status: row.status,
            transaction_type: row.transaction_type,
            outgoing_meta: row.outgoing_meta.0,
            created_at: row.created_at,
            transaction_bytes: row.transaction_bytes,
        }
    }
}

#[cfg(test)]
pub fn default_transaction(invoice_id: Uuid) -> Transaction {
    Transaction {
        id: Uuid::new_v4(),
        invoice_id,
        asset_id: 1984,
        chain: "statemint".to_string(),
        amount: Decimal::new(10000, 2),
        sender: "5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY".to_string(),
        recipient: "5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty".to_string(),
        block_number: Some(1000),
        position_in_block: Some(2),
        tx_hash: Some("0x1234567890abcdef".to_string()),
        origin: TransactionOrigin::default(),
        status: TransactionStatus::Waiting,
        transaction_type: TransactionType::Incoming,
        outgoing_meta: OutgoingTransactionMeta::default(),
        created_at: Utc::now(),
        transaction_bytes: Some("0xabcdef123456".to_string()),
    }
}

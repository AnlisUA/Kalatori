use std::fmt;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::types::{Text, Json};
use sqlx::{FromRow, Type};
use uuid::Uuid;

use crate::legacy_types::{PaymentStatus, WithdrawalStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum InvoiceStatus {
    // Active statuses
    Waiting,
    PartiallyPaid,
    // Final statuses
    Paid,
    OverPaid,
    AdminApproved,
    // Expired statuses
    UnpaidExpired,
    PartiallyPaidExpired,
    // Canceled statuses
    CustomerCanceled,
    AdminCanceled,
}

impl InvoiceStatus {
    /// Check if invoice is in an active state (still being monitored)
    pub const fn is_active(&self) -> bool {
        matches!(self, Self::Waiting | Self::PartiallyPaid)
    }

    /// Check if invoice is in a final state (completed)
    pub const fn is_final(&self) -> bool {
        matches!(self, Self::Paid | Self::OverPaid | Self::AdminApproved)
    }

    /// Check if invoice is expired
    pub const fn is_expired(&self) -> bool {
        matches!(self, Self::UnpaidExpired | Self::PartiallyPaidExpired)
    }

    /// Check if invoice is canceled
    pub const fn is_canceled(&self) -> bool {
        matches!(self, Self::CustomerCanceled | Self::AdminCanceled)
    }
}

// Convert InvoiceStatus to old PaymentStatus for backward compatibility
impl From<InvoiceStatus> for PaymentStatus {
    fn from(status: InvoiceStatus) -> Self {
        match status {
            InvoiceStatus::Paid | InvoiceStatus::OverPaid | InvoiceStatus::AdminApproved => {
                Self::Paid
            }
            _ => Self::Pending,
        }
    }
}

// Convert old PaymentStatus to InvoiceStatus
impl From<PaymentStatus> for InvoiceStatus {
    fn from(status: PaymentStatus) -> Self {
        match status {
            PaymentStatus::Pending => Self::Waiting,
            PaymentStatus::Paid => Self::Paid,
        }
    }
}

impl fmt::Display for InvoiceStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Waiting => write!(f, "Waiting"),
            Self::PartiallyPaid => write!(f, "PartiallyPaid"),
            Self::Paid => write!(f, "Paid"),
            Self::OverPaid => write!(f, "OverPaid"),
            Self::AdminApproved => write!(f, "AdminApproved"),
            Self::UnpaidExpired => write!(f, "UnpaidExpired"),
            Self::PartiallyPaidExpired => write!(f, "PartiallyPaidExpired"),
            Self::CustomerCanceled => write!(f, "CustomerCanceled"),
            Self::AdminCanceled => write!(f, "AdminCanceled"),
        }
    }
}

impl std::str::FromStr for InvoiceStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Waiting" => Ok(Self::Waiting),
            "PartiallyPaid" => Ok(Self::PartiallyPaid),
            "Paid" => Ok(Self::Paid),
            "OverPaid" => Ok(Self::OverPaid),
            "AdminApproved" => Ok(Self::AdminApproved),
            "UnpaidExpired" => Ok(Self::UnpaidExpired),
            "PartiallyPaidExpired" => Ok(Self::PartiallyPaidExpired),
            "CustomerCanceled" => Ok(Self::CustomerCanceled),
            "AdminCanceled" => Ok(Self::AdminCanceled),
            _ => Err(format!("Unknown invoice status: {s}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceCartItem {
    pub name: String,
    pub quantity: u32,
    pub unit_price: Decimal,
    pub icon_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceCart {
    pub items: Vec<InvoiceCartItem>,
}

impl InvoiceCart {
    // Prefer to create an empty cart explicitly over using Default trait
    pub fn empty() -> Self {
        Self { items: vec![] }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invoice {
    pub id: Uuid,
    pub order_id: String, // Merchant-provided order ID
    pub asset_id: Option<u32>,
    pub chain: String,
    pub amount: Decimal,
    pub payment_address: String,
    pub status: InvoiceStatus,
    pub withdrawal_status: WithdrawalStatus, // Temporary backward compat field
    pub callback: String,
    pub cart: InvoiceCart,
    pub valid_till: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(FromRow)]
pub struct InvoiceRow {
    pub id: Uuid,
    pub order_id: String,
    pub asset_id: Option<u32>,
    pub chain: String,
    pub amount: Text<Decimal>,
    pub payment_address: String,
    pub status: InvoiceStatus,
    pub withdrawal_status: WithdrawalStatus,
    pub callback: String,
    pub cart: Json<InvoiceCart>,
    pub valid_till: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<InvoiceRow> for Invoice {
    fn from(row: InvoiceRow) -> Self {
        Self {
            id: row.id,
            order_id: row.order_id,
            asset_id: row.asset_id,
            chain: row.chain,
            amount: row.amount.0,
            payment_address: row.payment_address,
            status: row.status,
            withdrawal_status: row.withdrawal_status,
            callback: row.callback,
            cart: row.cart.0,
            valid_till: row.valid_till,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug)]
pub struct CreateInvoiceData {
    pub order_id: String,
    pub asset_id: u32,
    pub chain: String,
    pub amount: Decimal,
    pub payment_address: String,
    pub callback: String,
    pub cart: InvoiceCart,
    pub valid_till: DateTime<Utc>,
}

#[derive(Debug)]
pub struct UpdateInvoiceData {
    pub amount: Decimal,
    pub cart: InvoiceCart,
    pub valid_till: DateTime<Utc>,
}

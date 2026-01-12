use std::fmt;

use serde::{Serialize, Deserialize};
use uuid::Uuid;
use rust_decimal::Decimal;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "sqlx-types", derive(sqlx::Type))]
pub enum InvoiceStatus {
    // Active statuses
    Waiting,
    PartiallyPaid,
    // Final statuses
    Paid,
    OverPaid,
    // Expired statuses
    UnpaidExpired,
    PartiallyPaidExpired,
    // Canceled statuses
    CustomerCanceled,
    AdminCanceled,
}

impl InvoiceStatus {
    /// Check if invoice is in an active state (still being monitored)
    pub const fn is_active(self) -> bool {
        matches!(
            self,
            Self::Waiting | Self::PartiallyPaid
        )
    }

    /// Check if invoice is in a final state (completed)
    pub const fn is_final(self) -> bool {
        matches!(self, Self::Paid | Self::OverPaid)
    }

    /// Check if invoice is expired
    pub const fn is_expired(self) -> bool {
        matches!(
            self,
            Self::UnpaidExpired | Self::PartiallyPaidExpired
        )
    }

    /// Check if invoice is canceled
    pub const fn is_canceled(self) -> bool {
        matches!(
            self,
            Self::CustomerCanceled | Self::AdminCanceled
        )
    }
}

impl fmt::Display for InvoiceStatus {
    fn fmt(
        &self,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        match self {
            Self::Waiting => write!(f, "Waiting"),
            Self::PartiallyPaid => write!(f, "PartiallyPaid"),
            Self::Paid => write!(f, "Paid"),
            Self::OverPaid => write!(f, "OverPaid"),
            Self::UnpaidExpired => write!(f, "UnpaidExpired"),
            Self::PartiallyPaidExpired => write!(f, "PartiallyPaidExpired"),
            Self::CustomerCanceled => write!(f, "CustomerCanceled"),
            Self::AdminCanceled => write!(f, "AdminCanceled"),
        }
    }
}

impl std::str::FromStr for InvoiceStatus {
    // TODO: use a custom error type
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Waiting" => Ok(Self::Waiting),
            "PartiallyPaid" => Ok(Self::PartiallyPaid),
            "Paid" => Ok(Self::Paid),
            "OverPaid" => Ok(Self::OverPaid),
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
    pub price: Decimal, // Price per single item
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tax: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discount: Option<Decimal>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceCart {
    pub items: Vec<InvoiceCartItem>,
}

impl InvoiceCart {
    // Prefer to create an empty cart explicitly over using Default trait
    pub fn empty() -> Self {
        Self {
            items: vec![],
        }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invoice {
    pub id: Uuid,
    pub order_id: String,
    pub asset: String,
    pub asset_id: String,
    pub chain: String,
    pub amount: Decimal,
    pub payment_address: String,
    pub status: InvoiceStatus,
    pub payment_url: String,
    pub redirect_url: String,
    pub valid_till: DateTime<Utc>,
    pub cart: InvoiceCart,
    pub total_received_amount: Decimal,
    // pub transactions: Vec<Transaction>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateInvoiceParams {
    pub order_id: String,
    pub amount: Decimal,
    #[serde(default = "InvoiceCart::empty")]
    #[serde(skip_serializing_if = "InvoiceCart::is_empty")]
    pub cart: InvoiceCart,
    pub redirect_url: String,
}

fn default_include_transaction() -> bool {
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetInvoiceParams {
    pub invoice_id: Uuid,
    #[serde(default = "default_include_transaction")]
    pub include_transaction: bool,
}

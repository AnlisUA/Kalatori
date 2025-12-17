use std::fmt;

use chrono::{
    DateTime,
    Duration,
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

use crate::legacy_types::{
    CurrencyInfo,
    OrderQuery,
    PaymentStatus,
    Timestamp,
    WithdrawalStatus,
};

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

#[expect(dead_code)]
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
        matches!(
            self,
            Self::Paid | Self::OverPaid | Self::AdminApproved
        )
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

// Convert InvoiceStatus to old PaymentStatus for backward compatibility
impl From<InvoiceStatus> for PaymentStatus {
    fn from(status: InvoiceStatus) -> Self {
        match status {
            InvoiceStatus::Paid | InvoiceStatus::OverPaid | InvoiceStatus::AdminApproved => {
                Self::Paid
            },
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
    fn fmt(
        &self,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
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
        Self {
            items: vec![],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invoice {
    pub id: Uuid,
    pub order_id: String, // Merchant-provided order ID
    // TODO: make it non-optional, for native asset use asset_id = 0
    pub asset_id: Option<u32>,
    pub chain: String,
    pub amount: Decimal,
    pub payment_address: String,
    pub status: InvoiceStatus,
    pub withdrawal_status: WithdrawalStatus, // Temporary backward compat field
    pub callback: String,
    pub cart: InvoiceCart,
    pub redirect_url: String,
    pub valid_till: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub version: u16, // Optimistic locking version, auto-incremented on updates
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
    pub redirect_url: String,
    pub valid_till: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub version: u16,
}

impl From<InvoiceRow> for Invoice {
    fn from(row: InvoiceRow) -> Self {
        Self {
            id: row.id,
            order_id: row.order_id,
            asset_id: row.asset_id,
            chain: row.chain,
            amount: row.amount.into_inner(),
            payment_address: row.payment_address,
            status: row.status,
            withdrawal_status: row.withdrawal_status,
            callback: row.callback,
            cart: row.cart.0,
            redirect_url: row.redirect_url,
            valid_till: row.valid_till,
            created_at: row.created_at,
            updated_at: row.updated_at,
            version: row.version,
        }
    }
}

#[expect(dead_code)]
#[derive(Debug)]
pub struct CreateInvoiceData {
    pub order_id: String,
    pub asset_id: u32,
    pub chain: String,
    pub amount: Decimal,
    pub payment_address: String,
    pub callback: String,
    pub cart: InvoiceCart,
    pub redirect_url: String,
    pub valid_till: DateTime<Utc>,
}

#[derive(Debug)]
pub struct UpdateInvoiceData {
    pub id: Uuid, // Invoice ID to update
    pub amount: Decimal,
    pub cart: InvoiceCart,
    pub valid_till: DateTime<Utc>,
    pub version: u16, // Current version for optimistic locking
}

// Conversion utilities for backward compatibility with V2 API

impl Invoice {
    /// Create a new Invoice from `OrderQuery` (V2 API input)
    ///
    /// # Errors
    /// Returns error if amount conversion from f64 to Decimal fails
    pub fn from_order_query(
        order_query: OrderQuery,
        currency_info: CurrencyInfo,
        payment_address: String,
        account_lifetime: Timestamp,
    ) -> Result<Self, String> {
        let now = Utc::now();
        let valid_till = calculate_valid_till(account_lifetime);
        let amount = f64_to_decimal(order_query.amount)?;

        Ok(Self {
            id: Uuid::new_v4(),
            order_id: order_query.order,
            asset_id: currency_info.asset_id,
            chain: currency_info.chain_name,
            amount,
            payment_address,
            status: InvoiceStatus::Waiting,
            withdrawal_status: WithdrawalStatus::Waiting,
            callback: order_query.callback,
            cart: InvoiceCart::empty(),
            redirect_url: order_query.redirect_url,
            valid_till,
            created_at: now,
            updated_at: now,
            version: 1,
        })
    }
}

/// Convert f64 amount to Decimal
///
/// # Errors
/// Returns error if the f64 value cannot be represented as Decimal
fn f64_to_decimal(amount: f64) -> Result<Decimal, String> {
    Decimal::try_from(amount)
        .map_err(|e| format!("Failed to convert amount {amount} to Decimal: {e}"))
}

/// Calculate `valid_till` timestamp from `account_lifetime`
pub fn calculate_valid_till(account_lifetime: Timestamp) -> DateTime<Utc> {
    let lifetime_ms = account_lifetime.0;
    #[expect(clippy::cast_possible_wrap)]
    let duration = Duration::milliseconds(lifetime_ms as i64);
    #[expect(clippy::arithmetic_side_effects)]
    {
        Utc::now() + duration
    }
}

#[cfg(test)]
pub fn default_invoice() -> Invoice {
    let now = Utc::now();
    let id = Uuid::new_v4();

    Invoice {
        id,
        order_id: id.to_string(),
        asset_id: Some(1984),
        chain: "statemint".to_string(),
        amount: Decimal::new(10000, 2),
        payment_address: "5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY".to_string(),
        status: InvoiceStatus::Waiting,
        withdrawal_status: WithdrawalStatus::Waiting,
        callback: "http://localhost:8080/callback".to_string(),
        cart: InvoiceCart::empty(),
        redirect_url: "http://localhost:8080/thankyou".to_string(),
        #[expect(clippy::arithmetic_side_effects)]
        valid_till: now + Duration::hours(24),
        created_at: now,
        updated_at: now,
        version: 1,
    }
}

#[cfg(test)]
pub fn default_update_invoice_data(invoice_id: Uuid) -> UpdateInvoiceData {
    let now = Utc::now();

    UpdateInvoiceData {
        id: invoice_id,
        amount: Decimal::new(15000, 2),
        cart: InvoiceCart::empty(),
        #[expect(clippy::arithmetic_side_effects)]
        valid_till: now + Duration::hours(24),
        version: 1,
    }
}

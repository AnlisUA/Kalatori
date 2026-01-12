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
use sqlx::FromRow;
use uuid::Uuid;

use super::ChainType;

// Re-export types from kalatori_client for consistency
pub use kalatori_client::types::{
    InvoiceCart,
    InvoiceStatus,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invoice {
    pub id: Uuid,
    // Merchant-provided order ID
    pub order_id: String,
    pub asset_id: String,
    pub chain: ChainType,
    pub amount: Decimal,
    pub payment_address: String,
    pub status: InvoiceStatus,
    // Temporary backward compat field
    pub callback: String,
    pub cart: InvoiceCart,
    pub redirect_url: String,
    pub valid_till: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub version: u16, // Optimistic locking version, auto-incremented on updates
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceWithIncomingAmount {
    pub invoice: Invoice,
    pub incoming_amount: Decimal,
}

#[derive(FromRow)]
pub struct InvoiceRow {
    pub id: Uuid,
    pub order_id: String,
    pub asset_id: String,
    pub chain: ChainType,
    pub amount: Text<Decimal>,
    pub payment_address: String,
    pub status: InvoiceStatus,
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

#[derive(Debug, Clone)]
pub struct CreateInvoiceData {
    pub id: Uuid,
    pub order_id: String,
    pub asset_id: String,
    pub chain: ChainType,
    pub amount: Decimal,
    pub payment_address: String,
    pub cart: InvoiceCart,
    pub redirect_url: String,
    pub callback_url: Option<String>,
    pub valid_till: DateTime<Utc>,
}

impl From<CreateInvoiceData> for Invoice {
    fn from(data: CreateInvoiceData) -> Self {
        let now = Utc::now();

        Self {
            id: data.id,
            order_id: data.order_id,
            asset_id: data.asset_id,
            chain: data.chain,
            amount: data.amount,
            payment_address: data.payment_address,
            status: InvoiceStatus::Waiting,
            // TODO: make it optional in DB as well
            callback: data.callback_url.unwrap_or_default(),
            cart: data.cart,
            redirect_url: data.redirect_url,
            valid_till: data.valid_till,
            created_at: now,
            updated_at: now,
            version: 1,
        }
    }
}

#[derive(Debug)]
pub struct UpdateInvoiceData {
    pub id: Uuid, // Invoice ID to update
    pub amount: Decimal,
    pub cart: InvoiceCart,
    pub valid_till: DateTime<Utc>,
    pub version: u16, // Current version for optimistic locking
}

#[cfg(test)]
pub fn default_invoice() -> Invoice {
    default_create_invoice_data().into()
}

#[cfg(test)]
pub fn default_create_invoice_data() -> CreateInvoiceData {
    let now = Utc::now();
    let id = Uuid::new_v4();

    CreateInvoiceData {
        id,
        order_id: id.to_string(),
        asset_id: 1984.to_string(),
        chain: ChainType::PolkadotAssetHub,
        amount: Decimal::new(10000, 2),
        payment_address: "5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY".to_string(),
        callback_url: None,
        cart: InvoiceCart::empty(),
        redirect_url: "http://localhost:8080/thankyou".to_string(),
        #[expect(clippy::arithmetic_side_effects)]
        valid_till: now + chrono::Duration::hours(24),
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
        valid_till: now + chrono::Duration::hours(24),
        version: 1,
    }
}

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::InvoiceCart;

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

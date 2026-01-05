use rust_decimal::Decimal;
use serde::Deserialize;
use uuid::Uuid;

use crate::types::InvoiceCart;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CreateInvoiceParams {
    pub order_id: String,
    pub asset_id: Option<String>,
    pub amount: Decimal,
    #[serde(default = "InvoiceCart::empty")]
    pub cart: InvoiceCart,
    pub redirect_url: String,
    pub callback_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct UpdateInvoiceParams {
    pub invoice_id: Uuid,
    pub amount: Option<Decimal>,
    pub cart: Option<InvoiceCart>,
}

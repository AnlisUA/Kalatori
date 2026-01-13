use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::InvoiceCart;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    pub category: String,
    pub code: String,
    pub message: String,
    // pub details: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ApiResultStructured<T> {
    Ok {
        result: T,
    },
    Err {
        error: ApiError,
    },
}

pub type ApiResult<T> = Result<T, ApiError>;

impl<T> From<ApiResultStructured<T>> for ApiResult<T> {
    fn from(value: ApiResultStructured<T>) -> Self {
        match value {
            ApiResultStructured::Ok { result } => Ok(result),
            ApiResultStructured::Err { error } => Err(error),
        }
    }
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateInvoiceParams {
    pub invoice_id: Uuid,
    pub amount: Decimal,
    #[serde(default = "InvoiceCart::empty")]
    #[serde(skip_serializing_if = "InvoiceCart::is_empty")]
    pub cart: InvoiceCart,
}

pub type CancelInvoiceParams = GetInvoiceParams;

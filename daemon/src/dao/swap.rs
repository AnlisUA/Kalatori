use alloy::primitives::Address;
use sqlx::types::Text;
use thiserror::Error;
use uuid::Uuid;

use crate::types::FrontEndSwap;

use super::DaoExecutor;

#[derive(sqlx::FromRow)]
struct FrontEndSwapRow {
    #[expect(dead_code)]
    pub id: Uuid,
    pub invoice_id: Uuid,
    pub from_amount_units: Text<u128>,
    pub from_chain_id: u32,
    pub from_asset_id: Text<Address>,
    pub transaction_hash: String,
}

impl From<FrontEndSwapRow> for FrontEndSwap {
    fn from(value: FrontEndSwapRow) -> Self {
        Self {
            invoice_id: value.invoice_id,
            from_amount_units: value.from_amount_units.0,
            from_chain_id: value.from_chain_id,
            from_asset_id: value.from_asset_id.0,
            transaction_hash: value.transaction_hash,
        }
    }
}

#[derive(Debug, Error)]
pub enum DaoSwapError {
    /// Referenced invoice doesn't exist (foreign key violation)
    #[error("Invoice not found: {invoice_id}")]
    InvoiceNotFound { invoice_id: Uuid },

    #[error("Database error during swap operation")]
    DatabaseError,
}

impl From<sqlx::Error> for DaoSwapError {
    fn from(_e: sqlx::Error) -> Self {
        DaoSwapError::DatabaseError
    }
}

impl crate::api::ApiErrorExt for DaoSwapError {
    // TODO: create enum for categories and codes
    fn category(&self) -> &str {
        match self {
            DaoSwapError::InvoiceNotFound {
                ..
            } => "RELATED_ENTITY_NOT_FOUND",
            DaoSwapError::DatabaseError => "INTERNAL_SERVER_ERROR",
        }
    }

    fn code(&self) -> &str {
        match self {
            DaoSwapError::InvoiceNotFound {
                ..
            } => "RELATED_INVOICE_NOT_FOUND",
            DaoSwapError::DatabaseError => "INTERNAL_SERVER_ERROR",
        }
    }

    fn message(&self) -> &str {
        match self {
            DaoSwapError::InvoiceNotFound {
                ..
            } => "The related invoice id was not found.",
            DaoSwapError::DatabaseError => "A database error occurred.",
        }
    }

    fn http_status_code(&self) -> reqwest::StatusCode {
        match self {
            DaoSwapError::InvoiceNotFound {
                ..
            } => reqwest::StatusCode::BAD_REQUEST,
            DaoSwapError::DatabaseError => reqwest::StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

pub trait DaoSwapMethods: DaoExecutor + 'static {
    async fn create_front_end_swap(
        &self,
        swap: FrontEndSwap,
    ) -> Result<FrontEndSwap, DaoSwapError> {
        let query = sqlx::query_as::<_, FrontEndSwapRow>(
            "INSERT INTO front_end_swaps (id, invoice_id, from_amount_units, from_chain_id, from_asset_id, transaction_hash)
            VALUES (?, ?, ?, ?, ?, ?)
            RETURNING *"
        )
        .bind(Uuid::new_v4())
        .bind(swap.invoice_id)
        .bind(Text(swap.from_amount_units))
        .bind(swap.from_chain_id)
        .bind(Text(swap.from_asset_id))
        .bind(&swap.transaction_hash);

        self.fetch_one(query)
            .await
            .map_err(|e| {
                tracing::debug!(
                    error.category = "dao.swap",
                    error.operation = "create_front_end_swap",
                    invoice_id = %swap.invoice_id,
                    transaction_hash = %swap.transaction_hash,
                    error.source = ?e,
                    "Failed to create front end swap"
                );

                match &e {
                    sqlx::Error::Database(db_err) => {
                        let message = db_err.message();

                        if message.contains("FOREIGN KEY") {
                            return DaoSwapError::InvoiceNotFound {
                                invoice_id: swap.invoice_id,
                            };
                        }

                        DaoSwapError::DatabaseError
                    },
                    _ => DaoSwapError::DatabaseError,
                }
            })
    }
}

impl<T: DaoExecutor + 'static> DaoSwapMethods for T {}

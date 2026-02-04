use thiserror::Error;
use uuid::Uuid;
use sqlx::types::{Text, Json};

use crate::types::{OneInchSwap, OneInchSwapRow, OneInchSwapStatus};

use super::DaoExecutor;
use super::error_parsing::{StatusTriggerError, StatusTransitionError};

#[derive(Debug, Error)]
pub enum DaoSwapError {
    #[error("Swap not found: {swap_id}")]
    NotFound { swap_id: Uuid },
    #[error("Cannot transition from {current_status} to {attempted_status}")]
    StatusConstraintViolation {
        current_status: OneInchSwapStatus,
        attempted_status: OneInchSwapStatus,
    },
    #[error("Database error during swap operation")]
    DatabaseError,
}

impl From<StatusTriggerError<OneInchSwapStatus>> for DaoSwapError {
    fn from(e: StatusTriggerError<OneInchSwapStatus>) -> Self {
        DaoSwapError::StatusConstraintViolation {
            current_status: e.old_status,
            attempted_status: e.new_status,
        }
    }
}

impl StatusTransitionError for OneInchSwapStatus {
    type ErrorType = DaoSwapError;

    const ERROR_TYPE_PREFIX: &'static str = "SWAP_STATUS_TRANSITION|";
}

pub trait DaoSwapMethods: DaoExecutor + 'static {
    async fn create_swap(
        &self,
        swap: impl Into<OneInchSwap>,
    ) -> Result<OneInchSwap, DaoSwapError> {
        let swap = swap.into();

        let query = sqlx::query_as::<_, OneInchSwapRow>(
            "INSERT INTO swaps (id, invoice_id, from_chain, to_chain, from_token_address, to_token_address, from_amount_units, from_address, to_address, status, to_amount, order_hash, secrets, raw_order, created_at, valid_till)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            RETURNING *"
        )
        .bind(swap.id)
        .bind(swap.request.invoice_id)
        .bind(swap.request.from_chain)
        .bind(swap.request.to_chain)
        .bind(Text(swap.request.from_token_address))
        .bind(Text(swap.request.to_token_address))
        .bind(Text(swap.request.from_amount_units))
        .bind(Text(swap.request.from_address))
        .bind(Text(swap.request.to_address))
        .bind(swap.status)
        .bind(Text(swap.to_amount))
        .bind(Text(swap.order_hash))
        .bind(Json(swap.secrets))
        .bind(Json(swap.raw_order))
        .bind(swap.created_at.to_rfc3339())
        .bind(swap.valid_till.to_rfc3339());

        self.fetch_one(query)
            .await
            .map_err(|e| {
                tracing::debug!(
                    error.category = "dao.swap",
                    error.operation = "create_swap",
                    error.source = ?e,
                    swap.id = %swap.id,
                );
                println!("Error: {:?}", e);

                DaoSwapError::DatabaseError
            })
    }

    async fn get_submitted_swaps(&self) -> Result<Vec<OneInchSwap>, DaoSwapError> {
        let query = sqlx::query_as::<_, OneInchSwapRow>(
            "UPDATE swaps
            SET status = 'Pending'
            WHERE status = 'Submitted'
            RETURNING *"
        );

        self.fetch_all(query)
            .await
            .map_err(|e| {
                tracing::debug!(
                    error.category = "dao.swap",
                    error.operation = "get_submitted_swaps",
                    error.source = ?e,
                    "Failed get get submitted swaps and set their status to 'Pending'"
                );

                DaoSwapError::DatabaseError
            })
    }

    async fn update_swap_submitted(
        &self,
        swap_id: Uuid,
    ) -> Result<OneInchSwap, DaoSwapError> {
        let query = sqlx::query_as::<_, OneInchSwapRow>(
            "UPDATE swaps
            SET status = 'Submitted', submitted_at = datetime('now')
            WHERE id = ?
            RETURNING *"
        )
        .bind(swap_id);

        self.fetch_one(query)
            .await
            .map_err(|e| {
                tracing::debug!(
                    error.category = "dao.swap",
                    error.operation = "update_swap_submitted",
                    %swap_id,
                    error.source = ?e,
                    "Failed to update swap as submitted"
                );

                if let Some(error) = OneInchSwapStatus::from_sqlx_error(&e) {
                    return error;
                }

                match e {
                    sqlx::Error::RowNotFound => DaoSwapError::NotFound {
                        swap_id,
                    },
                    _ => DaoSwapError::DatabaseError
                }
            })
    }

    async fn update_swap_completed(
        &self,
        swap_id: Uuid,
    ) -> Result<OneInchSwap, DaoSwapError> {
        let query = sqlx::query_as::<_, OneInchSwapRow>(
            "UPDATE swaps
            SET status = 'Completed', finished_at = datetime('now')
            WHERE id = ?
            RETURNING *"
        )
        .bind(swap_id);

        self.fetch_one(query)
            .await
            .map_err(|e| {
                tracing::debug!(
                    error.category = "dao.swap",
                    error.operation = "update_swap_completed",
                    %swap_id,
                    error.source = ?e,
                    "Failed to update swap as completed"
                );

                if let Some(error) = OneInchSwapStatus::from_sqlx_error(&e) {
                    return error;
                }

                match e {
                    sqlx::Error::RowNotFound => DaoSwapError::NotFound {
                        swap_id,
                    },
                    _ => DaoSwapError::DatabaseError
                }
            })
    }

    async fn update_swap_failed(
        &self,
        swap_id: Uuid,
        error_message: String,
    ) -> Result<OneInchSwap, DaoSwapError> {
        let query = sqlx::query_as::<_, OneInchSwapRow>(
            "UPDATE swaps
            SET status = 'Failed', error_message = ?, finished_at = datetime('now')
            WHERE id = ?
            RETURNING *"
        )
        .bind(error_message.to_string())
        .bind(swap_id);

        self.fetch_one(query)
            .await
            .map_err(|e| {
                tracing::debug!(
                    error.category = "dao.swap",
                    error.operation = "update_swap_completed",
                    %swap_id,
                    error.source = ?e,
                    "Failed to update swap as completed"
                );

                if let Some(error) = OneInchSwapStatus::from_sqlx_error(&e) {
                    return error;
                }

                match e {
                    sqlx::Error::RowNotFound => DaoSwapError::NotFound {
                        swap_id,
                    },
                    _ => DaoSwapError::DatabaseError
                }
            })
    }
}

impl<T: DaoExecutor + 'static> DaoSwapMethods for T {}

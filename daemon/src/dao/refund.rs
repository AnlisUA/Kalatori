use sqlx::types::Text;
use thiserror::Error;
use uuid::Uuid;

use crate::types::{
    Refund,
    RefundRow,
    RefundStatus,
};

use super::DaoExecutor;
use super::error_parsing::{
    StatusTransitionError,
    TriggerError,
};

// ============================================================================
// Refund Domain Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum DaoRefundError {
    /// Refund not found by ID
    #[error("Refund not found: {refund_id}")]
    NotFound { refund_id: Uuid },

    /// Referenced invoice doesn't exist (foreign key violation)
    #[error("Invoice not found: {invoice_id}")]
    InvoiceNotFound { invoice_id: Uuid },

    /// Status transition not allowed
    #[error("Cannot transition from {current_status} to {attempted_status}")]
    StatusConstraintViolation {
        current_status: RefundStatus,
        attempted_status: RefundStatus,
    },

    /// Database operation failed
    #[error("Database error during refund operation")]
    DatabaseError,
}

impl From<sqlx::Error> for DaoRefundError {
    fn from(_e: sqlx::Error) -> Self {
        DaoRefundError::DatabaseError
    }
}

impl From<TriggerError<RefundStatus>> for DaoRefundError {
    fn from(e: TriggerError<RefundStatus>) -> Self {
        DaoRefundError::StatusConstraintViolation {
            current_status: e.old_status,
            attempted_status: e.new_status,
        }
    }
}

impl StatusTransitionError for RefundStatus {
    type ErrorType = DaoRefundError;

    const ERROR_TYPE_PREFIX: &'static str = "REFUND_STATUS_TRANSITION|";
}

pub trait DaoRefundMethods: DaoExecutor + 'static {
    #[cfg_attr(not(test), expect(dead_code))]
    async fn create_refund(
        &self,
        refund: Refund,
    ) -> Result<Refund, DaoRefundError> {
        let query = sqlx::query_as::<_, RefundRow>(
        "INSERT INTO refunds (id, invoice_id, asset_id, asset_name, chain, amount, source_address, destination_address, initiator_type, initiator_id, status, created_at, updated_at, retry_count, last_attempt_at, next_retry_at, failure_message)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            RETURNING *"
        )
            .bind(refund.id)
            .bind(refund.invoice_id)
            .bind(refund.transfer_info.asset_id)
            .bind(&refund.transfer_info.asset_name)
            .bind(&refund.transfer_info.chain)
            .bind(Text(refund.transfer_info.amount))
            .bind(&refund.transfer_info.source_address)
            .bind(&refund.transfer_info.destination_address)
            .bind(refund.initiator_type)
            .bind(refund.initiator_id)
            .bind(refund.status)
            .bind(refund.created_at.naive_utc())
            .bind(refund.updated_at.naive_utc())
            .bind(refund.retry_meta.retry_count)
            .bind(refund.retry_meta.last_attempt_at.map(|dt| dt.naive_utc()))
            .bind(refund.retry_meta.next_retry_at.map(|dt| dt.naive_utc()))
            .bind(&refund.retry_meta.failure_message);

        self.fetch_one(query)
            .await
            .map_err(|e| {
                tracing::debug!(
                    error.category = "dao.refund",
                    error.operation = "create_refund",
                    refund_id = %refund.id,
                    invoice_id = %refund.invoice_id,
                    error.source = ?e,
                    "Failed to create refund"
                );

                match &e {
                    sqlx::Error::Database(db_err) => {
                        let message = db_err.message();

                        if message.contains("FOREIGN KEY") {
                            return DaoRefundError::InvoiceNotFound {
                                invoice_id: refund.invoice_id,
                            };
                        }

                        DaoRefundError::DatabaseError
                    },
                    _ => DaoRefundError::DatabaseError,
                }
            })
    }

    #[cfg_attr(not(test), expect(dead_code))]
    async fn get_refund_by_id(
        &self,
        refund_id: Uuid,
    ) -> Result<Option<Refund>, DaoRefundError> {
        let query = sqlx::query_as::<_, RefundRow>(
            "SELECT *
                FROM refunds
                WHERE id = ?",
        )
        .bind(refund_id);

        self.fetch_optional(query)
            .await
            .map_err(|e| {
                tracing::debug!(
                    error.category = "dao.refund",
                    error.operation = "get_refund_by_id",
                    %refund_id,
                    error.source = ?e,
                    "Failed to fetch refund"
                );
                DaoRefundError::DatabaseError
            })
    }

    #[cfg_attr(not(test), expect(dead_code))]
    async fn get_pending_refunds(&self) -> Result<Vec<Refund>, DaoRefundError> {
        let query = sqlx::query_as::<_, RefundRow>(
            "SELECT *
            FROM refunds
            WHERE status = 'Waiting'
            AND (next_retry_at IS NULL OR next_retry_at <= datetime('now'))
            ORDER BY created_at ASC",
        );

        self.fetch_all(query)
            .await
            .map_err(|e| {
                tracing::debug!(
                    error.category = "dao.refund",
                    error.operation = "get_pending_refunds",
                    error.source = ?e,
                    "Failed to fetch pending refunds"
                );
                DaoRefundError::DatabaseError
            })
    }

    #[cfg_attr(not(test), expect(dead_code))]
    async fn update_refund_status(
        &self,
        refund_id: Uuid,
        status: RefundStatus,
    ) -> Result<Refund, DaoRefundError> {
        let query = sqlx::query_as::<_, RefundRow>(
            "UPDATE refunds
            SET status = ?, updated_at = datetime('now')
            WHERE id = ?
            RETURNING *",
        )
        .bind(status)
        .bind(refund_id);

        self.fetch_one(query)
            .await
            .map_err(|e| {
                tracing::debug!(
                    error.category = "dao.refund",
                    error.operation = "update_refund_status",
                    %refund_id,
                    new_status = ?status,
                    error.source = ?e,
                    "Failed to update refund status"
                );

                // Parse with RefundStatus type
                if let Some(error) = RefundStatus::from_sqlx_error(&e) {
                    return error;
                }

                match e {
                    sqlx::Error::RowNotFound => DaoRefundError::NotFound {
                        refund_id,
                    },
                    _ => DaoRefundError::DatabaseError,
                }
            })
    }

    #[cfg_attr(not(test), expect(dead_code))]
    async fn update_refund_retry(
        &self,
        refund_id: Uuid,
        retry_count: i32,
        last_attempt_at: chrono::DateTime<chrono::Utc>,
        next_retry_at: Option<chrono::DateTime<chrono::Utc>>,
        failure_message: Option<String>,
    ) -> Result<Refund, DaoRefundError> {
        let query = sqlx::query_as::<_, RefundRow>(
            "UPDATE refunds
            SET retry_count = ?,
                last_attempt_at = ?,
                next_retry_at = ?,
                failure_message = ?,
                updated_at = datetime('now')
            WHERE id = ?
            RETURNING *",
        )
        .bind(retry_count)
        .bind(last_attempt_at)
        .bind(next_retry_at)
        .bind(&failure_message)
        .bind(refund_id);

        self.fetch_one(query)
            .await
            .map_err(|e| {
                tracing::debug!(
                    error.category = "dao.refund",
                    error.operation = "update_refund_retry",
                    %refund_id,
                    retry_count,
                    error.source = ?e,
                    "Failed to update refund retry"
                );

                // Check for trigger violation
                if let Some(error) = RefundStatus::from_sqlx_error(&e) {
                    return error;
                }

                match e {
                    sqlx::Error::RowNotFound => DaoRefundError::NotFound {
                        refund_id,
                    },
                    _ => DaoRefundError::DatabaseError,
                }
            })
    }
}

impl<T: DaoExecutor + 'static> DaoRefundMethods for T {}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use crate::dao::create_test_dao;
    use crate::dao::invoice::DaoInvoiceMethods;
    use crate::types::{
        default_create_invoice_data,
        default_refund,
    };

    use super::*;

    #[tokio::test]
    async fn test_refund_create_and_get() {
        let dao = create_test_dao().await;
        let invoice = dao
            .create_invoice(default_create_invoice_data())
            .await
            .unwrap();

        // Create refund
        let refund = default_refund(invoice.id);
        let refund_id = refund.id;
        let created = dao.create_refund(refund).await.unwrap();

        // Verify fields
        assert_eq!(created.id, refund_id);
        assert_eq!(created.invoice_id, invoice.id);
        assert_eq!(created.status, RefundStatus::Waiting);
        assert_eq!(created.retry_meta.retry_count, 0);

        // Get by ID
        let fetched = dao
            .get_refund_by_id(refund_id)
            .await
            .unwrap();
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().id, refund_id);

        // Get non-existent
        let not_found = dao
            .get_refund_by_id(Uuid::new_v4())
            .await
            .unwrap();
        assert!(not_found.is_none());
    }

    #[tokio::test]
    async fn test_get_pending_refunds_filtering() {
        let dao = create_test_dao().await;
        let invoice = dao
            .create_invoice(default_create_invoice_data())
            .await
            .unwrap();

        // Create refund with Waiting status (should be returned)
        let refund1 = default_refund(invoice.id);
        dao.create_refund(refund1)
            .await
            .unwrap();

        // Create refund with InProgress status (should NOT be returned)
        let mut refund2 = default_refund(invoice.id);
        refund2.status = RefundStatus::InProgress;
        dao.create_refund(refund2)
            .await
            .unwrap();

        // Create refund with Completed status (should NOT be returned)
        let mut refund3 = default_refund(invoice.id);
        refund3.status = RefundStatus::Completed;
        dao.create_refund(refund3)
            .await
            .unwrap();

        // Get pending refunds
        let pending = dao.get_pending_refunds().await.unwrap();

        // Should only return refund1
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].status, RefundStatus::Waiting);
    }

    #[tokio::test]
    async fn test_update_refund_status() {
        let dao = create_test_dao().await;
        let invoice = dao
            .create_invoice(default_create_invoice_data())
            .await
            .unwrap();

        let refund = default_refund(invoice.id);
        let refund_id = refund.id;
        let created = dao.create_refund(refund).await.unwrap();
        assert_eq!(created.status, RefundStatus::Waiting);

        // Update to InProgress
        let updated = dao
            .update_refund_status(refund_id, RefundStatus::InProgress)
            .await
            .unwrap();
        assert_eq!(updated.status, RefundStatus::InProgress);

        // Update to Completed
        let completed = dao
            .update_refund_status(refund_id, RefundStatus::Completed)
            .await
            .unwrap();
        assert_eq!(
            completed.status,
            RefundStatus::Completed
        );
    }

    #[tokio::test]
    async fn test_update_refund_retry() {
        let dao = create_test_dao().await;
        let invoice = dao
            .create_invoice(default_create_invoice_data())
            .await
            .unwrap();

        let refund = default_refund(invoice.id);
        let refund_id = refund.id;
        dao.create_refund(refund).await.unwrap();

        // First retry
        let now = Utc::now();
        let next_retry = now + chrono::Duration::minutes(1);
        let updated = dao
            .update_refund_retry(
                refund_id,
                1,
                now,
                Some(next_retry),
                Some("Insufficient balance".to_string()),
            )
            .await
            .unwrap();

        assert_eq!(updated.retry_meta.retry_count, 1);
        assert!(
            updated
                .retry_meta
                .last_attempt_at
                .is_some()
        );
        assert!(
            updated
                .retry_meta
                .next_retry_at
                .is_some()
        );
        assert_eq!(
            updated.retry_meta.failure_message,
            Some("Insufficient balance".to_string())
        );
    }

    #[tokio::test]
    async fn test_refund_status_transition_triggers() {
        let dao = create_test_dao().await;
        let invoice = default_create_invoice_data();
        let invoice_id = invoice.id;
        dao.create_invoice(invoice)
            .await
            .unwrap();

        // Scenario 1: Invalid transition from Completed -> Waiting
        let refund1 = Refund {
            status: RefundStatus::Completed,
            ..default_refund(invoice_id)
        };
        let id1 = refund1.id;
        dao.create_refund(refund1)
            .await
            .unwrap();

        let result = dao
            .update_refund_status(id1, RefundStatus::Waiting)
            .await;
        match result.unwrap_err() {
            DaoRefundError::StatusConstraintViolation {
                current_status,
                attempted_status,
            } => {
                assert_eq!(current_status, RefundStatus::Completed);
                assert_eq!(attempted_status, RefundStatus::Waiting);
            },
            err => panic!("Expected StatusConstraintViolation, got: {err:?}"),
        }

        // Scenario 2: Valid transition Waiting -> InProgress -> Completed
        let refund2 = Refund {
            status: RefundStatus::Waiting,
            ..default_refund(invoice_id)
        };
        let id2 = refund2.id;
        dao.create_refund(refund2)
            .await
            .unwrap();

        let updated1 = dao
            .update_refund_status(id2, RefundStatus::InProgress)
            .await
            .unwrap();
        assert_eq!(
            updated1.status,
            RefundStatus::InProgress
        );

        let updated2 = dao
            .update_refund_status(id2, RefundStatus::Completed)
            .await
            .unwrap();
        assert_eq!(updated2.status, RefundStatus::Completed);
    }
}

use sqlx::types::{
    Json,
    Text,
};
use thiserror::Error;
use uuid::Uuid;

use crate::legacy_types::WithdrawalStatus;
use crate::types::{
    Invoice,
    InvoiceRow,
    InvoiceStatus,
    InvoiceWithIncomingAmount,
    UpdateInvoiceData,
};

use super::DaoExecutor;
use super::error_parsing::{
    StatusTransitionError,
    TriggerError,
};

// ============================================================================
// Invoice Domain Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum DaoInvoiceError {
    /// Invoice not found by ID or `order_id`
    #[error("Invoice not found: {identifier}")]
    NotFound {
        identifier: String, // Can be UUID string or order_id
    },

    /// Optimistic locking failure - invoice was modified by another request
    #[error("Invoice {invoice_id} was modified (expected version {expected_version})")]
    VersionConflict {
        invoice_id: Uuid,
        expected_version: u16,
    },

    /// Status transition not allowed (invoice in wrong state)
    #[error("Cannot transition from {current_status} to {attempted_status}")]
    StatusConstraintViolation {
        current_status: InvoiceStatus,
        attempted_status: InvoiceStatus,
    },

    /// Withdrawal status constraint violation
    #[error("Cannot transition withdrawal from {current_status} to {attempted_status}")]
    WithdrawalStatusConstraintViolation {
        current_status: WithdrawalStatus,
        attempted_status: WithdrawalStatus,
    },

    /// Duplicate `order_id` (UNIQUE constraint violation)
    #[error("Order ID '{order_id}' already exists")]
    DuplicateOrderId { order_id: String },

    /// Database operation failed
    #[error("Database error during invoice operation")]
    DatabaseError,
}

impl From<sqlx::Error> for DaoInvoiceError {
    fn from(_e: sqlx::Error) -> Self {
        // Only convert generic database errors
        // Specific errors are handled at call site
        DaoInvoiceError::DatabaseError
    }
}

impl From<TriggerError<InvoiceStatus>> for DaoInvoiceError {
    fn from(err: TriggerError<InvoiceStatus>) -> Self {
        DaoInvoiceError::StatusConstraintViolation {
            current_status: err.old_status,
            attempted_status: err.new_status,
        }
    }
}

impl From<TriggerError<WithdrawalStatus>> for DaoInvoiceError {
    fn from(err: TriggerError<WithdrawalStatus>) -> Self {
        DaoInvoiceError::WithdrawalStatusConstraintViolation {
            current_status: err.old_status,
            attempted_status: err.new_status,
        }
    }
}

impl StatusTransitionError for InvoiceStatus {
    type ErrorType = DaoInvoiceError;

    const ERROR_TYPE_PREFIX: &'static str = "INVOICE_STATUS_TRANSITION|";
}

impl StatusTransitionError for WithdrawalStatus {
    type ErrorType = DaoInvoiceError;

    const ERROR_TYPE_PREFIX: &'static str = "INVOICE_WITHDRAWAL_TRANSITION|";
}

#[derive(sqlx::FromRow)]
struct InvoiceWithAmountsRow {
    #[sqlx(flatten)]
    invoice: InvoiceRow,
    amounts: sqlx::types::Json<Vec<String>>,
}

impl From<InvoiceWithAmountsRow> for InvoiceWithIncomingAmount {
    fn from(row: InvoiceWithAmountsRow) -> Self {
        let incoming_amount = row
            .amounts
            .0
            .into_iter()
            .filter_map(|amt_str| {
                amt_str
                    .parse::<rust_decimal::Decimal>()
                    .ok()
            })
            .sum();

        Self {
            invoice: row.invoice.into(),
            incoming_amount,
        }
    }
}

pub trait DaoInvoiceMethods: DaoExecutor + 'static {
    async fn create_invoice(
        &self,
        invoice: Invoice,
    ) -> Result<Invoice, DaoInvoiceError> {
        let query = sqlx::query_as::<_, InvoiceRow>(
        "INSERT INTO invoices (id, order_id, asset_id, chain, amount, payment_address, status, withdrawal_status, callback, cart, redirect_url, valid_till, created_at, updated_at, version)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            RETURNING *"
        )
            .bind(invoice.id)
            .bind(&invoice.order_id)
            .bind(invoice.asset_id)
            .bind(&invoice.chain)
            .bind(Text(invoice.amount))
            .bind(&invoice.payment_address)
            .bind(invoice.status)
            .bind(invoice.withdrawal_status)
            .bind(&invoice.callback)
            .bind(Json(invoice.cart))
            .bind(invoice.redirect_url)
            .bind(invoice.valid_till)
            .bind(invoice.created_at)
            .bind(invoice.updated_at)
            .bind(invoice.version);

        self.fetch_one(query)
            .await
            .map_err(|e| {
                tracing::debug!(
                    error.category = "dao.invoice",
                    error.operation = "create_invoice",
                    order_id = %invoice.order_id,
                    invoice_id = %invoice.id,
                    error.source = ?e,
                    "Failed to create invoice"
                );

                match &e {
                    sqlx::Error::Database(db_err) => {
                        let message = db_err.message();

                        if message.contains("UNIQUE") && message.contains("order_id") {
                            return DaoInvoiceError::DuplicateOrderId {
                                order_id: invoice.order_id.clone(),
                            };
                        }

                        DaoInvoiceError::DatabaseError
                    },
                    _ => DaoInvoiceError::DatabaseError,
                }
            })
    }

    async fn get_invoice_by_id(
        &self,
        invoice_id: Uuid,
    ) -> Result<Option<Invoice>, DaoInvoiceError> {
        let query = sqlx::query_as::<_, InvoiceRow>(
            "SELECT *
            FROM invoices
            WHERE id = ?",
        )
        .bind(invoice_id);

        self.fetch_optional(query)
            .await
            .map_err(|e| {
                tracing::debug!(
                    error.category = "dao.invoice",
                    error.operation = "get_invoice_by_id",
                    %invoice_id,
                    error.source = ?e,
                    "Failed to fetch invoice"
                );
                DaoInvoiceError::DatabaseError
            })
    }

    async fn get_invoice_by_order_id(
        &self,
        order_id: &str,
    ) -> Result<Option<Invoice>, DaoInvoiceError> {
        let query = sqlx::query_as::<_, InvoiceRow>(
            "SELECT *
            FROM invoices
            WHERE order_id = ?",
        )
        .bind(order_id);

        self.fetch_optional(query)
            .await
            .map_err(|e| {
                tracing::debug!(
                    error.category = "dao.invoice",
                    error.operation = "get_invoice_by_order_id",
                    %order_id,
                    error.source = ?e,
                    "Failed to fetch invoice by order_id"
                );
                DaoInvoiceError::DatabaseError
            })
    }

    /// Get all active invoices that need to be monitored and total amount of
    /// received incoming transactions. We suppose that invoices with status
    /// 'Waiting' or '`PartiallyPaid`' don't have outgoing transactions,
    /// so they are not included in calculations.
    /// Returns invoices with status 'Waiting' or '`PartiallyPaid`'
    async fn get_active_invoices_with_amounts(
        &self
    ) -> Result<Vec<InvoiceWithIncomingAmount>, DaoInvoiceError> {
        let query = sqlx::query_as::<_, InvoiceWithAmountsRow>(
            "SELECT
                i.*,
                CASE
                    WHEN COUNT(t.amount) = 0 THEN '[]'
                    ELSE json_group_array(t.amount)
                END as amounts
            FROM invoices i
            LEFT JOIN transactions t
                ON i.id = t.invoice_id
                AND t.transaction_type = 'Incoming'
            WHERE i.status IN ('Waiting', 'PartiallyPaid')
            GROUP BY i.id
            ORDER BY i.created_at ASC",
        );

        self.fetch_all(query)
            .await
            .map_err(|e| {
                tracing::debug!(
                    error.category = "dao.invoice",
                    error.operation = "get_invoices_paid_amount",
                    error.source = ?e,
                    "Failed to fetch paid amounts for invoices"
                );
                DaoInvoiceError::DatabaseError
            })
    }

    async fn update_invoice_status(
        &self,
        invoice_id: Uuid,
        status: InvoiceStatus,
    ) -> Result<Invoice, DaoInvoiceError> {
        let query = sqlx::query_as::<_, InvoiceRow>(
            "UPDATE invoices
            SET status = ?,
                updated_at = datetime('now'),
                version = version + 1
            WHERE id = ?
            RETURNING *",
        )
        .bind(status)
        .bind(invoice_id);

        self.fetch_one(query)
            .await
            .map_err(|e| {
                tracing::debug!(
                    error.category = "dao.invoice",
                    error.operation = "update_invoice_status",
                    %invoice_id,
                    new_status = ?status,
                    error.source = ?e,
                    "Failed to update invoice status"
                );

                // Check for trigger violation
                if let Some(error) = InvoiceStatus::from_sqlx_error(&e) {
                    return error;
                }

                match e {
                    sqlx::Error::RowNotFound => DaoInvoiceError::NotFound {
                        identifier: invoice_id.to_string(),
                    },
                    _ => DaoInvoiceError::DatabaseError,
                }
            })
    }

    async fn update_invoice_data(
        &self,
        data: UpdateInvoiceData,
    ) -> Result<Invoice, DaoInvoiceError> {
        let query = sqlx::query_as::<_, InvoiceRow>(
            "UPDATE invoices
            SET amount = ?,
                cart = ?,
                valid_till = ?,
                updated_at = datetime('now'),
                version = version + 1
            WHERE id = ? AND status = 'Waiting' AND version = ?
            RETURNING *",
        )
        .bind(Text(data.amount))
        .bind(Json(data.cart))
        .bind(data.valid_till)
        .bind(data.id)
        .bind(data.version);

        match self.fetch_one(query).await {
            Ok(row) => Ok(row),
            Err(e) => {
                tracing::debug!(
                    error.category = "dao.invoice",
                    error.operation = "update_invoice_data",
                    invoice_id = %data.id,
                    expected_version = data.version,
                    error.source = ?e,
                    "Update failed"
                );

                // Check for trigger error first - parse with InvoiceStatus type
                if let Some(error) = InvoiceStatus::from_sqlx_error(&e) {
                    return Err(error);
                }

                // Not a trigger error, check if RowNotFound
                if matches!(e, sqlx::Error::RowNotFound) {
                    // Query current status to determine if NotFound or VersionConflict
                    let diagnostic_query = sqlx::query_as::<_, (i32, String)>(
                        "SELECT version, status FROM invoices WHERE id = ?",
                    )
                    .bind(data.id);

                    match self
                        .fetch_optional(diagnostic_query)
                        .await
                    {
                        Ok(Some((_current_version, _current_status))) => {
                            // Invoice exists but version mismatch
                            // (status check was in WHERE clause, trigger would have fired if
                            // wrong)
                            Err(DaoInvoiceError::VersionConflict {
                                invoice_id: data.id,
                                expected_version: data.version,
                            })
                        },
                        Ok(None) => Err(DaoInvoiceError::NotFound {
                            identifier: data.id.to_string(),
                        }),
                        Err(e) => {
                            tracing::warn!(
                                error.category = "dao.invoice",
                                error.operation = "update_invoice_data.diagnostic",
                                invoice_id = %data.id,
                                error.source = ?e,
                                "Diagnostic query failed"
                            );
                            Err(DaoInvoiceError::DatabaseError)
                        },
                    }
                } else {
                    Err(DaoInvoiceError::DatabaseError)
                }
            },
        }
    }

    async fn update_invoice_withdrawal_status(
        &self,
        invoice_id: Uuid,
        status: WithdrawalStatus,
    ) -> Result<Invoice, DaoInvoiceError> {
        let query = sqlx::query_as::<_, InvoiceRow>(
            "UPDATE invoices
            SET withdrawal_status = ?,
                updated_at = datetime('now'),
                version = version + 1
            WHERE id = ?
            RETURNING *",
        )
        .bind(status)
        .bind(invoice_id);

        match self.fetch_one(query).await {
            Ok(row) => Ok(row),
            Err(e) => {
                tracing::debug!(
                    error.category = "dao.invoice",
                    error.operation = "update_invoice_withdrawal_status",
                    %invoice_id,
                    new_status = ?status,
                    error.source = ?e,
                    "Update failed"
                );

                // Parse trigger error with WithdrawalStatus type
                if let Some(error) = WithdrawalStatus::from_sqlx_error(&e) {
                    return Err(error);
                }

                // RowNotFound means invoice doesn't exist
                match e {
                    sqlx::Error::RowNotFound => Err(DaoInvoiceError::NotFound {
                        identifier: invoice_id.to_string(),
                    }),
                    _ => Err(DaoInvoiceError::DatabaseError),
                }
            },
        }
    }

    async fn update_invoices_expired(&self) -> Result<Vec<Invoice>, DaoInvoiceError> {
        let query = sqlx::query_as::<_, InvoiceRow>(
            "UPDATE invoices
            SET status = 'UnpaidExpired',
                updated_at = datetime('now'),
                version = version + 1
            WHERE status = 'Waiting' AND valid_till < datetime('now')
            RETURNING *",
        );

        self.fetch_all(query)
            .await
            .map_err(|e| {
                tracing::debug!(
                    error.category = "dao.invoice",
                    error.operation = "update_invoices_expired",
                    error.source = ?e,
                    "Failed to update expired invoices"
                );
                DaoInvoiceError::DatabaseError
            })
    }
}

impl<T: DaoExecutor + 'static> DaoInvoiceMethods for T {}

#[cfg(test)]
mod tests {
    use rust_decimal::Decimal;

    use crate::dao::create_test_dao;
    use crate::dao::transaction::DaoTransactionMethods;
    use crate::types::{
        Transaction,
        TransactionType,
        default_invoice,
        default_transaction,
        default_update_invoice_data,
    };

    use super::*;

    #[expect(clippy::too_many_lines)]
    #[tokio::test]
    async fn test_get_active_invoices_with_amounts() {
        let dao = create_test_dao().await;

        // Create invoice 1 with Waiting status (will have 2 incoming transactions)
        let invoice1 = default_invoice();
        let invoice1_id = invoice1.id;
        dao.create_invoice(invoice1)
            .await
            .unwrap();

        // Create 2 incoming transactions for invoice1
        let tx1_amount = Decimal::new(10050, 2); // 100.50
        let tx1 = Transaction {
            id: Uuid::new_v4(),
            invoice_id: invoice1_id,
            amount: tx1_amount,
            transaction_type: TransactionType::Incoming,
            ..default_transaction(invoice1_id)
        };
        dao.create_transaction(tx1)
            .await
            .unwrap();

        let tx2_amount = Decimal::new(5025, 2); // 50.25
        let tx2 = Transaction {
            id: Uuid::new_v4(),
            invoice_id: invoice1_id,
            amount: tx2_amount,
            transaction_type: TransactionType::Incoming,
            ..default_transaction(invoice1_id)
        };
        dao.create_transaction(tx2)
            .await
            .unwrap();

        // Create invoice 2 with PartiallyPaid status (will have 1 incoming transaction)
        let invoice2 = Invoice {
            status: InvoiceStatus::PartiallyPaid,
            ..default_invoice()
        };
        let invoice2_id = invoice2.id;
        dao.create_invoice(invoice2)
            .await
            .unwrap();

        let tx3_amount = Decimal::new(7599, 2); // 75.99
        let tx3 = Transaction {
            id: Uuid::new_v4(),
            invoice_id: invoice2_id,
            amount: tx3_amount,
            transaction_type: TransactionType::Incoming,
            ..default_transaction(invoice2_id)
        };
        dao.create_transaction(tx3)
            .await
            .unwrap();

        // Create invoice 3 with Waiting status (no transactions)
        let invoice3 = default_invoice();
        let invoice3_id = invoice3.id;
        dao.create_invoice(invoice3)
            .await
            .unwrap();

        // Create invoice 4 with Waiting status and an Outgoing transaction (should not
        // be counted)
        let invoice4 = default_invoice();
        let invoice4_id = invoice4.id;
        dao.create_invoice(invoice4)
            .await
            .unwrap();

        let tx4_outgoing = Transaction {
            id: Uuid::new_v4(),
            invoice_id: invoice4_id,
            amount: Decimal::new(10000, 2),
            transaction_type: TransactionType::Outgoing,
            ..default_transaction(invoice4_id)
        };
        dao.create_transaction(tx4_outgoing)
            .await
            .unwrap();

        // Create invoice 5 with Paid status (should not be in results)
        let invoice5 = Invoice {
            status: InvoiceStatus::Paid,
            ..default_invoice()
        };
        let invoice5_id = invoice5.id;
        dao.create_invoice(invoice5)
            .await
            .unwrap();

        let tx5_amount = Decimal::new(10000, 2);
        let tx5 = Transaction {
            id: Uuid::new_v4(),
            invoice_id: invoice5_id,
            amount: tx5_amount,
            transaction_type: TransactionType::Incoming,
            ..default_transaction(invoice5_id)
        };
        dao.create_transaction(tx5)
            .await
            .unwrap();

        // Execute the test
        let results = dao
            .get_active_invoices_with_amounts()
            .await
            .unwrap();

        // Should return 4 active invoices (Waiting and PartiallyPaid only)
        assert_eq!(results.len(), 4);

        // Find each invoice in results
        let invoice1_result = results
            .iter()
            .find(|r| r.invoice.id == invoice1_id)
            .expect("Invoice 1 should be in results");

        let invoice2_result = results
            .iter()
            .find(|r| r.invoice.id == invoice2_id)
            .expect("Invoice 2 should be in results");

        let invoice3_result = results
            .iter()
            .find(|r| r.invoice.id == invoice3_id)
            .expect("Invoice 3 should be in results");

        let invoice4_result = results
            .iter()
            .find(|r| r.invoice.id == invoice4_id)
            .expect("Invoice 4 should be in results");

        // Verify amounts are summed correctly with full precision
        let expected_invoice1_total = tx1_amount + tx2_amount; // 100.50 + 50.25 = 150.75
        assert_eq!(
            invoice1_result.incoming_amount, expected_invoice1_total,
            "Invoice 1 should have sum of 2 incoming transactions"
        );

        assert_eq!(
            invoice2_result.incoming_amount, tx3_amount,
            "Invoice 2 should have amount from single incoming transaction"
        );

        assert_eq!(
            invoice3_result.incoming_amount,
            Decimal::ZERO,
            "Invoice 3 should have zero incoming amount (no transactions)"
        );

        assert_eq!(
            invoice4_result.incoming_amount,
            Decimal::ZERO,
            "Invoice 4 should have zero incoming amount (only outgoing transaction)"
        );

        // Verify invoice 5 (Paid status) is NOT in results
        assert!(
            results
                .iter()
                .all(|r| r.invoice.id != invoice5_id),
            "Paid invoice should not be in active invoices results"
        );

        // Verify ordering (should be by created_at ASC)
        assert_eq!(
            results[0].invoice.id, invoice1_id,
            "First invoice should be invoice1"
        );
        assert_eq!(
            results[1].invoice.id, invoice2_id,
            "Second invoice should be invoice2"
        );
        assert_eq!(
            results[2].invoice.id, invoice3_id,
            "Third invoice should be invoice3"
        );
        assert_eq!(
            results[3].invoice.id, invoice4_id,
            "Fourth invoice should be invoice4"
        );
    }

    #[tokio::test]
    async fn test_invoice_crud_operations() {
        let dao = create_test_dao().await;

        // Create invoice
        let invoice = default_invoice();
        let invoice_id = invoice.id;
        let order_id = invoice.order_id.clone();

        let created = dao
            .create_invoice(invoice)
            .await
            .unwrap();

        // Verify created invoice fields
        assert_eq!(created.id, invoice_id);
        assert_eq!(created.order_id, order_id);
        assert_eq!(created.version, 1);
        assert_eq!(created.status, InvoiceStatus::Waiting);

        // Get by ID - should return Some
        let by_id = dao
            .get_invoice_by_id(invoice_id)
            .await
            .unwrap();
        assert!(by_id.is_some());
        let by_id = by_id.unwrap();
        assert_eq!(by_id.id, invoice_id);
        assert_eq!(by_id.order_id, order_id);

        // Get by order_id - should return Some
        let by_order = dao
            .get_invoice_by_order_id(&order_id)
            .await
            .unwrap();
        assert!(by_order.is_some());
        let by_order = by_order.unwrap();
        assert_eq!(by_order.id, invoice_id);
        assert_eq!(by_order.order_id, order_id);

        // Get by non-existent ID - should return None
        let non_existent_id = dao
            .get_invoice_by_id(Uuid::new_v4())
            .await
            .unwrap();
        assert!(non_existent_id.is_none());

        // Get by non-existent order_id - should return None
        let non_existent_order = dao
            .get_invoice_by_order_id("non_existent_order")
            .await
            .unwrap();
        assert!(non_existent_order.is_none());
    }

    #[tokio::test]
    async fn test_create_invoice_duplicate_order_id_fails() {
        let dao = create_test_dao().await;

        // Create first invoice
        let invoice1 = default_invoice();
        let order_id = invoice1.order_id.clone();
        dao.create_invoice(invoice1)
            .await
            .unwrap();

        // Try to create second invoice with same order_id
        let invoice2 = Invoice {
            order_id: order_id.clone(),
            ..default_invoice()
        };

        let result = dao.create_invoice(invoice2).await;

        // Should fail with DuplicateOrderId error
        assert!(result.is_err());
        match result.unwrap_err() {
            DaoInvoiceError::DuplicateOrderId {
                order_id: oid,
            } => {
                assert_eq!(oid, order_id);
            },
            err => panic!("Expected DuplicateOrderId error, got: {err:?}"),
        }
    }

    #[tokio::test]
    async fn test_update_invoice_status_and_triggers() {
        let dao = create_test_dao().await;

        // Create invoice with Waiting status
        let invoice = default_invoice();
        let invoice_id = invoice.id;
        let created = dao
            .create_invoice(invoice)
            .await
            .unwrap();

        assert_eq!(created.status, InvoiceStatus::Waiting);
        assert_eq!(created.version, 1);
        let original_updated_at = created.updated_at;

        // Sleep to ensure timestamp will change
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // Update status to Paid
        let updated = dao
            .update_invoice_status(invoice_id, InvoiceStatus::Paid)
            .await
            .unwrap();

        // Verify status changed
        assert_eq!(updated.status, InvoiceStatus::Paid);

        // Verify trigger incremented version
        assert_eq!(updated.version, 2);

        // Verify trigger updated timestamp
        assert_ne!(updated.updated_at, original_updated_at);

        // Try to update non-existent invoice
        let result = dao
            .update_invoice_status(Uuid::new_v4(), InvoiceStatus::Paid)
            .await;

        // Should fail with NotFound
        assert!(result.is_err());
        match result.unwrap_err() {
            DaoInvoiceError::NotFound {
                ..
            } => { /* Expected */ },
            err => panic!("Expected NotFound, got: {err:?}"),
        }
    }

    #[tokio::test]
    async fn test_update_invoice_data_happy_path() {
        let dao = create_test_dao().await;

        // Create invoice (version=1, amount=100.00)
        let invoice = default_invoice();
        let invoice_id = invoice.id;
        let created = dao
            .create_invoice(invoice)
            .await
            .unwrap();

        assert_eq!(created.version, 1);
        assert_eq!(
            created.amount,
            rust_decimal::Decimal::new(10000, 2)
        );

        // Update amount to 150.00 with version=1
        let mut update_data = default_update_invoice_data(invoice_id);
        update_data.version = 1;
        let expected_cart = update_data.cart.clone();

        let updated = dao
            .update_invoice_data(update_data)
            .await
            .unwrap();

        // Verify amount updated
        assert_eq!(
            updated.amount,
            rust_decimal::Decimal::new(15000, 2)
        );

        // Verify version incremented
        assert_eq!(updated.version, 2);

        // Verify cart and valid_till also updated
        assert_eq!(updated.cart, expected_cart);

        // Update again with version=2
        let mut update_data2 = default_update_invoice_data(invoice_id);
        update_data2.version = 2;
        update_data2.amount = rust_decimal::Decimal::new(20000, 2); // 200.00

        let updated2 = dao
            .update_invoice_data(update_data2)
            .await
            .unwrap();

        // Verify version incremented again
        assert_eq!(updated2.version, 3);
        assert_eq!(
            updated2.amount,
            rust_decimal::Decimal::new(20000, 2)
        );
    }

    #[tokio::test]
    async fn test_update_invoice_data_optimistic_locking_failures() {
        let dao = create_test_dao().await;

        // Scenario A: Stale version
        let invoice1 = default_invoice();
        let id1 = invoice1.id;
        let created1 = dao
            .create_invoice(invoice1)
            .await
            .unwrap();
        assert_eq!(created1.version, 1);

        // Update status (version becomes 2)
        dao.update_invoice_status(id1, InvoiceStatus::PartiallyPaid)
            .await
            .unwrap();

        // Try update_invoice_data with stale version=1
        let update_data = default_update_invoice_data(id1);
        assert_eq!(update_data.version, 1);

        let result = dao
            .update_invoice_data(update_data)
            .await;

        // Should fail with VersionConflict (invoice exists but version/status mismatch)
        // Since status is PartiallyPaid (not Waiting), WHERE clause fails ->
        // RowNotFound -> diagnostic -> VersionConflict
        assert!(result.is_err());
        match result.unwrap_err() {
            DaoInvoiceError::VersionConflict {
                invoice_id,
                expected_version,
            } => {
                assert_eq!(invoice_id, id1);
                assert_eq!(expected_version, 1);
            },
            err => panic!("Expected VersionConflict, got: {err:?}"),
        }

        // Scenario B: Wrong status (not in Waiting state)
        let invoice2 = Invoice {
            status: InvoiceStatus::Waiting,
            ..default_invoice()
        };
        let id2 = invoice2.id;
        dao.create_invoice(invoice2)
            .await
            .unwrap();

        // Update status to Paid
        dao.update_invoice_status(id2, InvoiceStatus::Paid)
            .await
            .unwrap();

        // Try update_invoice_data (requires status='Waiting')
        let update_data2 = UpdateInvoiceData {
            id: id2,
            version: 2, // Correct version after status update
            ..default_update_invoice_data(id2)
        };

        let result2 = dao
            .update_invoice_data(update_data2)
            .await;

        // Should fail with VersionConflict (status='Paid' doesn't match WHERE clause
        // requirement of 'Waiting') This gets reported as VersionConflict
        // because the invoice exists but WHERE clause doesn't match
        assert!(result2.is_err());
        match result2.unwrap_err() {
            DaoInvoiceError::VersionConflict {
                invoice_id,
                expected_version,
            } => {
                assert_eq!(invoice_id, id2);
                assert_eq!(expected_version, 2);
            },
            err => panic!("Expected VersionConflict, got: {err:?}"),
        }

        // Scenario C: Non-existent invoice
        let update_data3 = default_update_invoice_data(Uuid::new_v4());
        let result3 = dao
            .update_invoice_data(update_data3)
            .await;

        // Should fail with NotFound
        assert!(result3.is_err());
        match result3.unwrap_err() {
            DaoInvoiceError::NotFound {
                ..
            } => { /* Expected */ },
            err => panic!("Expected NotFound, got: {err:?}"),
        }
    }

    #[tokio::test]
    async fn test_update_withdrawal_status_transitions() {
        let dao = create_test_dao().await;

        // Test transition to Completed
        let invoice1 = default_invoice();
        let id1 = invoice1.id;
        let created1 = dao
            .create_invoice(invoice1)
            .await
            .unwrap();
        assert_eq!(
            created1.withdrawal_status,
            WithdrawalStatus::Waiting
        );
        assert_eq!(created1.version, 1);

        let updated1 = dao
            .update_invoice_withdrawal_status(id1, WithdrawalStatus::Completed)
            .await
            .unwrap();

        assert_eq!(
            updated1.withdrawal_status,
            WithdrawalStatus::Completed
        );
        assert_eq!(updated1.version, 2); // Trigger incremented

        // Test transition to Failed
        let invoice2 = default_invoice();
        let id2 = invoice2.id;
        dao.create_invoice(invoice2)
            .await
            .unwrap();

        let updated2 = dao
            .update_invoice_withdrawal_status(id2, WithdrawalStatus::Failed)
            .await
            .unwrap();

        assert_eq!(
            updated2.withdrawal_status,
            WithdrawalStatus::Failed
        );

        // Test transition to Forced
        let invoice3 = default_invoice();
        let id3 = invoice3.id;
        dao.create_invoice(invoice3)
            .await
            .unwrap();

        let updated3 = dao
            .update_invoice_withdrawal_status(id3, WithdrawalStatus::Forced)
            .await
            .unwrap();

        assert_eq!(
            updated3.withdrawal_status,
            WithdrawalStatus::Forced
        );
    }

    #[tokio::test]
    async fn test_update_withdrawal_status_idempotency() {
        let dao = create_test_dao().await;

        // Create invoice with Waiting withdrawal_status
        let invoice = default_invoice();
        let invoice_id = invoice.id;
        dao.create_invoice(invoice)
            .await
            .unwrap();

        // First update: Waiting -> Completed (should succeed)
        let updated = dao
            .update_invoice_withdrawal_status(invoice_id, WithdrawalStatus::Completed)
            .await
            .unwrap();

        assert_eq!(
            updated.withdrawal_status,
            WithdrawalStatus::Completed
        );

        // Second update: Completed -> Failed (should fail - not in Waiting state)
        let result = dao
            .update_invoice_withdrawal_status(invoice_id, WithdrawalStatus::Failed)
            .await;

        // Should fail with WithdrawalConstraintViolation
        assert!(result.is_err());
        match result.unwrap_err() {
            DaoInvoiceError::WithdrawalStatusConstraintViolation {
                current_status,
                attempted_status,
            } => {
                assert_eq!(
                    current_status,
                    WithdrawalStatus::Completed
                );
                assert_eq!(
                    attempted_status,
                    WithdrawalStatus::Failed
                );
            },
            err => panic!("Expected WithdrawalConstraintViolation, got: {err:?}"),
        }

        // Verify withdrawal_status is still Completed (unchanged)
        let retrieved = dao
            .get_invoice_by_id(invoice_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            retrieved.withdrawal_status,
            WithdrawalStatus::Completed
        );

        // Try to update non-existent invoice
        let result2 = dao
            .update_invoice_withdrawal_status(
                Uuid::new_v4(),
                WithdrawalStatus::Completed,
            )
            .await;

        // Should fail with NotFound
        assert!(result2.is_err());
        match result2.unwrap_err() {
            DaoInvoiceError::NotFound {
                ..
            } => { /* Expected */ },
            err => panic!("Expected NotFound, got: {err:?}"),
        }
    }

    #[tokio::test]
    async fn test_invoice_status_transition_triggers() {
        let dao = create_test_dao().await;

        // Scenario 1: Invalid transition from Paid (final state) -> Waiting
        let invoice1 = Invoice {
            status: InvoiceStatus::Paid,
            ..default_invoice()
        };
        let id1 = invoice1.id;
        dao.create_invoice(invoice1)
            .await
            .unwrap();

        let result = dao
            .update_invoice_status(id1, InvoiceStatus::Waiting)
            .await;
        match result.unwrap_err() {
            DaoInvoiceError::StatusConstraintViolation {
                current_status,
                attempted_status,
            } => {
                assert_eq!(current_status, InvoiceStatus::Paid);
                assert_eq!(attempted_status, InvoiceStatus::Waiting);
            },
            err => panic!("Expected StatusConstraintViolation, got: {err:?}"),
        }

        // Scenario 2: Valid transition from Waiting -> Paid
        let invoice2 = Invoice {
            status: InvoiceStatus::Waiting,
            ..default_invoice()
        };
        let id2 = invoice2.id;
        dao.create_invoice(invoice2)
            .await
            .unwrap();

        let updated = dao
            .update_invoice_status(id2, InvoiceStatus::Paid)
            .await
            .unwrap();
        assert_eq!(updated.status, InvoiceStatus::Paid);

        // Scenario 3: Invalid transition from PartiallyPaid -> Waiting
        let invoice3 = Invoice {
            status: InvoiceStatus::PartiallyPaid,
            ..default_invoice()
        };
        let id3 = invoice3.id;
        dao.create_invoice(invoice3)
            .await
            .unwrap();

        let result = dao
            .update_invoice_status(id3, InvoiceStatus::Waiting)
            .await;
        match result.unwrap_err() {
            DaoInvoiceError::StatusConstraintViolation {
                current_status,
                attempted_status,
            } => {
                assert_eq!(
                    current_status,
                    InvoiceStatus::PartiallyPaid
                );
                assert_eq!(attempted_status, InvoiceStatus::Waiting);
            },
            err => panic!("Expected StatusConstraintViolation, got: {err:?}"),
        }
    }
}

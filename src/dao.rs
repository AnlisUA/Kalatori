/// ## Data Access Object (DAO) for Kalatori application.
///
/// Please follow the architectural vision for the DAO methods:
/// - Keep methods focused on single responsibilities (e.g., create, read, update). Don't implement
///   any business logic here.
/// - All creation and update methods should return the full updated object.
/// - We manually update `updated_at` and increment `version` in UPDATE statements rather than using
///   database triggers.

use names::Generator;
use sqlx::types::{Json, Text};
use uuid::Uuid;

use crate::configs::DatabaseConfig;
use crate::legacy_types::{ServerInfo, WithdrawalStatus};
use crate::types::{
    Invoice, InvoiceRow, InvoiceStatus, Transaction, TransactionRow, UpdateInvoiceData,
};

pub type DaoError = sqlx::Error;
pub type DaoResult<T> = Result<T, DaoError>;

#[expect(clippy::upper_case_acronyms)]
#[derive(Clone)]
pub struct DAO {
    pool: sqlx::SqlitePool,
}

impl DAO {
    pub async fn new(config: DatabaseConfig) -> DaoResult<Self> {
        let (pool_options, connection_options) = if config.temporary {
            tracing::info!("Using in-memory temporary database");
            let pool_opts = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1);
            let conn_opts = sqlx::sqlite::SqliteConnectOptions::new()
                .create_if_missing(true)
                .in_memory(true);
            (pool_opts, conn_opts)
        } else {
            let pool_opts = sqlx::sqlite::SqlitePoolOptions::new();
            let conn_opts = sqlx::sqlite::SqliteConnectOptions::new()
                .create_if_missing(true)
                .filename(format!("{}/kalatori_db.sqlite", config.dir));
            (pool_opts, conn_opts)
        };

        let pool = pool_options
            .connect_with(connection_options)
            .await
            .expect("Failed to create database connection pool");

        tracing::info!("Run database migrations...");
        sqlx::migrate!("./migrations").run(&pool).await?;
        tracing::info!("Database migrations done.");

        Ok(Self { pool })
    }

    pub async fn create_invoice(&self, invoice: Invoice) -> DaoResult<Invoice> {
        let invoice = sqlx::query_as::<_, InvoiceRow>(
            "INSERT INTO invoices (id, order_id, asset_id, chain, amount, payment_address, status, withdrawal_status, callback, cart, valid_till, created_at, updated_at, version)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
            .bind(invoice.valid_till)
            .bind(invoice.created_at)
            .bind(invoice.updated_at)
            .bind(invoice.version)
            .fetch_one(&self.pool)
            .await?;

        Ok(invoice.into())
    }

    pub async fn get_invoice_by_id(&self, invoice_id: Uuid) -> DaoResult<Option<Invoice>> {
        let invoice = sqlx::query_as::<_, InvoiceRow>(
            "SELECT *
             FROM invoices
             WHERE id = ?",
        )
            .bind(invoice_id)
            .fetch_optional(&self.pool)
            .await?;

        Ok(invoice.map(From::from))
    }

    pub async fn get_invoice_by_order_id(&self, order_id: &str) -> DaoResult<Option<Invoice>> {
        let invoice = sqlx::query_as::<_, InvoiceRow>(
            "SELECT *
             FROM invoices
             WHERE order_id = ?",
        )
            .bind(order_id)
            .fetch_optional(&self.pool)
            .await?;

        Ok(invoice.map(From::from))
    }

    /// Get all active invoices that need to be monitored
    /// Returns invoices with status 'Waiting' or '`PartiallyPaid`'
    pub async fn get_active_invoices(&self) -> DaoResult<Vec<Invoice>> {
        let invoices = sqlx::query_as::<_, InvoiceRow>(
            "SELECT *
             FROM invoices
             WHERE status IN ('Waiting', 'PartiallyPaid')
             ORDER BY created_at ASC",
        )
            .fetch_all(&self.pool)
            .await?;

        Ok(invoices.into_iter().map(From::from).collect())
    }

    pub async fn update_invoice_status(
        &self,
        invoice_id: Uuid,
        status: InvoiceStatus,
    ) -> DaoResult<Invoice> {
        // TODO: add status transition validation
        let result = sqlx::query_as::<_, InvoiceRow>(
            "UPDATE invoices
                SET status = ?,
                    updated_at = datetime('now'),
                    version = version + 1
                WHERE id = ?
                RETURNING *",
        )
            .bind(status)
            .bind(invoice_id)
            .fetch_one(&self.pool)
            .await?;

        Ok(result.into())
    }

    pub async fn update_invoice_data(&self, data: UpdateInvoiceData) -> DaoResult<Invoice> {
        let result = sqlx::query_as::<_, InvoiceRow>(
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
            .bind(data.version)
            .fetch_one(&self.pool)
            .await?;

        Ok(result.into())
    }

    pub async fn update_invoice_withdrawal_status(
        &self,
        invoice_id: Uuid,
        status: WithdrawalStatus,
    ) -> DaoResult<Invoice> {
        sqlx::query_as::<_, InvoiceRow>(
            "UPDATE invoices
                SET withdrawal_status = ?,
                    updated_at = datetime('now'),
                    version = version + 1
                WHERE id = ? AND withdrawal_status == 'Waiting'
                RETURNING *",
        )
            .bind(status)
            .bind(invoice_id)
            .fetch_one(&self.pool)
            .await
            .map(From::from)
    }

    pub async fn create_transaction(&self, transaction: Transaction) -> DaoResult<Transaction> {
        let transaction = sqlx::query_as::<_, TransactionRow>(
            "INSERT INTO transactions (id, invoice_id, asset_id, chain, amount, sender, recipient, block_number, position_in_block, tx_hash, origin, status, transaction_type, outgoing_meta, created_at, transaction_bytes)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             RETURNING *"
        )
            .bind(transaction.id)
            .bind(transaction.invoice_id)
            .bind(transaction.asset_id)
            .bind(&transaction.chain)
            .bind(Text(transaction.amount))
            .bind(&transaction.sender)
            .bind(&transaction.recipient)
            .bind(transaction.block_number)
            .bind(transaction.position_in_block)
            .bind(&transaction.tx_hash)
            .bind(Json(&transaction.origin))
            .bind(transaction.status)
            .bind(transaction.transaction_type)
            .bind(Json(&transaction.outgoing_meta))
            .bind(transaction.created_at)
            .bind(&transaction.transaction_bytes)
            .fetch_one(&self.pool)
            .await?;

        Ok(transaction.into())
    }

    pub async fn update_transaction(&self, transaction: Transaction) -> DaoResult<Transaction> {
        let transaction = sqlx::query_as::<_, TransactionRow>(
            "UPDATE transactions
             SET invoice_id = ?, asset_id = ?, chain = ?, amount = ?, sender = ?, recipient = ?,
                 block_number = ?, position_in_block = ?, tx_hash = ?, origin = ?, status = ?,
                 transaction_type = ?, outgoing_meta = ?, transaction_bytes = ?
             WHERE id = ?
             RETURNING *",
        )
            .bind(transaction.invoice_id)
            .bind(transaction.asset_id)
            .bind(&transaction.chain)
            .bind(Text(transaction.amount))
            .bind(&transaction.sender)
            .bind(&transaction.recipient)
            .bind(transaction.block_number)
            .bind(transaction.position_in_block)
            .bind(&transaction.tx_hash)
            .bind(Json(&transaction.origin))
            .bind(transaction.status)
            .bind(transaction.transaction_type)
            .bind(Json(&transaction.outgoing_meta))
            .bind(&transaction.transaction_bytes)
            .bind(transaction.id)
            .fetch_one(&self.pool)
            .await?;

        Ok(transaction.into())
    }

    // TODO: Implement create_transaction_outgoing when OutgoingTransaction type is defined
    // async fn create_transaction_outgoing(&self, transaction: OutgoingTransaction) -> DaoResult<Uuid> {
    //     todo!("Implement outgoing transaction creation")
    // }

    pub async fn get_invoice_transactions(&self, invoice_id: Uuid) -> DaoResult<Vec<Transaction>> {
        let transactions = sqlx::query_as::<_, TransactionRow>(
            "SELECT id, invoice_id, asset_id, chain, amount, sender, recipient, block_number, position_in_block, tx_hash, origin, status, transaction_type, outgoing_meta, created_at, transaction_bytes
             FROM transactions
             WHERE invoice_id = ?
             ORDER BY created_at ASC",
        )
            .bind(invoice_id)
            .fetch_all(&self.pool)
            .await?;

        Ok(transactions.into_iter().map(From::from).collect())
    }

    pub async fn transaction_exists_by_bytes(&self, transaction_bytes: &str) -> DaoResult<bool> {
        let result = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM transactions WHERE transaction_bytes = ?",
        )
            .bind(transaction_bytes)
            .fetch_one(&self.pool)
            .await?;

        Ok(result > 0)
    }

    /// Upsert server info (used by migration)
    /// Returns true if a new record was inserted, false if updated
    pub async fn upsert_server_info(&self, server_info: &ServerInfo) -> DaoResult<bool> {
        let rows_affected = sqlx::query(
            "INSERT INTO server_info (instance_id, version, remark) VALUES (?, ?, ?)
             ON CONFLICT(instance_id) DO UPDATE SET
                version = excluded.version,
                remark = excluded.remark",
        )
            .bind(&server_info.instance_id)
            .bind(&server_info.version)
            .bind(&server_info.kalatori_remark)
            .execute(&self.pool)
            .await?
            .rows_affected();

        // If rows_affected is 1, it was an insert; if 2, it was an update
        Ok(rows_affected == 1)
    }

    pub async fn initialize_server_info(&self) -> DaoResult<String> {
        let info = sqlx::query_as::<_, ServerInfo>(
            "SELECT instance_id, version, remark as kalatori_remark FROM server_info",
        )
            .fetch_optional(&self.pool)
            .await?;

        if let Some(server_info) = info {
            Ok(server_info.instance_id)
        } else {
            let mut generator = Generator::default();
            let new_instance_id = generator
                .next()
                .unwrap_or_else(|| "unknown-instance".to_string());

            let version = env!("CARGO_PKG_VERSION").to_string();

            let result = sqlx::query_as::<_, ServerInfo>(
                "INSERT INTO server_info (instance_id, version)
                 VALUES (?, ?)
                 RETURNING instance_id, version, kalatori_remark",
            )
            .bind(&new_instance_id)
            .bind(version)
            .fetch_one(&self.pool)
            .await?;

            Ok(result.instance_id)
        }
    }
}

#[cfg(test)]
async fn create_test_dao() -> DAO {
    use crate::configs::DatabaseConfig;

    let config = DatabaseConfig {
        path: String::new(),
        dir: String::new(),
        temporary: true,
    };

    DAO::new(config)
        .await
        .expect("Failed to create test DAO")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::legacy_types::WithdrawalStatus;
    use crate::types::{
        default_invoice, default_transaction, default_update_invoice_data, Invoice,
        InvoiceStatus, Transaction, TransactionOrigin, TransactionStatus, TransactionType,
        OutgoingTransactionMeta,
    };
    use chrono::Utc;
    use uuid::Uuid;

    #[tokio::test]
    async fn test_invoice_crud_operations() {
        let dao = create_test_dao().await;

        // Create invoice
        let invoice = default_invoice();
        let invoice_id = invoice.id;
        let order_id = invoice.order_id.clone();

        let created = dao.create_invoice(invoice).await.unwrap();

        // Verify created invoice fields
        assert_eq!(created.id, invoice_id);
        assert_eq!(created.order_id, order_id);
        assert_eq!(created.version, 1);
        assert_eq!(created.status, InvoiceStatus::Waiting);

        // Get by ID - should return Some
        let by_id = dao.get_invoice_by_id(invoice_id).await.unwrap();
        assert!(by_id.is_some());
        let by_id = by_id.unwrap();
        assert_eq!(by_id.id, invoice_id);
        assert_eq!(by_id.order_id, order_id);

        // Get by order_id - should return Some
        let by_order = dao.get_invoice_by_order_id(&order_id).await.unwrap();
        assert!(by_order.is_some());
        let by_order = by_order.unwrap();
        assert_eq!(by_order.id, invoice_id);
        assert_eq!(by_order.order_id, order_id);

        // Get by non-existent ID - should return None
        let non_existent_id = dao.get_invoice_by_id(Uuid::new_v4()).await.unwrap();
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
        dao.create_invoice(invoice1).await.unwrap();

        // Try to create second invoice with same order_id
        let invoice2 = Invoice {
            order_id,
            ..default_invoice()
        };

        let result = dao.create_invoice(invoice2).await;

        // Should fail with UNIQUE constraint error
        assert!(result.is_err());
        match result.unwrap_err() {
            sqlx::Error::Database(db_err) => {
                assert!(db_err.message().contains("UNIQUE"));
            }
            err => panic!("Expected database UNIQUE constraint error, got: {:?}", err),
        }
    }

    #[tokio::test]
    async fn test_get_active_invoices_filtering() {
        let dao = create_test_dao().await;

        // Create invoice with Waiting status (default)
        let invoice_waiting = default_invoice();
        dao.create_invoice(invoice_waiting).await.unwrap();

        // Create invoice with PartiallyPaid status
        let invoice_partial = Invoice {
            status: InvoiceStatus::PartiallyPaid,
            ..default_invoice()
        };
        dao.create_invoice(invoice_partial).await.unwrap();

        // Create invoice with Paid status
        let invoice_paid = Invoice {
            status: InvoiceStatus::Paid,
            ..default_invoice()
        };
        dao.create_invoice(invoice_paid).await.unwrap();

        // Create invoice with UnpaidExpired status
        let invoice_expired = Invoice {
            status: InvoiceStatus::UnpaidExpired,
            ..default_invoice()
        };
        dao.create_invoice(invoice_expired).await.unwrap();

        // Get active invoices
        let active = dao.get_active_invoices().await.unwrap();

        // Should only return Waiting and PartiallyPaid
        assert_eq!(active.len(), 2);
        assert!(active.iter().all(|inv| inv.status.is_active()));

        // Verify we have one Waiting and one PartiallyPaid
        let waiting_count = active
            .iter()
            .filter(|inv| inv.status == InvoiceStatus::Waiting)
            .count();
        let partial_count = active
            .iter()
            .filter(|inv| inv.status == InvoiceStatus::PartiallyPaid)
            .count();
        assert_eq!(waiting_count, 1);
        assert_eq!(partial_count, 1);
    }

    #[tokio::test]
    async fn test_get_active_invoices_ordering() {
        let dao = create_test_dao().await;

        // Create 3 invoices with Waiting status at different times
        let invoice1 = default_invoice();
        let id1 = invoice1.id;
        dao.create_invoice(invoice1).await.unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        let invoice2 = default_invoice();
        let id2 = invoice2.id;
        dao.create_invoice(invoice2).await.unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        let invoice3 = default_invoice();
        let id3 = invoice3.id;
        dao.create_invoice(invoice3).await.unwrap();

        // Get active invoices
        let active = dao.get_active_invoices().await.unwrap();

        // Should be ordered by created_at ASC (oldest first)
        assert_eq!(active.len(), 3);
        assert_eq!(active[0].id, id1);
        assert_eq!(active[1].id, id2);
        assert_eq!(active[2].id, id3);
    }

    #[tokio::test]
    async fn test_get_active_invoices_empty() {
        let dao = create_test_dao().await;

        // Query empty database
        let active = dao.get_active_invoices().await.unwrap();
        assert!(active.is_empty());

        // Create invoice with Paid status (not active)
        let invoice_paid = Invoice {
            status: InvoiceStatus::Paid,
            ..default_invoice()
        };
        dao.create_invoice(invoice_paid).await.unwrap();

        // Query again - should still be empty
        let active = dao.get_active_invoices().await.unwrap();
        assert!(active.is_empty());
    }

    #[tokio::test]
    async fn test_update_invoice_status_and_triggers() {
        let dao = create_test_dao().await;

        // Create invoice with Waiting status
        let invoice = default_invoice();
        let invoice_id = invoice.id;
        let created = dao.create_invoice(invoice).await.unwrap();

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

        // Should fail with RowNotFound
        assert!(result.is_err());
        match result.unwrap_err() {
            sqlx::Error::RowNotFound => { /* Expected */ }
            err => panic!("Expected RowNotFound, got: {:?}", err),
        }
    }

    #[tokio::test]
    async fn test_update_invoice_data_happy_path() {
        let dao = create_test_dao().await;

        // Create invoice (version=1, amount=100.00)
        let invoice = default_invoice();
        let invoice_id = invoice.id;
        let created = dao.create_invoice(invoice).await.unwrap();

        assert_eq!(created.version, 1);
        assert_eq!(created.amount, rust_decimal::Decimal::new(10000, 2));

        // Update amount to 150.00 with version=1
        let mut update_data = default_update_invoice_data(invoice_id);
        update_data.version = 1;
        let expected_cart = update_data.cart.clone();

        let updated = dao.update_invoice_data(update_data).await.unwrap();

        // Verify amount updated
        assert_eq!(updated.amount, rust_decimal::Decimal::new(15000, 2));

        // Verify version incremented
        assert_eq!(updated.version, 2);

        // Verify cart and valid_till also updated
        assert_eq!(updated.cart, expected_cart);

        // Update again with version=2
        let mut update_data2 = default_update_invoice_data(invoice_id);
        update_data2.version = 2;
        update_data2.amount = rust_decimal::Decimal::new(20000, 2); // 200.00

        let updated2 = dao.update_invoice_data(update_data2).await.unwrap();

        // Verify version incremented again
        assert_eq!(updated2.version, 3);
        assert_eq!(updated2.amount, rust_decimal::Decimal::new(20000, 2));
    }

    #[tokio::test]
    async fn test_update_invoice_data_optimistic_locking_failures() {
        let dao = create_test_dao().await;

        // Scenario A: Stale version
        let invoice1 = default_invoice();
        let id1 = invoice1.id;
        let created1 = dao.create_invoice(invoice1).await.unwrap();
        assert_eq!(created1.version, 1);

        // Update status (version becomes 2)
        dao.update_invoice_status(id1, InvoiceStatus::PartiallyPaid)
            .await
            .unwrap();

        // Try update_invoice_data with stale version=1
        let update_data = default_update_invoice_data(id1);
        assert_eq!(update_data.version, 1);

        let result = dao.update_invoice_data(update_data).await;

        // Should fail with RowNotFound
        assert!(result.is_err());
        match result.unwrap_err() {
            sqlx::Error::RowNotFound => { /* Expected */ }
            err => panic!("Expected RowNotFound, got: {:?}", err),
        }

        // Scenario B: Wrong status (not in Waiting state)
        let invoice2 = Invoice {
            status: InvoiceStatus::Waiting,
            ..default_invoice()
        };
        let id2 = invoice2.id;
        dao.create_invoice(invoice2).await.unwrap();

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

        let result2 = dao.update_invoice_data(update_data2).await;

        // Should fail with RowNotFound (status constraint)
        assert!(result2.is_err());
        match result2.unwrap_err() {
            sqlx::Error::RowNotFound => { /* Expected */ }
            err => panic!("Expected RowNotFound, got: {:?}", err),
        }

        // Scenario C: Non-existent invoice
        let update_data3 = default_update_invoice_data(Uuid::new_v4());
        let result3 = dao.update_invoice_data(update_data3).await;

        // Should fail with RowNotFound
        assert!(result3.is_err());
        match result3.unwrap_err() {
            sqlx::Error::RowNotFound => { /* Expected */ }
            err => panic!("Expected RowNotFound, got: {:?}", err),
        }
    }

    #[tokio::test]
    async fn test_update_withdrawal_status_transitions() {
        let dao = create_test_dao().await;

        // Test transition to Completed
        let invoice1 = default_invoice();
        let id1 = invoice1.id;
        let created1 = dao.create_invoice(invoice1).await.unwrap();
        assert_eq!(created1.withdrawal_status, WithdrawalStatus::Waiting);
        assert_eq!(created1.version, 1);

        let updated1 = dao
            .update_invoice_withdrawal_status(id1, WithdrawalStatus::Completed)
            .await
            .unwrap();

        assert_eq!(updated1.withdrawal_status, WithdrawalStatus::Completed);
        assert_eq!(updated1.version, 2); // Trigger incremented

        // Test transition to Failed
        let invoice2 = default_invoice();
        let id2 = invoice2.id;
        dao.create_invoice(invoice2).await.unwrap();

        let updated2 = dao
            .update_invoice_withdrawal_status(id2, WithdrawalStatus::Failed)
            .await
            .unwrap();

        assert_eq!(updated2.withdrawal_status, WithdrawalStatus::Failed);

        // Test transition to Forced
        let invoice3 = default_invoice();
        let id3 = invoice3.id;
        dao.create_invoice(invoice3).await.unwrap();

        let updated3 = dao
            .update_invoice_withdrawal_status(id3, WithdrawalStatus::Forced)
            .await
            .unwrap();

        assert_eq!(updated3.withdrawal_status, WithdrawalStatus::Forced);
    }

    #[tokio::test]
    async fn test_update_withdrawal_status_idempotency() {
        let dao = create_test_dao().await;

        // Create invoice with Waiting withdrawal_status
        let invoice = default_invoice();
        let invoice_id = invoice.id;
        dao.create_invoice(invoice).await.unwrap();

        // First update: Waiting -> Completed (should succeed)
        let updated = dao
            .update_invoice_withdrawal_status(invoice_id, WithdrawalStatus::Completed)
            .await
            .unwrap();

        assert_eq!(updated.withdrawal_status, WithdrawalStatus::Completed);

        // Second update: Completed -> Failed (should fail - not in Waiting state)
        let result = dao
            .update_invoice_withdrawal_status(invoice_id, WithdrawalStatus::Failed)
            .await;

        // Should fail with RowNotFound (WHERE withdrawal_status == 'Waiting' fails)
        assert!(result.is_err());
        match result.unwrap_err() {
            sqlx::Error::RowNotFound => { /* Expected */ }
            err => panic!("Expected RowNotFound, got: {:?}", err),
        }

        // Verify withdrawal_status is still Completed (unchanged)
        let retrieved = dao.get_invoice_by_id(invoice_id).await.unwrap().unwrap();
        assert_eq!(retrieved.withdrawal_status, WithdrawalStatus::Completed);

        // Try to update non-existent invoice
        let result2 = dao
            .update_invoice_withdrawal_status(Uuid::new_v4(), WithdrawalStatus::Completed)
            .await;

        // Should fail with RowNotFound
        assert!(result2.is_err());
        match result2.unwrap_err() {
            sqlx::Error::RowNotFound => { /* Expected */ }
            err => panic!("Expected RowNotFound, got: {:?}", err),
        }
    }

    // Transaction Tests

    #[tokio::test]
    async fn test_transaction_crud_operations() {
        let dao = create_test_dao().await;

        // Create invoice (required for FK)
        let invoice = default_invoice();
        dao.create_invoice(invoice.clone()).await.unwrap();

        // 1. Create incoming transaction
        let transaction = default_transaction(invoice.id);
        let tx_id = transaction.id;
        let created = dao.create_transaction(transaction.clone()).await.unwrap();

        // 2. Verify all fields match
        assert_eq!(created.id, tx_id);
        assert_eq!(created.invoice_id, invoice.id);
        assert_eq!(created.transaction_type, TransactionType::Incoming);
        assert_eq!(created.block_number, Some(1000)); // From default
        assert_eq!(created.status, TransactionStatus::Waiting);

        // 3. Update transaction (change status)
        let mut updated_tx = created.clone();
        updated_tx.status = TransactionStatus::Completed;
        updated_tx.tx_hash = Some("0xabcd1234".to_string());

        let updated = dao.update_transaction(updated_tx).await.unwrap();
        assert_eq!(updated.status, TransactionStatus::Completed);
        assert_eq!(updated.tx_hash, Some("0xabcd1234".to_string()));

        // 4. Get transactions for invoice
        let txs = dao.get_invoice_transactions(invoice.id).await.unwrap();
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].id, tx_id);

        // 5. Get transactions for non-existent invoice
        let empty = dao
            .get_invoice_transactions(Uuid::new_v4())
            .await
            .unwrap();
        assert!(empty.is_empty());
    }

    #[tokio::test]
    async fn test_create_transaction_types() {
        let dao = create_test_dao().await;
        let invoice = dao.create_invoice(default_invoice()).await.unwrap();

        // Create Incoming transaction
        let incoming = Transaction {
            transaction_type: TransactionType::Incoming,
            ..default_transaction(invoice.id)
        };
        let created_in = dao.create_transaction(incoming).await.unwrap();
        assert_eq!(created_in.transaction_type, TransactionType::Incoming);

        // Create Outgoing transaction
        let outgoing = Transaction {
            transaction_type: TransactionType::Outgoing,
            ..default_transaction(invoice.id)
        };
        let created_out = dao.create_transaction(outgoing).await.unwrap();
        assert_eq!(created_out.transaction_type, TransactionType::Outgoing);
    }

    #[tokio::test]
    async fn test_create_transaction_foreign_key_constraint() {
        let dao = create_test_dao().await;

        // Try to create transaction with non-existent invoice_id
        let transaction = default_transaction(Uuid::new_v4());
        let result = dao.create_transaction(transaction).await;

        // Should fail with foreign key constraint error
        assert!(result.is_err());
        match result.unwrap_err() {
            sqlx::Error::Database(db_err) => {
                assert!(db_err.message().contains("FOREIGN KEY"));
            }
            err => panic!("Expected FK constraint error, got: {:?}", err),
        }
    }

    #[tokio::test]
    async fn test_transaction_status_transitions() {
        let dao = create_test_dao().await;
        let invoice = dao.create_invoice(default_invoice()).await.unwrap();

        // Create transaction in Waiting status
        let mut tx = default_transaction(invoice.id);
        tx.status = TransactionStatus::Waiting;
        let created = dao.create_transaction(tx).await.unwrap();
        assert_eq!(created.status, TransactionStatus::Waiting);

        // Transition to InProgress
        let mut in_progress = created.clone();
        in_progress.status = TransactionStatus::InProgress;
        let updated1 = dao.update_transaction(in_progress).await.unwrap();
        assert_eq!(updated1.status, TransactionStatus::InProgress);

        // Transition to Completed
        let mut completed = updated1.clone();
        completed.status = TransactionStatus::Completed;
        let updated2 = dao.update_transaction(completed).await.unwrap();
        assert_eq!(updated2.status, TransactionStatus::Completed);

        // Test Failed status
        let mut tx_failed = default_transaction(invoice.id);
        tx_failed.status = TransactionStatus::Failed;
        let failed = dao.create_transaction(tx_failed).await.unwrap();
        assert_eq!(failed.status, TransactionStatus::Failed);
    }

    #[tokio::test]
    async fn test_transaction_json_fields() {
        let dao = create_test_dao().await;
        let invoice = dao.create_invoice(default_invoice()).await.unwrap();

        // Test TransactionOrigin with refund_id
        let origin_with_refund = TransactionOrigin {
            refund_id: Some(Uuid::new_v4()),
            payout_id: None,
            internal_transfer_id: None,
        };

        let tx_with_origin = Transaction {
            origin: origin_with_refund.clone(),
            ..default_transaction(invoice.id)
        };

        let created = dao.create_transaction(tx_with_origin).await.unwrap();
        assert_eq!(created.origin, origin_with_refund);

        // Test OutgoingTransactionMeta with metadata
        let outgoing_meta = OutgoingTransactionMeta {
            extrinsic_bytes: Some("0x123456".to_string()),
            built_at: Some(Utc::now()),
            sent_at: Some(Utc::now()),
            confirmed_at: None,
            failed_at: None,
            failure_message: None,
        };

        let tx_with_meta = Transaction {
            outgoing_meta: outgoing_meta.clone(),
            ..default_transaction(invoice.id)
        };

        let created2 = dao.create_transaction(tx_with_meta).await.unwrap();
        assert_eq!(
            created2.outgoing_meta.extrinsic_bytes,
            outgoing_meta.extrinsic_bytes
        );
    }

    #[tokio::test]
    async fn test_transaction_exists_by_bytes() {
        let dao = create_test_dao().await;
        let invoice = dao.create_invoice(default_invoice()).await.unwrap();

        // Create transaction with transaction_bytes
        let tx = Transaction {
            transaction_bytes: Some("0xdeadbeef".to_string()),
            ..default_transaction(invoice.id)
        };
        dao.create_transaction(tx).await.unwrap();

        // Check exists
        let exists = dao
            .transaction_exists_by_bytes("0xdeadbeef")
            .await
            .unwrap();
        assert!(exists);

        // Check non-existent
        let not_exists = dao
            .transaction_exists_by_bytes("0xnotfound")
            .await
            .unwrap();
        assert!(!not_exists);
    }

    #[tokio::test]
    async fn test_get_invoice_transactions_ordering() {
        let dao = create_test_dao().await;
        let invoice = dao.create_invoice(default_invoice()).await.unwrap();

        // Create 3 transactions at different times
        let tx1 = default_transaction(invoice.id);
        let id1 = tx1.id;
        dao.create_transaction(tx1).await.unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        let tx2 = default_transaction(invoice.id);
        let id2 = tx2.id;
        dao.create_transaction(tx2).await.unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        let tx3 = default_transaction(invoice.id);
        let id3 = tx3.id;
        dao.create_transaction(tx3).await.unwrap();

        // Get all transactions
        let txs = dao.get_invoice_transactions(invoice.id).await.unwrap();

        // Verify ordered by created_at ASC
        assert_eq!(txs.len(), 3);
        assert_eq!(txs[0].id, id1);
        assert_eq!(txs[1].id, id2);
        assert_eq!(txs[2].id, id3);
    }

    #[tokio::test]
    async fn test_update_transaction_not_found() {
        let dao = create_test_dao().await;
        let invoice = dao.create_invoice(default_invoice()).await.unwrap();

        // Try to update non-existent transaction
        let tx = default_transaction(invoice.id);
        let result = dao.update_transaction(tx).await;

        // Should fail with RowNotFound
        assert!(result.is_err());
        match result.unwrap_err() {
            sqlx::Error::RowNotFound => { /* Expected */ }
            err => panic!("Expected RowNotFound, got: {:?}", err),
        }
    }

    #[tokio::test]
    async fn test_transaction_nullable_fields() {
        let dao = create_test_dao().await;
        let invoice = dao.create_invoice(default_invoice()).await.unwrap();

        // Create transaction with NULL fields (pending transaction)
        let pending_tx = Transaction {
            block_number: None,
            position_in_block: None,
            tx_hash: None,
            transaction_bytes: None,
            ..default_transaction(invoice.id)
        };

        let created = dao.create_transaction(pending_tx).await.unwrap();
        assert!(created.block_number.is_none());
        assert!(created.position_in_block.is_none());
        assert!(created.tx_hash.is_none());

        // Update to finalized (add blockchain location)
        let mut finalized = created.clone();
        finalized.block_number = Some(5000);
        finalized.position_in_block = Some(3);
        finalized.tx_hash = Some("0xfinalized".to_string());

        let updated = dao.update_transaction(finalized).await.unwrap();
        assert_eq!(updated.block_number, Some(5000));
        assert_eq!(updated.position_in_block, Some(3));
    }
}

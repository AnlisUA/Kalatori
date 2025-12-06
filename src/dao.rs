/// ## Data Access Object (DAO) for Kalatori application.
///
/// Please follow the architectural vision for the DAO methods:
/// - Keep methods focused on single responsibilities (e.g., create, read,
///   update). Don't implement any business logic here.
/// - All creation and update methods should return the full updated object.
/// - We manually update `updated_at` and increment `version` in UPDATE
///   statements rather than using database triggers.
/// - We want to be able to compare datetime fields directly in SQL queries,
///   so we convert `chrono::DateTime<Utc>` to `NaiveDateTime` when binding parameters
///   (see details [here](https://docs.rs/sqlx/latest/sqlx/sqlite/types/index.html#note-current_timestamp-and-comparisoninteroperability-of-datetime-values)).
use chrono::{
    DateTime,
    Utc,
};
use names::Generator;
use sqlx::SqliteTransaction;
use sqlx::types::{
    Json,
    Text,
};
use uuid::Uuid;

use crate::chain_client::GeneralTransactionId;
use crate::configs::DatabaseConfig;
use crate::legacy_types::{
    ServerInfo,
    WithdrawalStatus,
};
use crate::types::{
    Invoice,
    InvoiceRow,
    InvoiceStatus,
    Payout,
    PayoutRow,
    PayoutStatus,
    Refund,
    RefundRow,
    RefundStatus,
    RetryMeta,
    Transaction,
    TransactionRow,
    UpdateInvoiceData,
};

pub type DaoError = sqlx::Error;
pub type DaoResult<T> = Result<T, DaoError>;

pub struct DaoTransaction {
    transaction: SqliteTransaction<'static>,
}

impl DaoTransaction {
    pub async fn commit(self) -> DaoResult<()> {
        self.transaction.commit().await
    }

    #[expect(dead_code)]
    pub async fn rollback(self) -> DaoResult<()> {
        self.transaction.rollback().await
    }
}

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
                .filename(format!(
                    "{}/kalatori_db.sqlite",
                    config.dir
                ));
            (pool_opts, conn_opts)
        };

        let pool = pool_options
            .connect_with(connection_options)
            .await
            .expect("Failed to create database connection pool");

        let dao = Self {
            pool,
        };

        let sqlite_version = dao.sqlite_version().await?;
        tracing::info!(
            "Current SQLite version: {}",
            sqlite_version
        );

        tracing::info!("Run database migrations...");

        sqlx::migrate!("./migrations")
            .run(&dao.pool)
            .await?;

        tracing::info!("Database migrations done.");

        Ok(dao)
    }

    pub async fn begin_transaction(&self) -> DaoResult<DaoTransaction> {
        let transaction = self.pool.begin().await?;
        Ok(DaoTransaction {
            transaction,
        })
    }

    pub async fn sqlite_version(&self) -> DaoResult<String> {
        let version: String = sqlx::query_scalar("SELECT sqlite_version()")
            .fetch_one(&self.pool)
            .await?;

        Ok(version)
    }

    pub async fn create_invoice(
        &self,
        invoice: Invoice,
    ) -> DaoResult<Invoice> {
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

    pub async fn get_invoice_by_id(
        &self,
        invoice_id: Uuid,
    ) -> DaoResult<Option<Invoice>> {
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

    pub async fn get_invoice_by_order_id(
        &self,
        order_id: &str,
    ) -> DaoResult<Option<Invoice>> {
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

        Ok(invoices
            .into_iter()
            .map(From::from)
            .collect())
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

    pub async fn update_invoice_data(
        &self,
        data: UpdateInvoiceData,
    ) -> DaoResult<Invoice> {
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

    pub async fn create_transaction(
        &self,
        transaction: Transaction,
    ) -> DaoResult<Transaction> {
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

    pub async fn update_transaction_successful(
        &self,
        dao_transaction: &mut DaoTransaction,
        transaction_id: Uuid,
        chain_transaction_id: GeneralTransactionId,
        confirmed_at: DateTime<Utc>,
    ) -> DaoResult<Transaction> {
        // TODO: add additional check that transaction is `Outgoing`? Check it's status?
        // TODO: add updated_at field?
        let transaction = sqlx::query_as::<_, TransactionRow>(
            "UPDATE transactions
             SET block_number = ?, position_in_block = ?, tx_hash = ?, status = 'Completed',
                 outgoing_meta = json_set(
                     outgoing_meta,
                     '$.confirmed_at', ?
                 )
             WHERE id = ?
             RETURNING *",
        )
        .bind(chain_transaction_id.block_number)
        .bind(chain_transaction_id.position_in_block)
        .bind(chain_transaction_id.hash)
        // TODO: Naive datetime does not work here for some reason, using rfc3339 string
        // It doesn't seem to be critical for now but it's quite inconsistent with other places
        .bind(confirmed_at.to_rfc3339())
        .bind(transaction_id)
        .fetch_one(&mut *dao_transaction.transaction)
        .await?;

        Ok(transaction.into())
    }

    pub async fn update_transaction_failed(
        &self,
        dao_transaction: &mut DaoTransaction,
        transaction_id: Uuid,
        chain_transaction_id: GeneralTransactionId,
        failure_message: String,
        failed_at: DateTime<Utc>,
    ) -> DaoResult<Transaction> {
        // TODO: add additional check that transaction is `Outgoing`? Check it's status?
        let transaction = sqlx::query_as::<_, TransactionRow>(
            "UPDATE transactions
             SET block_number = ?, position_in_block = ?, tx_hash = ?, status = 'Failed',
                 outgoing_meta = json_set(
                     outgoing_meta,
                     '$.failed_at', ?,
                     '$.failure_message', ?
                 )
             WHERE id = ?
             RETURNING *",
        )
        .bind(chain_transaction_id.block_number)
        .bind(chain_transaction_id.position_in_block)
        .bind(chain_transaction_id.hash)
        // TODO: Naive datetime does not work here for some reason, using rfc3339 string
        // It doesn't seem to be critical for now but it's quite inconsistent with other places
        .bind(failed_at.to_rfc3339())
        .bind(failure_message)
        .bind(transaction_id)
        .fetch_one(&mut *dao_transaction.transaction)
        .await?;

        Ok(transaction.into())
    }

    #[cfg_attr(not(test), expect(dead_code))]
    pub async fn update_transaction(
        &self,
        transaction: Transaction,
    ) -> DaoResult<Transaction> {
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

    // TODO: Implement create_transaction_outgoing when OutgoingTransaction type is
    // defined async fn create_transaction_outgoing(&self, transaction:
    // OutgoingTransaction) -> DaoResult<Uuid> {     todo!("Implement outgoing
    // transaction creation") }

    pub async fn get_invoice_transactions(
        &self,
        invoice_id: Uuid,
    ) -> DaoResult<Vec<Transaction>> {
        let transactions = sqlx::query_as::<_, TransactionRow>(
            "SELECT id, invoice_id, asset_id, chain, amount, sender, recipient, block_number, position_in_block, tx_hash, origin, status, transaction_type, outgoing_meta, created_at, transaction_bytes
             FROM transactions
             WHERE invoice_id = ?
             ORDER BY created_at ASC",
        )
            .bind(invoice_id)
            .fetch_all(&self.pool)
            .await?;

        Ok(transactions
            .into_iter()
            .map(From::from)
            .collect())
    }

    pub async fn transaction_exists_by_bytes(
        &self,
        transaction_bytes: &str,
    ) -> DaoResult<bool> {
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
    pub async fn upsert_server_info(
        &self,
        server_info: &ServerInfo,
    ) -> DaoResult<bool> {
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
                 RETURNING instance_id, version, remark as kalatori_remark",
            )
            .bind(&new_instance_id)
            .bind(version)
            .fetch_one(&self.pool)
            .await?;

            Ok(result.instance_id)
        }
    }

    // Payout methods

    pub async fn create_payout(
        &self,
        payout: Payout,
    ) -> DaoResult<Payout> {
        let payout = sqlx::query_as::<_, PayoutRow>(
            "INSERT INTO payouts (id, invoice_id, asset_id, chain, source_address, destination_address, amount, initiator_type, initiator_id, status, created_at, updated_at, retry_count, last_attempt_at, next_retry_at, failure_message)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             RETURNING *"
        )
            .bind(payout.id)
            .bind(payout.invoice_id)
            .bind(payout.transfer_info.asset_id)
            .bind(&payout.transfer_info.chain)
            .bind(&payout.transfer_info.source_address)
            .bind(&payout.transfer_info.destination_address)
            .bind(Text(payout.transfer_info.amount))
            .bind(payout.initiator_type)
            .bind(payout.initiator_id)
            .bind(payout.status)
            .bind(payout.created_at.naive_utc())
            .bind(payout.updated_at.naive_utc())
            .bind(payout.retry_meta.retry_count)
            .bind(payout.retry_meta.last_attempt_at.map(|dt| dt.naive_utc()))
            .bind(payout.retry_meta.next_retry_at.map(|dt| dt.naive_utc()))
            .bind(&payout.retry_meta.failure_message)
            .fetch_one(&self.pool)
            .await?
            .into();

        Ok(payout)
    }

    #[cfg_attr(not(test), expect(dead_code))]
    pub async fn get_payout_by_id(
        &self,
        payout_id: Uuid,
    ) -> DaoResult<Option<Payout>> {
        let payout = sqlx::query_as::<_, PayoutRow>(
            "SELECT *
             FROM payouts
             WHERE id = ?",
        )
        .bind(payout_id)
        .fetch_optional(&self.pool)
        .await?
        .map(From::from);

        Ok(payout)
    }

    /// Fetch pending payouts and mark them as `InProgress`
    // TODO: besides of Payouts it should also return associated outgoing
    // Transactions
    pub async fn get_pending_payouts(
        &self,
        limit: u32,
    ) -> DaoResult<Vec<Payout>> {
        // TODO: in future versions of sqlite (bundled in sqlx) we'll probably be able
        // to use UPDATE ... ORDER BY LIMIT directly
        let payouts = sqlx::query_as::<_, PayoutRow>(
            "WITH sel AS (
                SELECT id
                FROM payouts
                WHERE status = 'Waiting'
                    AND (next_retry_at IS NULL OR next_retry_at <= datetime('now'))
                ORDER BY created_at ASC
                LIMIT ?
            )
            UPDATE payouts
            SET status = 'InProgress',
                updated_at = datetime('now')
            WHERE id IN (SELECT id FROM sel)
            RETURNING *",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(From::from)
        .collect();

        Ok(payouts)
    }

    pub async fn update_payout_status(
        &self,
        dao_transaction: &mut DaoTransaction,
        payout_id: Uuid,
        status: PayoutStatus,
    ) -> DaoResult<Payout> {
        // TODO: add status transition validation
        let payout = sqlx::query_as::<_, PayoutRow>(
            "UPDATE payouts
             SET status = ?, updated_at = datetime('now')
             WHERE id = ?
             RETURNING *",
        )
        .bind(status)
        .bind(payout_id)
        .fetch_one(&mut *dao_transaction.transaction)
        .await?
        .into();

        Ok(payout)
    }

    pub async fn update_payout_retry(
        &self,
        dao_transaction: &mut DaoTransaction,
        payout_id: Uuid,
        retry_meta: RetryMeta,
        is_retriable: bool,
    ) -> DaoResult<Payout> {
        let status = if is_retriable {
            PayoutStatus::FailedRetriable
        } else {
            PayoutStatus::Failed
        };

        let payout = sqlx::query_as::<_, PayoutRow>(
            "UPDATE payouts
             SET retry_count = ?,
                 last_attempt_at = ?,
                 next_retry_at = ?,
                 failure_message = ?,
                 status = ?,
                 updated_at = datetime('now')
             WHERE id = ?
             RETURNING *",
        )
        .bind(retry_meta.retry_count)
        .bind(retry_meta.last_attempt_at)
        .bind(retry_meta.next_retry_at)
        .bind(&retry_meta.failure_message)
        .bind(status)
        .bind(payout_id)
        .fetch_one(&mut *dao_transaction.transaction)
        .await?
        .into();

        Ok(payout)
    }

    // Refund methods

    #[cfg_attr(not(test), expect(dead_code))]
    pub async fn create_refund(
        &self,
        refund: Refund,
    ) -> DaoResult<Refund> {
        let refund = sqlx::query_as::<_, RefundRow>(
            "INSERT INTO refunds (id, invoice_id, asset_id, chain, amount, source_address, destination_address, initiator_type, initiator_id, status, created_at, updated_at, retry_count, last_attempt_at, next_retry_at, failure_message)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             RETURNING *"
        )
            .bind(refund.id)
            .bind(refund.invoice_id)
            .bind(refund.transfer_info.asset_id)
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
            .bind(&refund.retry_meta.failure_message)
            .fetch_one(&self.pool)
            .await?;

        Ok(refund.into())
    }

    #[cfg_attr(not(test), expect(dead_code))]
    pub async fn get_refund_by_id(
        &self,
        refund_id: Uuid,
    ) -> DaoResult<Option<Refund>> {
        let refund = sqlx::query_as::<_, RefundRow>(
            "SELECT *
             FROM refunds
             WHERE id = ?",
        )
        .bind(refund_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(refund.map(From::from))
    }

    #[cfg_attr(not(test), expect(dead_code))]
    pub async fn get_pending_refunds(&self) -> DaoResult<Vec<Refund>> {
        let refunds = sqlx::query_as::<_, RefundRow>(
            "SELECT *
             FROM refunds
             WHERE status = 'Waiting'
               AND (next_retry_at IS NULL OR next_retry_at <= datetime('now'))
             ORDER BY created_at ASC",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(refunds
            .into_iter()
            .map(From::from)
            .collect())
    }

    #[cfg_attr(not(test), expect(dead_code))]
    pub async fn update_refund_status(
        &self,
        refund_id: Uuid,
        status: RefundStatus,
    ) -> DaoResult<Refund> {
        let refund = sqlx::query_as::<_, RefundRow>(
            "UPDATE refunds
             SET status = ?, updated_at = datetime('now')
             WHERE id = ?
             RETURNING *",
        )
        .bind(status)
        .bind(refund_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(refund.into())
    }

    #[cfg_attr(not(test), expect(dead_code))]
    pub async fn update_refund_retry(
        &self,
        refund_id: Uuid,
        retry_count: i32,
        last_attempt_at: chrono::DateTime<chrono::Utc>,
        next_retry_at: Option<chrono::DateTime<chrono::Utc>>,
        failure_message: Option<String>,
    ) -> DaoResult<Refund> {
        let refund = sqlx::query_as::<_, RefundRow>(
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
        .bind(refund_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(refund.into())
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
    use chrono::Utc;
    use uuid::Uuid;

    use crate::legacy_types::WithdrawalStatus;
    use crate::types::{
        Invoice,
        InvoiceStatus,
        OutgoingTransactionMeta,
        PayoutStatus,
        RefundStatus,
        RetryMeta,
        Transaction,
        TransactionOrigin,
        TransactionStatus,
        TransactionType,
        default_invoice,
        default_payout,
        default_refund,
        default_transaction,
        default_update_invoice_data,
    };

    use super::*;

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
            order_id,
            ..default_invoice()
        };

        let result = dao.create_invoice(invoice2).await;

        // Should fail with UNIQUE constraint error
        assert!(result.is_err());
        match result.unwrap_err() {
            sqlx::Error::Database(db_err) => {
                assert!(db_err.message().contains("UNIQUE"));
            },
            err => panic!("Expected database UNIQUE constraint error, got: {err:?}"),
        }
    }

    #[tokio::test]
    async fn test_get_active_invoices_filtering() {
        let dao = create_test_dao().await;

        // Create invoice with Waiting status (default)
        let invoice_waiting = default_invoice();
        dao.create_invoice(invoice_waiting)
            .await
            .unwrap();

        // Create invoice with PartiallyPaid status
        let invoice_partial = Invoice {
            status: InvoiceStatus::PartiallyPaid,
            ..default_invoice()
        };
        dao.create_invoice(invoice_partial)
            .await
            .unwrap();

        // Create invoice with Paid status
        let invoice_paid = Invoice {
            status: InvoiceStatus::Paid,
            ..default_invoice()
        };
        dao.create_invoice(invoice_paid)
            .await
            .unwrap();

        // Create invoice with UnpaidExpired status
        let invoice_expired = Invoice {
            status: InvoiceStatus::UnpaidExpired,
            ..default_invoice()
        };
        dao.create_invoice(invoice_expired)
            .await
            .unwrap();

        // Get active invoices
        let active = dao.get_active_invoices().await.unwrap();

        // Should only return Waiting and PartiallyPaid
        assert_eq!(active.len(), 2);
        assert!(
            active
                .iter()
                .all(|inv| inv.status.is_active())
        );

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
        dao.create_invoice(invoice1)
            .await
            .unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        let invoice2 = default_invoice();
        let id2 = invoice2.id;
        dao.create_invoice(invoice2)
            .await
            .unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        let invoice3 = default_invoice();
        let id3 = invoice3.id;
        dao.create_invoice(invoice3)
            .await
            .unwrap();

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
        dao.create_invoice(invoice_paid)
            .await
            .unwrap();

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

        // Should fail with RowNotFound
        assert!(result.is_err());
        match result.unwrap_err() {
            sqlx::Error::RowNotFound => { /* Expected */ },
            err => panic!("Expected RowNotFound, got: {err:?}"),
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

        // Should fail with RowNotFound
        assert!(result.is_err());
        match result.unwrap_err() {
            sqlx::Error::RowNotFound => { /* Expected */ },
            err => panic!("Expected RowNotFound, got: {err:?}"),
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

        // Should fail with RowNotFound (status constraint)
        assert!(result2.is_err());
        match result2.unwrap_err() {
            sqlx::Error::RowNotFound => { /* Expected */ },
            err => panic!("Expected RowNotFound, got: {err:?}"),
        }

        // Scenario C: Non-existent invoice
        let update_data3 = default_update_invoice_data(Uuid::new_v4());
        let result3 = dao
            .update_invoice_data(update_data3)
            .await;

        // Should fail with RowNotFound
        assert!(result3.is_err());
        match result3.unwrap_err() {
            sqlx::Error::RowNotFound => { /* Expected */ },
            err => panic!("Expected RowNotFound, got: {err:?}"),
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

        // Should fail with RowNotFound (WHERE withdrawal_status == 'Waiting' fails)
        assert!(result.is_err());
        match result.unwrap_err() {
            sqlx::Error::RowNotFound => { /* Expected */ },
            err => panic!("Expected RowNotFound, got: {err:?}"),
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

        // Should fail with RowNotFound
        assert!(result2.is_err());
        match result2.unwrap_err() {
            sqlx::Error::RowNotFound => { /* Expected */ },
            err => panic!("Expected RowNotFound, got: {err:?}"),
        }
    }

    // Transaction Tests

    #[tokio::test]
    async fn test_transaction_crud_operations() {
        let dao = create_test_dao().await;

        // Create invoice (required for FK)
        let invoice = default_invoice();
        dao.create_invoice(invoice.clone())
            .await
            .unwrap();

        // 1. Create incoming transaction
        let transaction = default_transaction(invoice.id);
        let tx_id = transaction.id;
        let created = dao
            .create_transaction(transaction.clone())
            .await
            .unwrap();

        // 2. Verify all fields match
        assert_eq!(created.id, tx_id);
        assert_eq!(created.invoice_id, invoice.id);
        assert_eq!(
            created.transaction_type,
            TransactionType::Incoming
        );
        assert_eq!(created.block_number, Some(1000)); // From default
        assert_eq!(
            created.status,
            TransactionStatus::Waiting
        );

        // 3. Update transaction (change status)
        let mut updated_tx = created.clone();
        updated_tx.status = TransactionStatus::Completed;
        updated_tx.tx_hash = Some("0xabcd1234".to_string());

        let updated = dao
            .update_transaction(updated_tx)
            .await
            .unwrap();
        assert_eq!(
            updated.status,
            TransactionStatus::Completed
        );
        assert_eq!(
            updated.tx_hash,
            Some("0xabcd1234".to_string())
        );

        // 4. Get transactions for invoice
        let txs = dao
            .get_invoice_transactions(invoice.id)
            .await
            .unwrap();
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
        let invoice = dao
            .create_invoice(default_invoice())
            .await
            .unwrap();

        // Create Incoming transaction
        let incoming = Transaction {
            transaction_type: TransactionType::Incoming,
            ..default_transaction(invoice.id)
        };
        let created_in = dao
            .create_transaction(incoming)
            .await
            .unwrap();
        assert_eq!(
            created_in.transaction_type,
            TransactionType::Incoming
        );

        // Create Outgoing transaction
        let outgoing = Transaction {
            transaction_type: TransactionType::Outgoing,
            ..default_transaction(invoice.id)
        };
        let created_out = dao
            .create_transaction(outgoing)
            .await
            .unwrap();
        assert_eq!(
            created_out.transaction_type,
            TransactionType::Outgoing
        );
    }

    #[tokio::test]
    async fn test_create_transaction_foreign_key_constraint() {
        let dao = create_test_dao().await;

        // Try to create transaction with non-existent invoice_id
        let transaction = default_transaction(Uuid::new_v4());
        let result = dao
            .create_transaction(transaction)
            .await;

        // Should fail with foreign key constraint error
        assert!(result.is_err());
        match result.unwrap_err() {
            sqlx::Error::Database(db_err) => {
                assert!(db_err.message().contains("FOREIGN KEY"));
            },
            err => panic!("Expected FK constraint error, got: {err:?}"),
        }
    }

    #[tokio::test]
    async fn test_transaction_status_transitions() {
        let dao = create_test_dao().await;
        let invoice = dao
            .create_invoice(default_invoice())
            .await
            .unwrap();

        // Create transaction in Waiting status
        let mut tx = default_transaction(invoice.id);
        tx.status = TransactionStatus::Waiting;
        let created = dao
            .create_transaction(tx)
            .await
            .unwrap();
        assert_eq!(
            created.status,
            TransactionStatus::Waiting
        );

        // Transition to InProgress
        let mut in_progress = created.clone();
        in_progress.status = TransactionStatus::InProgress;
        let updated1 = dao
            .update_transaction(in_progress)
            .await
            .unwrap();
        assert_eq!(
            updated1.status,
            TransactionStatus::InProgress
        );

        // Transition to Completed
        let mut completed = updated1.clone();
        completed.status = TransactionStatus::Completed;
        let updated2 = dao
            .update_transaction(completed)
            .await
            .unwrap();
        assert_eq!(
            updated2.status,
            TransactionStatus::Completed
        );

        // Test Failed status
        let mut tx_failed = default_transaction(invoice.id);
        tx_failed.status = TransactionStatus::Failed;
        let failed = dao
            .create_transaction(tx_failed)
            .await
            .unwrap();
        assert_eq!(failed.status, TransactionStatus::Failed);
    }

    #[tokio::test]
    async fn test_update_transaction_failed_and_successful() {
        let dao = create_test_dao().await;
        let invoice = dao
            .create_invoice(default_invoice())
            .await
            .unwrap();

        let tx = Transaction {
            block_number: None,
            position_in_block: None,
            tx_hash: None,
            ..default_transaction(invoice.id)
        };

        let created = dao
            .create_transaction(tx)
            .await
            .unwrap();

        assert!(created.block_number.is_none());
        assert!(created.position_in_block.is_none());
        assert!(created.tx_hash.is_none());

        let transaction_id = created.id;

        let chain_transaction_id = GeneralTransactionId {
            block_number: Some(123),
            position_in_block: Some(1),
            hash: None,
        };

        let mut dao_transaction1 = dao.begin_transaction().await.unwrap();
        let now1 = Utc::now();

        let updated1 = dao
            .update_transaction_failed(
                &mut dao_transaction1,
                transaction_id,
                chain_transaction_id.clone(),
                "Network error".to_string(),
                now1,
            )
            .await
            .unwrap();

        dao_transaction1.commit().await.unwrap();

        assert_eq!(updated1.block_number, Some(123));
        assert_eq!(updated1.position_in_block, Some(1));
        assert!(updated1.tx_hash.is_none());
        assert_eq!(
            updated1.status,
            TransactionStatus::Failed
        );
        assert_eq!(
            updated1.outgoing_meta.failed_at,
            Some(now1)
        );

        let mut dao_transaction2 = dao.begin_transaction().await.unwrap();
        let now2 = Utc::now();

        let updated2 = dao
            .update_transaction_successful(
                &mut dao_transaction2,
                transaction_id,
                chain_transaction_id,
                now2,
            )
            .await
            .unwrap();

        dao_transaction2.commit().await.unwrap();

        assert_eq!(updated2.block_number, Some(123));
        assert_eq!(updated2.position_in_block, Some(1));
        assert!(updated2.tx_hash.is_none());
        assert_eq!(
            updated2.status,
            TransactionStatus::Completed
        );
        assert_eq!(
            updated2.outgoing_meta.confirmed_at,
            Some(now2)
        );
    }

    #[tokio::test]
    async fn test_transaction_json_fields() {
        let dao = create_test_dao().await;
        let invoice = dao
            .create_invoice(default_invoice())
            .await
            .unwrap();

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

        let _created = dao
            .create_transaction(tx_with_origin)
            .await
            .unwrap();

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

        let created2 = dao
            .create_transaction(tx_with_meta)
            .await
            .unwrap();
        assert_eq!(
            created2.outgoing_meta.extrinsic_bytes,
            outgoing_meta.extrinsic_bytes
        );
    }

    #[tokio::test]
    async fn test_transaction_exists_by_bytes() {
        let dao = create_test_dao().await;
        let invoice = dao
            .create_invoice(default_invoice())
            .await
            .unwrap();

        // Create transaction with transaction_bytes
        let tx = Transaction {
            transaction_bytes: Some("0xdeadbeef".to_string()),
            ..default_transaction(invoice.id)
        };
        dao.create_transaction(tx)
            .await
            .unwrap();

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
        let invoice = dao
            .create_invoice(default_invoice())
            .await
            .unwrap();

        // Create 3 transactions at different times
        let tx1 = default_transaction(invoice.id);
        let id1 = tx1.id;
        dao.create_transaction(tx1)
            .await
            .unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        let tx2 = default_transaction(invoice.id);
        let id2 = tx2.id;
        dao.create_transaction(tx2)
            .await
            .unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        let tx3 = default_transaction(invoice.id);
        let id3 = tx3.id;
        dao.create_transaction(tx3)
            .await
            .unwrap();

        // Get all transactions
        let txs = dao
            .get_invoice_transactions(invoice.id)
            .await
            .unwrap();

        // Verify ordered by created_at ASC
        assert_eq!(txs.len(), 3);
        assert_eq!(txs[0].id, id1);
        assert_eq!(txs[1].id, id2);
        assert_eq!(txs[2].id, id3);
    }

    #[tokio::test]
    async fn test_update_transaction_not_found() {
        let dao = create_test_dao().await;
        let invoice = dao
            .create_invoice(default_invoice())
            .await
            .unwrap();

        // Try to update non-existent transaction
        let tx = default_transaction(invoice.id);
        let result = dao.update_transaction(tx).await;

        // Should fail with RowNotFound
        assert!(result.is_err());
        match result.unwrap_err() {
            sqlx::Error::RowNotFound => { /* Expected */ },
            err => panic!("Expected RowNotFound, got: {err:?}"),
        }
    }

    #[tokio::test]
    async fn test_transaction_nullable_fields() {
        let dao = create_test_dao().await;
        let invoice = dao
            .create_invoice(default_invoice())
            .await
            .unwrap();

        // Create transaction with NULL fields (pending transaction)
        let pending_tx = Transaction {
            block_number: None,
            position_in_block: None,
            tx_hash: None,
            transaction_bytes: None,
            ..default_transaction(invoice.id)
        };

        let created = dao
            .create_transaction(pending_tx)
            .await
            .unwrap();
        assert!(created.block_number.is_none());
        assert!(created.position_in_block.is_none());
        assert!(created.tx_hash.is_none());

        // Update to finalized (add blockchain location)
        let mut finalized = created.clone();
        finalized.block_number = Some(5000);
        finalized.position_in_block = Some(3);
        finalized.tx_hash = Some("0xfinalized".to_string());

        let updated = dao
            .update_transaction(finalized)
            .await
            .unwrap();
        assert_eq!(updated.block_number, Some(5000));
        assert_eq!(updated.position_in_block, Some(3));
    }

    // Payout tests

    #[tokio::test]
    async fn test_payout_create_and_get() {
        let dao = create_test_dao().await;
        let invoice = dao
            .create_invoice(default_invoice())
            .await
            .unwrap();

        // Create payout
        let payout = default_payout(invoice.id);
        let payout_id = payout.id;
        let created = dao
            .create_payout(payout.clone())
            .await
            .unwrap();

        // Verify fields
        assert_eq!(created, payout);

        // Get by ID
        let fetched = dao
            .get_payout_by_id(payout_id)
            .await
            .unwrap();
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap(), payout);

        // Get non-existent
        let not_found = dao
            .get_payout_by_id(Uuid::new_v4())
            .await
            .unwrap();
        assert!(not_found.is_none());
    }

    #[tokio::test]
    async fn test_get_pending_payouts_filtering() {
        let dao = create_test_dao().await;
        let invoice = dao
            .create_invoice(default_invoice())
            .await
            .unwrap();

        // Create payout with Waiting status (should be returned)
        let payout1 = default_payout(invoice.id);
        dao.create_payout(payout1)
            .await
            .unwrap();

        // Create payout with InProgress status (should NOT be returned)
        let mut payout2 = default_payout(invoice.id);
        payout2.status = PayoutStatus::InProgress;
        dao.create_payout(payout2)
            .await
            .unwrap();

        // Create payout with Completed status (should NOT be returned)
        let mut payout3 = default_payout(invoice.id);
        payout3.status = PayoutStatus::Completed;
        dao.create_payout(payout3)
            .await
            .unwrap();

        // Create payout with Waiting status but next_retry_at in future (should NOT be
        // returned)
        let mut payout4 = default_payout(invoice.id);
        payout4.retry_meta.next_retry_at = Some(Utc::now() + chrono::Duration::hours(1));
        dao.create_payout(payout4)
            .await
            .unwrap();

        // Get pending payouts
        let pending = dao
            .get_pending_payouts(2)
            .await
            .unwrap();

        // Should only return payout1 (InProgress with no next_retry_at)
        assert_eq!(pending.len(), 1);
        assert_eq!(
            pending[0].status,
            PayoutStatus::InProgress
        );
        assert_eq!(
            pending[0].retry_meta,
            RetryMeta::default()
        );

        let payout5 = Payout {
            created_at: Utc::now() - chrono::Duration::minutes(10),
            ..default_payout(invoice.id)
        };
        dao.create_payout(payout5.clone())
            .await
            .unwrap();

        let payout6 = Payout {
            created_at: Utc::now() - chrono::Duration::minutes(5),
            retry_meta: RetryMeta {
                next_retry_at: Some(Utc::now() - chrono::Duration::minutes(2)),
                ..RetryMeta::default()
            },
            ..default_payout(invoice.id)
        };
        dao.create_payout(payout6.clone())
            .await
            .unwrap();

        let payout7 = default_payout(invoice.id);
        dao.create_payout(payout7)
            .await
            .unwrap();

        let pending_all = dao
            .get_pending_payouts(2)
            .await
            .unwrap();
        assert_eq!(pending_all.len(), 2);
        assert_eq!(
            pending_all[0].status,
            PayoutStatus::InProgress
        );
        assert_eq!(
            pending_all[1].status,
            PayoutStatus::InProgress
        );
        assert_eq!(pending_all[0].id, payout5.id);
        assert_eq!(pending_all[1].id, payout6.id);
    }

    #[tokio::test]
    async fn test_update_payout_status() {
        let dao = create_test_dao().await;
        let invoice = dao
            .create_invoice(default_invoice())
            .await
            .unwrap();

        let payout = default_payout(invoice.id);
        let payout_id = payout.id;
        let created = dao.create_payout(payout).await.unwrap();
        assert_eq!(created.status, PayoutStatus::Waiting);

        // Update to InProgress
        let mut trans1 = dao.begin_transaction().await.unwrap();
        let updated = dao
            .update_payout_status(
                &mut trans1,
                payout_id,
                PayoutStatus::InProgress,
            )
            .await
            .unwrap();

        trans1.commit().await.unwrap();
        assert_eq!(updated.status, PayoutStatus::InProgress);

        // Update to Completed
        let mut trans2 = dao.begin_transaction().await.unwrap();
        let completed = dao
            .update_payout_status(
                &mut trans2,
                payout_id,
                PayoutStatus::Completed,
            )
            .await
            .unwrap();

        trans2.commit().await.unwrap();
        assert_eq!(
            completed.status,
            PayoutStatus::Completed
        );
    }

    #[tokio::test]
    async fn test_update_payout_retry() {
        let dao = create_test_dao().await;
        let invoice = dao
            .create_invoice(default_invoice())
            .await
            .unwrap();

        let payout = default_payout(invoice.id);
        let payout_id = payout.id;
        dao.create_payout(payout).await.unwrap();

        // First retry
        let mut dao_transaction = dao.begin_transaction().await.unwrap();
        let now = Utc::now();
        let next_retry = now + chrono::Duration::minutes(1);

        let retry_meta = RetryMeta {
            retry_count: 1,
            last_attempt_at: Some(now),
            next_retry_at: Some(next_retry),
            failure_message: Some("Network error".to_string()),
        };

        let updated = dao
            .update_payout_retry(
                &mut dao_transaction,
                payout_id,
                retry_meta,
                true,
            )
            .await
            .unwrap();

        dao_transaction.commit().await.unwrap();

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
            Some("Network error".to_string())
        );
        assert_eq!(
            updated.status,
            PayoutStatus::FailedRetriable
        );

        // Second retry
        let mut dao_transaction2 = dao.begin_transaction().await.unwrap();
        let now2 = Utc::now();
        let next_retry2 = now2 + chrono::Duration::minutes(5);

        let retry_meta2 = RetryMeta {
            retry_count: 2,
            last_attempt_at: Some(now2),
            next_retry_at: Some(next_retry2),
            failure_message: Some("Connection timeout".to_string()),
        };

        let updated2 = dao
            .update_payout_retry(
                &mut dao_transaction2,
                payout_id,
                retry_meta2,
                false,
            )
            .await
            .unwrap();

        dao_transaction2.commit().await.unwrap();

        assert_eq!(updated2.retry_meta.retry_count, 2);
        assert_eq!(
            updated2.retry_meta.failure_message,
            Some("Connection timeout".to_string())
        );
        assert_eq!(updated2.status, PayoutStatus::Failed);
    }

    // Refund tests

    #[tokio::test]
    async fn test_refund_create_and_get() {
        let dao = create_test_dao().await;
        let invoice = dao
            .create_invoice(default_invoice())
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
            .create_invoice(default_invoice())
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
            .create_invoice(default_invoice())
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
            .create_invoice(default_invoice())
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
}

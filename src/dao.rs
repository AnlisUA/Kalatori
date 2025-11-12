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

    pub async fn create_invoice(&self, invoice: Invoice) -> DaoResult<()> {
        sqlx::query(
            "INSERT INTO invoices (id, order_id, asset_id, chain, amount, payment_address, status, withdrawal_status, callback, cart, valid_till, created_at, updated_at, version)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
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
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    pub async fn get_invoice_by_id(&self, invoice_id: Uuid) -> DaoResult<Option<Invoice>> {
        let invoice = sqlx::query_as::<_, InvoiceRow>(
            "SELECT id, order_id, asset_id, chain, amount, payment_address, status, withdrawal_status, callback, cart, valid_till, created_at, updated_at, version
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
            "SELECT id, order_id, asset_id, chain, amount, payment_address, status, withdrawal_status, callback, cart, valid_till, created_at, updated_at, version
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
            "SELECT id, order_id, asset_id, chain, amount, payment_address, status, withdrawal_status, callback, cart, valid_till, created_at, updated_at, version
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
                SET status = ?, updated_at = CURRENT_TIMESTAMP
                WHERE id = ?
                RETURNING *",
        )
        .bind(status)
        .bind(invoice_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(result.into())
    }

    pub async fn update_invoice_data(&self, data: UpdateInvoiceData) -> DaoResult<u64> {
        let result = sqlx::query(
            "UPDATE invoices
                SET amount = ?, cart = ?, valid_till = ?
                WHERE id = ? AND status = 'Waiting' AND version = ?",
        )
        .bind(Text(data.amount))
        .bind(Json(data.cart))
        .bind(data.valid_till)
        .bind(data.id)
        .bind(data.version)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    pub async fn update_invoice_withdrawal_status(
        &self,
        invoice_id: Uuid,
        status: WithdrawalStatus,
    ) -> DaoResult<Invoice> {
        sqlx::query_as::<_, InvoiceRow>(
            "UPDATE invoices
                SET withdrawal_status = ?, updated_at = CURRENT_TIMESTAMP
                WHERE id = ? AND withdrawal_status == 'Waiting'
                RETURNING *",
        )
        .bind(status)
        .bind(invoice_id)
        .fetch_one(&self.pool)
        .await
        .map(From::from)
    }

    pub async fn create_transaction(&self, transaction: Transaction) -> DaoResult<()> {
        sqlx::query(
            "INSERT INTO transactions (id, invoice_id, asset_id, chain, amount, sender, recipient, block_number, position_in_block, tx_hash, origin, status, transaction_type, outgoing_meta, created_at, transaction_bytes)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
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
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    pub async fn update_transaction(&self, transaction: Transaction) -> DaoResult<()> {
        sqlx::query(
            "UPDATE transactions
             SET invoice_id = ?, asset_id = ?, chain = ?, amount = ?, sender = ?, recipient = ?,
                 block_number = ?, position_in_block = ?, tx_hash = ?, origin = ?, status = ?,
                 transaction_type = ?, outgoing_meta = ?, transaction_bytes = ?
             WHERE id = ?",
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
        .execute(&self.pool)
        .await?;

        Ok(())
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
            "SELECT instance_id, version, kalatori_remark FROM server_info",
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

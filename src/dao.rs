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
mod error_parsing;
mod invoice;
mod payout;
mod refund;
mod transaction;

use names::Generator;
use sqlx::{
    Executor,
    SqliteTransaction,
};
use tokio::sync::Mutex;

use crate::configs::DatabaseConfig;
use crate::legacy_types::ServerInfo;

// Export traits
pub use invoice::DaoInvoiceMethods;
pub use payout::DaoPayoutMethods;
#[expect(unused_imports)]
pub use refund::DaoRefundMethods;
pub use transaction::DaoTransactionMethods;

// Export domain-specific errors
pub use invoice::DaoInvoiceError;
#[expect(unused_imports)]
pub use payout::DaoPayoutError;
#[expect(unused_imports)]
pub use refund::DaoRefundError;
pub use transaction::DaoTransactionError;

// Keep DaoResult for internal use (DaoExecutor trait methods)
pub(crate) type DaoResult<T> = Result<T, sqlx::Error>;

pub trait DaoExecutor: Send + Sync {
    async fn fetch_optional<'a, O>(
        &self,
        query: sqlx::query::QueryAs<'a, sqlx::Sqlite, O, sqlx::sqlite::SqliteArguments<'a>>,
    ) -> Result<Option<O>, sqlx::Error>
    where
        O: for<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> + Send + Unpin + 'static,
        Self: 'static;

    async fn fetch_one<'a, O>(
        &self,
        query: sqlx::query::QueryAs<'a, sqlx::Sqlite, O, sqlx::sqlite::SqliteArguments<'a>>,
    ) -> Result<O, sqlx::Error>
    where
        O: for<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> + Send + Unpin + 'static,
        Self: 'static;

    async fn fetch_all<'a, O>(
        &self,
        query: sqlx::query::QueryAs<'a, sqlx::Sqlite, O, sqlx::sqlite::SqliteArguments<'a>>,
    ) -> Result<Vec<O>, sqlx::Error>
    where
        O: for<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> + Send + Unpin + 'static,
        Self: 'static;
}

pub struct DaoTransaction {
    // Use `Mutex` to avoid mutability requirement in order to unify the API for both Transaction
    // and Pool Use `tokio::sync::Mutex` cause `std::sync::Mutex` is not `Send`
    transaction: Mutex<SqliteTransaction<'static>>,
}

impl DaoTransaction {
    pub async fn commit(self) -> DaoResult<()> {
        let lock = self.transaction.into_inner();
        lock.commit().await
    }

    #[expect(dead_code)]
    pub async fn rollback(self) -> DaoResult<()> {
        let lock = self.transaction.into_inner();
        lock.rollback().await
    }
}

impl DaoExecutor for DaoTransaction {
    async fn fetch_optional<'a, O>(
        &self,
        query: sqlx::query::QueryAs<'a, sqlx::Sqlite, O, sqlx::sqlite::SqliteArguments<'a>>,
    ) -> Result<Option<O>, sqlx::Error>
    where
        O: for<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> + Send + Unpin + 'static,
        Self: 'static,
    {
        let mut lock = self.transaction.lock().await;
        let result = (&mut **lock)
            .fetch_optional(query)
            .await?;

        if let Some(row) = result {
            O::from_row(&row).map(Some)
        } else {
            Ok(None)
        }
    }

    async fn fetch_one<'a, O>(
        &self,
        query: sqlx::query::QueryAs<'a, sqlx::Sqlite, O, sqlx::sqlite::SqliteArguments<'a>>,
    ) -> Result<O, sqlx::Error>
    where
        O: for<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> + Send + Unpin + 'static,
        Self: 'static,
    {
        let mut lock = self.transaction.lock().await;
        (&mut **lock)
            .fetch_one(query)
            .await
            .and_then(|row| O::from_row(&row))
    }

    async fn fetch_all<'a, O>(
        &self,
        query: sqlx::query::QueryAs<'a, sqlx::Sqlite, O, sqlx::sqlite::SqliteArguments<'a>>,
    ) -> Result<Vec<O>, sqlx::Error>
    where
        O: for<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> + Send + Unpin + 'static,
        Self: 'static,
    {
        let mut lock = self.transaction.lock().await;
        (&mut **lock)
            .fetch_all(query)
            .await?
            .into_iter()
            .map(|row| O::from_row(&row))
            .collect()
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
            transaction: Mutex::new(transaction),
        })
    }

    pub async fn sqlite_version(&self) -> DaoResult<String> {
        let version: String = sqlx::query_scalar("SELECT sqlite_version()")
            .fetch_one(&self.pool)
            .await?;

        Ok(version)
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
}

impl DaoExecutor for DAO {
    async fn fetch_optional<'a, O>(
        &self,
        query: sqlx::query::QueryAs<'a, sqlx::Sqlite, O, sqlx::sqlite::SqliteArguments<'a>>,
    ) -> Result<Option<O>, sqlx::Error>
    where
        O: for<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> + Send + Unpin + 'static,
        Self: 'static,
    {
        let result = self.pool.fetch_optional(query).await?;

        if let Some(row) = result {
            O::from_row(&row).map(Some)
        } else {
            Ok(None)
        }
    }

    async fn fetch_one<'a, O>(
        &self,
        query: sqlx::query::QueryAs<'a, sqlx::Sqlite, O, sqlx::sqlite::SqliteArguments<'a>>,
    ) -> Result<O, sqlx::Error>
    where
        O: for<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> + Send + Unpin + 'static,
        Self: 'static,
    {
        self.pool
            .fetch_one(query)
            .await
            .and_then(|row| O::from_row(&row))
    }

    async fn fetch_all<'a, O>(
        &self,
        query: sqlx::query::QueryAs<'a, sqlx::Sqlite, O, sqlx::sqlite::SqliteArguments<'a>>,
    ) -> Result<Vec<O>, sqlx::Error>
    where
        O: for<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> + Send + Unpin + 'static,
        Self: 'static,
    {
        self.pool
            .fetch_all(query)
            .await?
            .into_iter()
            .map(|row| O::from_row(&row))
            .collect()
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
    use crate::types::{
        Transaction,
        default_invoice,
        default_transaction,
    };

    use super::*;

    #[tokio::test]
    async fn print_sqlite_version() {
        let dao = create_test_dao().await;
        let version = dao.sqlite_version().await.unwrap();
        println!("SQLite version: {}", version);
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
}

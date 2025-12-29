//! `Sled` to `SQLite` Migration Module
//!
//! This module provides functionality to migrate data from the legacy `sled`
//! embedded database to the new `SQLite` database schema. It handles the
//! conversion of old types to new types and preserves all data integrity during
//! the migration process.
//!
//! # Idempotency
//!
//! The migration is **fully idempotent** and can be run multiple times safely:
//! - Invoices are checked by `order_id` before insertion
//! - Transactions are checked by `transaction_bytes` before insertion
//! - Existing data is skipped and reused for mapping
//! - Statistics track both new migrations and skipped duplicates
//!
//! # Migration Process
//!
//! 1. **Server Info**: Migrate singleton server information
//! 2. **Orders → Invoices**: Convert orders to invoices with `UUID` generation
//!    and mapping
//! 3. **Finalized Transactions**: Migrate completed transactions with
//!    deduplication
//! 4. **Pending Transactions**: Migrate pending transactions with deduplication
//!
//! # Usage
//!
//! ```no_run
//! use kalatori::sled_to_sqlite_migration::migrate_sled_to_sqlite;
//! use kalatori::dao::DAO;
//! use std::path::PathBuf;
//! use std::collections::HashMap;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let sled_path = PathBuf::from("/path/to/sled/db");
//! let dao = DAO::new(config).await?;
//! let currencies = HashMap::new(); // Your currency configuration
//!
//! let stats = migrate_sled_to_sqlite(sled_path, &dao, &currencies).await?;
//! println!("Migration complete: {stats}");
//! # Ok(())
//! # }
//! ```

use std::collections::{
    HashMap,
    HashSet,
};
use std::path::PathBuf;

use chrono::{
    DateTime,
    Duration,
    Utc,
};
use codec::Decode;
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::configs::{
    ChainConfig,
    DatabaseConfig,
};
use crate::dao::{
    DAO,
    DaoInterface,
    DaoInvoiceError,
    DaoTransactionError as DaoTransactionError,
};
use crate::error::Error;
use crate::legacy_types::{
    Amount,
    BlockNumber,
    CurrencyInfo,
    CurrencyProperties,
    ExtrinsicIndex,
    OrderInfo,
    PaymentStatus,
    ServerInfo,
    Timestamp,
    TransactionInfoDb,
    TransactionInfoDbInner,
    TxKind,
    TxStatus,
    build_currencies_from_config,
};
use crate::types::{
    Invoice,
    InvoiceCart,
    InvoiceStatus,
    OutgoingTransactionMeta,
    Transaction,
    TransactionOrigin,
    TransactionStatus,
    TransactionType,
};

// Sled table names
const ORDERS_TABLE: &[u8] = b"orders";
const TRANSACTIONS_TABLE: &str = "transactions";
const PENDING_TRANSACTIONS_TABLE: &str = "pending_transactions";
const SERVER_INFO_TABLE: &[u8] = b"server_info";
const SERVER_INFO_ID: &str = "instance_id";

/// Migration error types
#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    #[error("Sled database error: {0}")]
    SledError(#[from] sled::Error),

    #[error("SCALE decoding error: {0}")]
    DecodeError(#[from] codec::Error),

    #[error("SQLite DAO error: {0}")]
    DaoError(#[from] sqlx::Error),

    #[error("Invoice DAO error: {0}")]
    InvoiceError(#[from] DaoInvoiceError),

    #[error("Transaction DAO error: {0}")]
    TransactionError(#[from] DaoTransactionError),

    #[expect(dead_code, reason = "Error variant for future use")]
    #[error("Amount conversion error: {0}")]
    AmountConversionError(String),

    #[error("Decimal conversion error: {0}")]
    DecimalConversionError(String),

    #[error("Invalid timestamp: {0}")]
    InvalidTimestamp(String),

    #[error("Missing currency info for chain: {chain}, asset_id: {asset_id:?}")]
    MissingCurrencyInfo {
        chain: String,
        asset_id: Option<u32>,
    },

    #[error("Order ID not found in mapping: {0}")]
    OrderIdNotFound(String),

    #[error("JSON serialization error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Transaction hash calculation error: {0}")]
    TxHashError(String),
}

pub type MigrationResult<T> = Result<T, MigrationError>;

/// Statistics collected during migration
#[derive(Debug, Clone, Default)]
pub struct MigrationStats {
    pub invoices_migrated: usize,
    pub invoices_skipped_existing: usize,
    pub finalized_transactions_migrated: usize,
    pub pending_transactions_migrated: usize,
    pub transactions_skipped_duplicates: usize,
    pub server_info_migrated: bool,
    pub warnings: Vec<String>,
}

impl std::fmt::Display for MigrationStats {
    fn fmt(
        &self,
        f: &mut std::fmt::Formatter<'_>,
    ) -> std::fmt::Result {
        writeln!(f, "Migration Statistics:")?;
        writeln!(
            f,
            "  Invoices migrated: {}",
            self.invoices_migrated
        )?;
        writeln!(
            f,
            "  Invoices skipped (already exist): {}",
            self.invoices_skipped_existing
        )?;
        writeln!(
            f,
            "  Finalized transactions migrated: {}",
            self.finalized_transactions_migrated
        )?;
        writeln!(
            f,
            "  Pending transactions migrated: {}",
            self.pending_transactions_migrated
        )?;
        writeln!(
            f,
            "  Transactions skipped (duplicates): {}",
            self.transactions_skipped_duplicates
        )?;
        writeln!(
            f,
            "  Server info migrated: {}",
            self.server_info_migrated
        )?;
        if !self.warnings.is_empty() {
            writeln!(
                f,
                "  Warnings ({}):",
                self.warnings.len()
            )?;
            for warning in &self.warnings {
                writeln!(f, "    - {warning}")?;
            }
        }
        Ok(())
    }
}

/// Main migration function: migrates all data from sled to `SQLite`
///
/// # Arguments
///
/// * `sled_path` - Path to the sled database directory
/// * `dao` - Reference to the `SQLite` `DAO` instance
/// * `currencies` - Map of currency configurations for validation and
///   conversion
///
/// # Returns
///
/// Migration statistics including counts of migrated entities and any warnings
///
/// # Errors
///
/// Returns [`MigrationError`] if any step of the migration fails
async fn migrate_sled_to_sqlite(
    sled_path: PathBuf,
    dao: &DAO,
    currencies: &HashMap<String, CurrencyProperties>,
) -> MigrationResult<MigrationStats> {
    tracing::info!("Starting sled to SQLite migration from path: {sled_path:?}");

    let mut stats = MigrationStats::default();

    // Open sled database
    let sled_db = sled::open(&sled_path)?;
    tracing::info!("Sled database opened successfully");

    // Step 1: Migrate server_info
    tracing::info!("Step 1/4: Migrating server info...");
    match migrate_server_info(&sled_db, dao).await {
        Ok(migrated) => {
            stats.server_info_migrated = migrated;
            tracing::info!(
                "Server info migration: {}",
                if migrated {
                    "done"
                } else {
                    "skipped (not found)"
                }
            );
        },
        Err(e) => {
            let warning = format!("Server info migration failed: {e}");
            tracing::warn!("{warning}");
            stats.warnings.push(warning);
        },
    }

    // Step 2: Migrate orders → invoices
    tracing::info!("Step 2/4: Migrating orders to invoices...");
    let invoice_mapping = migrate_orders(&sled_db, dao, currencies, &mut stats).await?;
    tracing::info!(
        "Migrated {} invoices with UUID mapping",
        stats.invoices_migrated
    );

    // Step 3: Migrate finalized transactions
    tracing::info!("Step 3/4: Migrating finalized transactions...");
    let mut migrated_tx_bytes = HashSet::new();
    migrate_finalized_transactions(
        &sled_db,
        dao,
        &invoice_mapping,
        &mut stats,
        &mut migrated_tx_bytes,
    )
    .await?;
    tracing::info!(
        "Migrated {} finalized transactions",
        stats.finalized_transactions_migrated
    );

    // Step 4: Migrate pending transactions
    tracing::info!("Step 4/4: Migrating pending transactions...");
    migrate_pending_transactions(
        &sled_db,
        dao,
        &invoice_mapping,
        &mut stats,
        &migrated_tx_bytes,
    )
    .await?;
    tracing::info!(
        "Migrated {} pending transactions",
        stats.pending_transactions_migrated
    );

    tracing::info!("Migration completed successfully");
    tracing::info!("{stats}");

    Ok(stats)
}


pub async fn perform_sled_to_sqlite_migration(
    database_config: &DatabaseConfig,
    chain_config: &ChainConfig,
    dao: &DAO,
) -> Result<(), Error> {
    // Run sled to SQLite migration if sled database exists
    if !database_config.temporary {
        let sled_path = std::path::PathBuf::from(&database_config.path);
        if sled_path.exists() {
            tracing::info!(
                "Found sled database at {:?}, running migration to SQLite...",
                sled_path
            );

            let currencies = build_currencies_from_config(chain_config);

            match migrate_sled_to_sqlite(sled_path, dao, &currencies)
                .await
            {
                Ok(stats) => {
                    tracing::info!(
                        "Migration completed successfully: {} invoices ({} skipped as existing), \
                            {} finalized transactions, {} pending transactions, {} duplicate transactions skipped, \
                            server_info migrated: {}",
                        stats.invoices_migrated,
                        stats.invoices_skipped_existing,
                        stats.finalized_transactions_migrated,
                        stats.pending_transactions_migrated,
                        stats.transactions_skipped_duplicates,
                        stats.server_info_migrated
                    );

                    if !stats.warnings.is_empty() {
                        tracing::warn!(
                            "Migration completed with {} warnings:",
                            stats.warnings.len()
                        );
                        for warning in &stats.warnings {
                            tracing::warn!("  - {}", warning);
                        }
                    }
                },
                Err(e) => {
                    return Err(Error::MigrationFailed(e));
                },
            }
        } else {
            tracing::debug!(
                "No sled database found at {:?}, skipping migration",
                sled_path
            );
        }
    }

    Ok(())
}
/// Migrate `server_info` singleton record
async fn migrate_server_info(
    sled_db: &sled::Db,
    dao: &DAO,
) -> MigrationResult<bool> {
    let server_info_tree = sled_db.open_tree(SERVER_INFO_TABLE)?;

    let Some(server_info_data) = server_info_tree.get(SERVER_INFO_ID)? else {
        tracing::debug!("No server_info found in sled database");
        return Ok(false);
    };

    let server_info: ServerInfo = serde_json::from_slice(&server_info_data)?;

    tracing::info!(
        "Migrating server info: instance_id={}, version={}",
        server_info.instance_id,
        server_info.version
    );

    // Upsert into SQLite (idempotent)
    dao.upsert_server_info(&server_info)
        .await?;

    Ok(true)
}

/// Migrate orders tree to invoices table
async fn migrate_orders(
    sled_db: &sled::Db,
    dao: &DAO,
    currencies: &HashMap<String, CurrencyProperties>,
    stats: &mut MigrationStats,
) -> MigrationResult<HashMap<String, Uuid>> {
    let orders_tree = sled_db.open_tree(ORDERS_TABLE)?;
    let mut invoice_mapping = HashMap::new();

    for item in &orders_tree {
        let (key, value) = item?;

        // Decode order_id and order_info
        let order_id = String::decode(&mut &key[..])?;
        let order_info = OrderInfo::decode(&mut &value[..])
            .inspect_err(|e| tracing::error!("ERROR HAPPENS HERE {:?}", e))?;

        // Validate currency exists
        validate_currency_exists(currencies, &order_info.currency)?;

        // Check if invoice already exists (idempotency)
        let invoice_id = if let Some(existing_invoice) = dao
            .get_invoice_by_order_id(&order_id)
            .await?
        {
            tracing::debug!("Invoice for order_id '{order_id}' already exists, skipping");
            stats.invoices_skipped_existing = stats
                .invoices_skipped_existing
                .saturating_add(1);
            existing_invoice.id
        } else {
            // Generate new UUID for this invoice
            let invoice_id = Uuid::new_v4();

            // Convert OrderInfo to Invoice
            let invoice = convert_order_to_invoice(
                order_id.clone(),
                invoice_id,
                order_info,
                stats,
            )?;

            // Insert into SQLite
            dao.create_invoice(invoice).await?;
            stats.invoices_migrated = stats
                .invoices_migrated
                .saturating_add(1);

            invoice_id
        };

        // Store mapping (whether new or existing)
        invoice_mapping.insert(order_id, invoice_id);
    }

    Ok(invoice_mapping)
}

/// Migrate finalized transactions
///
/// **Idempotent**: Checks if transaction already exists by `transaction_bytes`
/// before inserting.
async fn migrate_finalized_transactions(
    sled_db: &sled::Db,
    dao: &DAO,
    invoice_mapping: &HashMap<String, Uuid>,
    stats: &mut MigrationStats,
    migrated_tx_bytes: &mut HashSet<String>,
) -> MigrationResult<()> {
    let tx_tree = sled_db.open_tree(TRANSACTIONS_TABLE)?;

    for item in &tx_tree {
        let (key, value) = item?;

        // Decode composite key: (order_id, block_number, position_in_block)
        let (order_id, block_number, position_in_block) =
            <(String, BlockNumber, ExtrinsicIndex)>::decode(&mut &key[..])?;

        // Decode transaction data
        let tx_db = TransactionInfoDb::decode(&mut &value[..])?;

        // Deduplication check (in-memory for current run)
        if migrated_tx_bytes.contains(&tx_db.transaction_bytes) {
            stats.transactions_skipped_duplicates = stats
                .transactions_skipped_duplicates
                .saturating_add(1);
            continue;
        }

        // Idempotency check (database check for previous runs)
        if dao
            .transaction_exists_by_bytes(&tx_db.transaction_bytes)
            .await?
        {
            tracing::debug!(
                "Transaction with bytes '{}' already exists, skipping",
                &tx_db.transaction_bytes[..20.min(tx_db.transaction_bytes.len())]
            );
            stats.transactions_skipped_duplicates = stats
                .transactions_skipped_duplicates
                .saturating_add(1);
            migrated_tx_bytes.insert(tx_db.transaction_bytes);
            continue;
        }

        // Look up invoice_id
        let invoice_id = invoice_mapping
            .get(&order_id)
            .ok_or_else(|| MigrationError::OrderIdNotFound(order_id.clone()))?;

        // Store transaction_bytes for deduplication before consuming tx_db
        let tx_bytes = tx_db.transaction_bytes.clone();

        // Convert to new Transaction type
        let transaction = convert_finalized_transaction_to_new(
            *invoice_id,
            block_number,
            position_in_block,
            tx_db,
            stats,
        )?;

        // Insert into SQLite
        dao.create_transaction(transaction)
            .await?;

        // Mark as migrated
        migrated_tx_bytes.insert(tx_bytes);
        stats.finalized_transactions_migrated = stats
            .finalized_transactions_migrated
            .saturating_add(1);
    }

    Ok(())
}

/// Migrate pending transactions
///
/// **Idempotent**: Checks if transaction already exists by `transaction_bytes`
/// before inserting.
async fn migrate_pending_transactions(
    sled_db: &sled::Db,
    dao: &DAO,
    invoice_mapping: &HashMap<String, Uuid>,
    stats: &mut MigrationStats,
    migrated_tx_bytes: &HashSet<String>,
) -> MigrationResult<()> {
    let pending_tree = sled_db.open_tree(PENDING_TRANSACTIONS_TABLE)?;

    for item in &pending_tree {
        let (key, value) = item?;

        // Decode composite key: (order_id, transaction_bytes)
        let (order_id, transaction_bytes) = <(String, String)>::decode(&mut &key[..])?;

        // Deduplication check (in-memory - already migrated in finalized or this run)
        if migrated_tx_bytes.contains(&transaction_bytes) {
            stats.transactions_skipped_duplicates = stats
                .transactions_skipped_duplicates
                .saturating_add(1);
            continue;
        }

        // Idempotency check (database check for previous runs)
        if dao
            .transaction_exists_by_bytes(&transaction_bytes)
            .await?
        {
            tracing::debug!(
                "Transaction with bytes '{}' already exists, skipping",
                &transaction_bytes[..20.min(transaction_bytes.len())]
            );
            stats.transactions_skipped_duplicates = stats
                .transactions_skipped_duplicates
                .saturating_add(1);
            continue;
        }

        // Decode transaction inner data
        let tx_inner = TransactionInfoDbInner::decode(&mut &value[..])?;

        // Look up invoice_id
        let invoice_id = invoice_mapping
            .get(&order_id)
            .ok_or_else(|| MigrationError::OrderIdNotFound(order_id.clone()))?;

        // Convert to new Transaction type
        let transaction = convert_pending_transaction_to_new(
            *invoice_id,
            transaction_bytes,
            tx_inner,
            stats,
        )?;

        // Insert into SQLite
        dao.create_transaction(transaction)
            .await?;

        stats.pending_transactions_migrated = stats
            .pending_transactions_migrated
            .saturating_add(1);
    }

    Ok(())
}

// ============================================================================
// Type Conversion Functions
// ============================================================================

/// Convert old `OrderInfo` to new Invoice
fn convert_order_to_invoice(
    order_id: String,
    invoice_id: Uuid,
    order_info: OrderInfo,
    stats: &mut MigrationStats,
) -> MigrationResult<Invoice> {
    let amount_decimal = f64_to_decimal(order_info.amount)?;
    let valid_till = timestamp_to_datetime(order_info.death)?;
    let created_at = estimate_created_at(order_info.death, stats);
    let invoice_status = payment_status_to_invoice_status(&order_info.payment_status);

    Ok(Invoice {
        id: invoice_id,
        order_id,
        asset_id: order_info.currency.asset_id,
        chain: order_info.currency.chain_name,
        amount: amount_decimal,
        payment_address: order_info.payment_account,
        status: invoice_status,
        withdrawal_status: order_info.withdrawal_status,
        callback: order_info.callback,
        cart: InvoiceCart::empty(),
        redirect_url: String::new(),
        valid_till,
        created_at,
        updated_at: Utc::now(),
        version: 1,
    })
}

/// Convert finalized transaction from sled to new Transaction type
fn convert_finalized_transaction_to_new(
    invoice_id: Uuid,
    block_number: BlockNumber,
    position_in_block: ExtrinsicIndex,
    tx_db: TransactionInfoDb,
    stats: &mut MigrationStats,
) -> MigrationResult<Transaction> {
    let amount_decimal = amount_enum_to_decimal(&tx_db.inner.amount, stats)?;
    let tx_hash = extract_tx_hash(&tx_db.transaction_bytes)?;
    let created_at = parse_timestamp_or_now(
        tx_db
            .inner
            .finalized_tx_timestamp
            .as_deref(),
    );
    let tx_status = tx_status_to_transaction_status(&tx_db.inner.status);
    let tx_type = tx_kind_to_transaction_type(tx_db.inner.kind);

    let asset_id = tx_db
        .inner
        .currency
        .asset_id
        .unwrap_or_else(|| {
            stats.warnings.push(format!(
                "Transaction for invoice {invoice_id} has no asset_id, using 0"
            ));
            0
        });

    Ok(Transaction {
        id: Uuid::new_v4(),
        invoice_id,
        asset_id,
        chain: tx_db.inner.currency.chain_name,
        amount: amount_decimal,
        sender: tx_db.inner.sender,
        recipient: tx_db.inner.recipient,
        block_number: Some(block_number),
        position_in_block: Some(position_in_block),
        tx_hash: Some(tx_hash),
        origin: TransactionOrigin::default(),
        status: tx_status,
        transaction_type: tx_type,
        outgoing_meta: OutgoingTransactionMeta::default(),
        created_at,
        transaction_bytes: Some(tx_db.transaction_bytes),
    })
}

/// Convert pending transaction from sled to new Transaction type
fn convert_pending_transaction_to_new(
    invoice_id: Uuid,
    transaction_bytes: String,
    tx_inner: TransactionInfoDbInner,
    stats: &mut MigrationStats,
) -> MigrationResult<Transaction> {
    let amount_decimal = amount_enum_to_decimal(&tx_inner.amount, stats)?;
    let tx_status = TransactionStatus::Waiting;
    let tx_type = tx_kind_to_transaction_type(tx_inner.kind);

    let asset_id = tx_inner
        .currency
        .asset_id
        .unwrap_or_else(|| {
            stats.warnings.push(format!(
                "Pending transaction for invoice {invoice_id} has no asset_id, using 0"
            ));
            0
        });

    Ok(Transaction {
        id: Uuid::new_v4(),
        invoice_id,
        asset_id,
        chain: tx_inner.currency.chain_name,
        amount: amount_decimal,
        sender: tx_inner.sender,
        recipient: tx_inner.recipient,
        block_number: None,
        position_in_block: None,
        tx_hash: None,
        origin: TransactionOrigin::default(),
        status: tx_status,
        transaction_type: tx_type,
        outgoing_meta: OutgoingTransactionMeta::default(),
        created_at: Utc::now(),
        transaction_bytes: Some(transaction_bytes),
    })
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Convert f64 amount to Decimal
fn f64_to_decimal(amount: f64) -> MigrationResult<Decimal> {
    Decimal::try_from(amount).map_err(|e| {
        MigrationError::DecimalConversionError(format!(
            "Failed to convert f64 {amount} to Decimal: {e}"
        ))
    })
}

/// Convert Amount enum (All | Exact) to Decimal
fn amount_enum_to_decimal(
    amount: &Amount,
    stats: &mut MigrationStats,
) -> MigrationResult<Decimal> {
    match amount {
        Amount::Exact(val) => f64_to_decimal(*val),
        Amount::All => {
            let warning = "Encountered Amount::All in transaction, using Decimal::ZERO".to_string();
            tracing::warn!("{warning}");
            stats.warnings.push(warning);
            Ok(Decimal::ZERO)
        },
    }
}

/// Convert `Timestamp` (milliseconds) to `DateTime<Utc>`
fn timestamp_to_datetime(timestamp: Timestamp) -> MigrationResult<DateTime<Utc>> {
    #[expect(clippy::cast_possible_wrap)]
    let millis = timestamp.0 as i64;
    DateTime::from_timestamp_millis(millis).ok_or_else(|| {
        MigrationError::InvalidTimestamp(format!(
            "Invalid timestamp milliseconds: {millis}"
        ))
    })
}

/// Estimate `created_at` timestamp from `valid_till` (`death`) timestamp
/// Assumes 24 hour account lifetime as default
fn estimate_created_at(
    death: Timestamp,
    stats: &mut MigrationStats,
) -> DateTime<Utc> {
    if let Ok(valid_till) = timestamp_to_datetime(death) {
        // Subtract 24 hours (default account lifetime)
        #[expect(clippy::arithmetic_side_effects)]
        let estimated = valid_till - Duration::hours(24);
        estimated
    } else {
        let warning = format!(
            "Failed to parse death timestamp {}, using current time for created_at",
            death.0
        );
        stats.warnings.push(warning);
        Utc::now()
    }
}

/// Parse RFC3339 timestamp string or return current time
fn parse_timestamp_or_now(timestamp: Option<&str>) -> DateTime<Utc> {
    timestamp
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map_or_else(Utc::now, |dt| dt.with_timezone(&Utc))
}

/// Extract transaction hash from `transaction_bytes` using Blake2-256
fn extract_tx_hash(transaction_bytes: &str) -> MigrationResult<String> {
    let bytes_str = transaction_bytes
        .strip_prefix("0x")
        .unwrap_or(transaction_bytes);

    let bytes = const_hex::decode(bytes_str).map_err(|e| {
        MigrationError::TxHashError(format!(
            "Failed to decode transaction_bytes hex string: {e}"
        ))
    })?;

    let mut hasher = blake2b_simd::Params::new()
        .hash_length(32)
        .to_state();
    hasher.update(&bytes);
    let hash = hasher.finalize();

    Ok(format!(
        "0x{}",
        const_hex::encode(hash.as_bytes())
    ))
}

/// Convert old `TxStatus` to new `TransactionStatus`
const fn tx_status_to_transaction_status(status: &TxStatus) -> TransactionStatus {
    match status {
        TxStatus::Pending => TransactionStatus::Waiting,
        TxStatus::Finalized => TransactionStatus::Completed,
        TxStatus::Failed => TransactionStatus::Failed,
    }
}

/// Convert old `TxKind` to new `TransactionType`
const fn tx_kind_to_transaction_type(kind: TxKind) -> TransactionType {
    match kind {
        TxKind::Payment => TransactionType::Incoming,
        TxKind::Withdrawal => TransactionType::Outgoing,
    }
}

/// Convert old `PaymentStatus` to new `InvoiceStatus`
const fn payment_status_to_invoice_status(status: &PaymentStatus) -> InvoiceStatus {
    match status {
        PaymentStatus::Pending => InvoiceStatus::Waiting,
        PaymentStatus::Paid => InvoiceStatus::Paid,
    }
}

/// Validate that currency configuration exists
fn validate_currency_exists(
    currencies: &HashMap<String, CurrencyProperties>,
    currency_info: &CurrencyInfo,
) -> MigrationResult<()> {
    let exists = currencies.values().any(|props| {
        props.chain_name == currency_info.chain_name && props.asset_id == currency_info.asset_id
    });

    if exists {
        Ok(())
    } else {
        Err(MigrationError::MissingCurrencyInfo {
            chain: currency_info.chain_name.clone(),
            asset_id: currency_info.asset_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configs::DatabaseConfig;
    use crate::legacy_types::{
        Amount,
        FinalizedTxDb,
        PaymentStatus,
        TokenKind,
        TransactionInfoDb,
        TransactionInfoDbInner,
        TxKind,
        TxStatus,
        WithdrawalStatus,
    };
    use codec::Encode;
    use std::collections::HashMap;
    use tempfile::TempDir;

    // ============================================================================
    // Test Helper Functions
    // ============================================================================

    /// Create a test DAO with in-memory `SQLite` database
    async fn create_test_dao() -> DAO {
        let config = DatabaseConfig {
            path: String::new(),
            dir: String::new(),
            temporary: true,
        };
        DAO::new(config)
            .await
            .expect("Failed to create test DAO")
    }

    /// Create test currency info
    fn create_test_currency(
        name: &str,
        asset_id: Option<u32>,
    ) -> CurrencyInfo {
        CurrencyInfo {
            currency: name.to_string(),
            chain_name: "AssetHub".to_string(),
            kind: if asset_id.is_some() {
                TokenKind::Asset
            } else {
                TokenKind::Native
            },
            decimals: 10,
            rpc_url: "wss://test.example.com".to_string(),
            asset_id,
            ss58: 42,
        }
    }

    /// Create test currencies `HashMap` for validation
    fn create_test_currencies() -> HashMap<String, CurrencyProperties> {
        let mut currencies = HashMap::new();
        currencies.insert(
            "USDC".to_string(),
            CurrencyProperties {
                chain_name: "AssetHub".to_string(),
                kind: TokenKind::Asset,
                decimals: 6,
                rpc_url: "wss://test.example.com".to_string(),
                asset_id: Some(1337),
                ss58: 42,
            },
        );
        currencies.insert(
            "DOT".to_string(),
            CurrencyProperties {
                chain_name: "AssetHub".to_string(),
                kind: TokenKind::Native,
                decimals: 10,
                rpc_url: "wss://test.example.com".to_string(),
                asset_id: None,
                ss58: 0,
            },
        );
        currencies
    }

    /// Create test `OrderInfo`
    fn create_test_order(
        order_id: &str,
        amount: f64,
        currency: &CurrencyInfo,
    ) -> OrderInfo {
        OrderInfo {
            withdrawal_status: WithdrawalStatus::Waiting,
            payment_status: PaymentStatus::Pending,
            amount,
            currency: currency.clone(),
            callback: format!("http://test.com/callback/{order_id}"),
            transactions: vec![],
            payment_account: format!("payment_account_{order_id}"),
            death: Timestamp(1_700_000_000_000), // 2023-11-14
        }
    }

    /// Populate sled database with test orders
    fn populate_sled_with_orders(
        sled_db: &sled::Db,
        count: usize,
        currency: &CurrencyInfo,
    ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let orders_tree = sled_db.open_tree(ORDERS_TABLE)?;
        let mut order_ids = Vec::new();

        for i in 0..count {
            let order_id = format!("order_{i}");
            #[expect(clippy::cast_possible_truncation)]
            let order_info = create_test_order(
                &order_id,
                100.0 + f64::from(i as u32),
                currency,
            );
            orders_tree.insert(order_id.encode(), order_info.encode())?;
            order_ids.push(order_id);
        }

        Ok(order_ids)
    }

    // ============================================================================
    // Basic Type Conversion Tests (existing tests kept)
    // ============================================================================

    #[test]
    fn test_amount_enum_to_decimal() {
        let mut stats = MigrationStats::default();

        // Test Exact amount
        let exact = Amount::Exact(123.456);
        let result = amount_enum_to_decimal(&exact, &mut stats).unwrap();
        assert_eq!(result.to_string(), "123.456");

        // Test All amount (should warn)
        let all = Amount::All;
        let result_all = amount_enum_to_decimal(&all, &mut stats).unwrap();
        assert_eq!(result_all, Decimal::ZERO);
        assert_eq!(stats.warnings.len(), 1);
    }

    #[test]
    fn test_timestamp_to_datetime() {
        let timestamp = Timestamp(1_700_000_000_000); // 2023-11-14
        let datetime = timestamp_to_datetime(timestamp).unwrap();
        assert_eq!(
            datetime.timestamp_millis(),
            1_700_000_000_000
        );
    }

    #[test]
    fn test_tx_status_conversion() {
        assert_eq!(
            tx_status_to_transaction_status(&TxStatus::Pending),
            TransactionStatus::Waiting
        );
        assert_eq!(
            tx_status_to_transaction_status(&TxStatus::Finalized),
            TransactionStatus::Completed
        );
        assert_eq!(
            tx_status_to_transaction_status(&TxStatus::Failed),
            TransactionStatus::Failed
        );
    }

    #[test]
    fn test_tx_kind_conversion() {
        assert_eq!(
            tx_kind_to_transaction_type(TxKind::Payment),
            TransactionType::Incoming
        );
        assert_eq!(
            tx_kind_to_transaction_type(TxKind::Withdrawal),
            TransactionType::Outgoing
        );
    }

    #[test]
    fn test_payment_status_conversion() {
        assert_eq!(
            payment_status_to_invoice_status(&PaymentStatus::Pending),
            InvoiceStatus::Waiting
        );
        assert_eq!(
            payment_status_to_invoice_status(&PaymentStatus::Paid),
            InvoiceStatus::Paid
        );
    }

    // ============================================================================
    // Extended Type Conversion Tests
    // ============================================================================

    #[test]
    fn test_f64_to_decimal_valid() {
        assert_eq!(
            f64_to_decimal(123.456)
                .unwrap()
                .to_string(),
            "123.456"
        );
        assert_eq!(
            f64_to_decimal(0.0).unwrap().to_string(),
            "0"
        );
        assert_eq!(
            f64_to_decimal(999_999.999_999)
                .unwrap()
                .to_string(),
            "999999.999999"
        );
    }

    #[test]
    fn test_f64_to_decimal_edge_cases() {
        // NaN should error
        assert!(f64_to_decimal(f64::NAN).is_err());

        // Infinity should error
        assert!(f64_to_decimal(f64::INFINITY).is_err());
        assert!(f64_to_decimal(f64::NEG_INFINITY).is_err());
    }

    #[test]
    fn test_extract_tx_hash() {
        let tx_bytes = "0x1234567890abcdef";
        let hash = extract_tx_hash(tx_bytes).unwrap();

        // Should return Blake2-256 hash in hex format
        assert!(hash.starts_with("0x"));
        assert_eq!(hash.len(), 66); // 0x + 64 hex chars (32 bytes)
    }

    #[test]
    fn test_estimate_created_at() {
        let mut stats = MigrationStats::default();
        let death = Timestamp(1_700_000_000_000); // 2023-11-14 22:13:20 UTC

        let created_at = estimate_created_at(death, &mut stats);

        // Should be 24 hours before death
        let expected = timestamp_to_datetime(death).unwrap() - Duration::hours(24);
        assert_eq!(
            created_at.timestamp_millis(),
            expected.timestamp_millis()
        );
    }

    #[test]
    fn test_parse_timestamp_or_now() {
        // Valid RFC3339
        let valid = "2023-11-14T22:13:20Z";
        let parsed = parse_timestamp_or_now(Some(valid));
        assert_eq!(parsed.timestamp(), 1_700_000_000);

        // Invalid timestamp should return current time (just check it's recent)
        let invalid = parse_timestamp_or_now(Some("invalid"));
        assert!(invalid.timestamp() > 1_600_000_000); // After 2020

        // None should return current time
        let none_result = parse_timestamp_or_now(None);
        assert!(none_result.timestamp() > 1_600_000_000);
    }

    // ============================================================================
    // Migration Step Tests
    // ============================================================================

    #[tokio::test]
    async fn test_migrate_empty_database() {
        let temp_dir = TempDir::new().unwrap();
        let sled_path = temp_dir.path().to_path_buf();
        // Open and close sled to create empty database
        {
            let _sled_db = sled::open(&sled_path).unwrap();
        } // sled_db is dropped here, releasing the lock

        let dao = create_test_dao().await;
        let currencies = create_test_currencies();

        let stats = migrate_sled_to_sqlite(sled_path, &dao, &currencies)
            .await
            .unwrap();

        assert_eq!(stats.invoices_migrated, 0);
        assert_eq!(stats.finalized_transactions_migrated, 0);
        assert_eq!(stats.pending_transactions_migrated, 0);
    }

    #[tokio::test]
    async fn test_migrate_single_order() {
        let temp_dir = TempDir::new().unwrap();
        let sled_path = temp_dir.path().to_path_buf();

        // Create test order and close database
        {
            let sled_db = sled::open(&sled_path).unwrap();
            let currency = create_test_currency("USDC", Some(1337));
            populate_sled_with_orders(&sled_db, 1, &currency).unwrap();
        } // sled_db is dropped here, releasing the lock

        let dao = create_test_dao().await;
        let currencies = create_test_currencies();

        let stats = migrate_sled_to_sqlite(sled_path, &dao, &currencies)
            .await
            .unwrap();

        assert_eq!(stats.invoices_migrated, 1);
        assert_eq!(stats.invoices_skipped_existing, 0);

        // Verify invoice exists in database
        let invoice_option = dao
            .get_invoice_by_order_id("order_0")
            .await
            .unwrap();
        assert!(invoice_option.is_some());
        let invoice = invoice_option.unwrap();
        assert_eq!(invoice.order_id, "order_0");
        assert_eq!(invoice.amount.to_string(), "100");
    }

    #[tokio::test]
    async fn test_migrate_multiple_orders() {
        let temp_dir = TempDir::new().unwrap();
        let sled_path = temp_dir.path().to_path_buf();

        let order_ids = {
            let sled_db = sled::open(&sled_path).unwrap();
            let currency = create_test_currency("USDC", Some(1337));
            populate_sled_with_orders(&sled_db, 5, &currency).unwrap()
        }; // sled_db is dropped here

        let dao = create_test_dao().await;
        let currencies = create_test_currencies();

        let stats = migrate_sled_to_sqlite(sled_path, &dao, &currencies)
            .await
            .unwrap();

        assert_eq!(stats.invoices_migrated, 5);

        // Verify all invoices exist
        for order_id in &order_ids {
            let invoice = dao
                .get_invoice_by_order_id(order_id)
                .await
                .unwrap();
            assert!(
                invoice.is_some(),
                "Invoice for {order_id} should exist"
            );
        }
    }

    // ============================================================================
    // Idempotency Tests
    // ============================================================================

    #[tokio::test]
    async fn test_idempotent_invoices() {
        let temp_dir = TempDir::new().unwrap();
        let sled_path = temp_dir.path().to_path_buf();

        {
            let sled_db = sled::open(&sled_path).unwrap();
            let currency = create_test_currency("USDC", Some(1337));
            populate_sled_with_orders(&sled_db, 3, &currency).unwrap();
        } // sled_db is dropped here

        let dao = create_test_dao().await;
        let currencies = create_test_currencies();

        // First migration
        let stats1 = migrate_sled_to_sqlite(sled_path.clone(), &dao, &currencies)
            .await
            .unwrap();
        assert_eq!(stats1.invoices_migrated, 3);
        assert_eq!(stats1.invoices_skipped_existing, 0);

        // Second migration (should skip all)
        let stats2 = migrate_sled_to_sqlite(sled_path, &dao, &currencies)
            .await
            .unwrap();
        assert_eq!(stats2.invoices_migrated, 0);
        assert_eq!(stats2.invoices_skipped_existing, 3);
    }

    #[tokio::test]
    async fn test_idempotent_full_migration() {
        let temp_dir = TempDir::new().unwrap();
        let sled_path = temp_dir.path().to_path_buf();

        {
            let sled_db = sled::open(&sled_path).unwrap();
            let currency = create_test_currency("USDC", Some(1337));
            populate_sled_with_orders(&sled_db, 2, &currency).unwrap();
        } // sled_db is dropped here

        let dao = create_test_dao().await;
        let currencies = create_test_currencies();

        // Run migration three times
        for run in 1..=3 {
            let stats = migrate_sled_to_sqlite(sled_path.clone(), &dao, &currencies)
                .await
                .unwrap();

            if run == 1 {
                assert_eq!(
                    stats.invoices_migrated, 2,
                    "First run should migrate 2 invoices"
                );
            } else {
                assert_eq!(
                    stats.invoices_migrated, 0,
                    "Run {run} should migrate 0 invoices"
                );
                assert_eq!(
                    stats.invoices_skipped_existing, 2,
                    "Run {run} should skip 2 invoices"
                );
            }
        }
    }

    // ============================================================================
    // Edge Case Tests
    // ============================================================================

    #[tokio::test]
    async fn test_missing_currency_info() {
        let temp_dir = TempDir::new().unwrap();
        let sled_path = temp_dir.path().to_path_buf();

        {
            let sled_db = sled::open(&sled_path).unwrap();
            // Create order with currency that won't be in currencies map
            let unknown_currency = create_test_currency("UNKNOWN", Some(9999));
            populate_sled_with_orders(&sled_db, 1, &unknown_currency).unwrap();
        } // sled_db is dropped here

        let dao = create_test_dao().await;
        let currencies = create_test_currencies(); // Doesn't include UNKNOWN currency

        let result = migrate_sled_to_sqlite(sled_path, &dao, &currencies).await;

        assert!(matches!(
            result,
            Err(MigrationError::MissingCurrencyInfo { .. })
        ));
    }

    #[test]
    fn test_invalid_timestamp() {
        // Extremely large timestamp that would overflow
        let invalid = Timestamp(i64::MAX as u64);
        let result = timestamp_to_datetime(invalid);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_currency_exists_valid() {
        let currencies = create_test_currencies();
        let usdc = create_test_currency("USDC", Some(1337));

        assert!(validate_currency_exists(&currencies, &usdc).is_ok());
    }

    #[test]
    fn test_validate_currency_exists_invalid() {
        let currencies = create_test_currencies();
        let unknown = create_test_currency("UNKNOWN", Some(9999));

        assert!(matches!(
            validate_currency_exists(&currencies, &unknown),
            Err(MigrationError::MissingCurrencyInfo { .. })
        ));
    }

    // ============================================================================
    // Server Info Migration Tests
    // ============================================================================

    #[tokio::test]
    async fn test_migrate_server_info() {
        let temp_dir = TempDir::new().unwrap();
        let sled_path = temp_dir.path().to_path_buf();

        // Create server_info in sled
        {
            let sled_db = sled::open(&sled_path).unwrap();
            let server_info_tree = sled_db
                .open_tree(SERVER_INFO_TABLE)
                .unwrap();

            let server_info = ServerInfo {
                instance_id: "test-instance-123".to_string(),
                version: "0.4.1".to_string(),
                kalatori_remark: Some("test remark".to_string()),
            };

            let server_info_json = serde_json::to_vec(&server_info).unwrap();
            server_info_tree
                .insert(SERVER_INFO_ID, server_info_json)
                .unwrap();
        } // sled_db is dropped here

        let dao = create_test_dao().await;
        let currencies = create_test_currencies();

        let stats = migrate_sled_to_sqlite(sled_path, &dao, &currencies)
            .await
            .unwrap();

        // Verify server_info was migrated
        assert!(stats.server_info_migrated);

        // Note: We can't easily verify the data without adding a getter to DAO,
        // but we can verify no errors occurred and the flag was set
    }

    #[tokio::test]
    async fn test_migrate_server_info_idempotent() {
        let temp_dir = TempDir::new().unwrap();
        let sled_path = temp_dir.path().to_path_buf();

        // Create server_info in sled
        {
            let sled_db = sled::open(&sled_path).unwrap();
            let server_info_tree = sled_db
                .open_tree(SERVER_INFO_TABLE)
                .unwrap();

            let server_info = ServerInfo {
                instance_id: "test-instance-456".to_string(),
                version: "0.4.1".to_string(),
                kalatori_remark: None,
            };

            let server_info_json = serde_json::to_vec(&server_info).unwrap();
            server_info_tree
                .insert(SERVER_INFO_ID, server_info_json)
                .unwrap();
        } // sled_db is dropped here

        let dao = create_test_dao().await;
        let currencies = create_test_currencies();

        // Run migration twice
        let stats1 = migrate_sled_to_sqlite(sled_path.clone(), &dao, &currencies)
            .await
            .unwrap();
        assert!(stats1.server_info_migrated);

        let stats2 = migrate_sled_to_sqlite(sled_path, &dao, &currencies)
            .await
            .unwrap();
        // Second run should also succeed (idempotent upsert)
        assert!(stats2.server_info_migrated);
    }

    #[tokio::test]
    async fn test_migrate_no_server_info() {
        let temp_dir = TempDir::new().unwrap();
        let sled_path = temp_dir.path().to_path_buf();

        // Create empty sled database (no server_info)
        {
            let _sled_db = sled::open(&sled_path).unwrap();
        } // sled_db is dropped here

        let dao = create_test_dao().await;
        let currencies = create_test_currencies();

        let stats = migrate_sled_to_sqlite(sled_path, &dao, &currencies)
            .await
            .unwrap();

        // Should not have migrated server_info (not found)
        assert!(!stats.server_info_migrated);
    }

    // ============================================================================
    // Transaction Migration Tests
    // ============================================================================

    /// Helper to create test finalized transaction
    fn create_test_finalized_transaction(
        order_id: &str,
        block_number: BlockNumber,
        position_in_block: ExtrinsicIndex,
        amount: f64,
        currency: &CurrencyInfo,
    ) -> (
        (String, BlockNumber, ExtrinsicIndex),
        TransactionInfoDb,
    ) {
        let tx_bytes = format!(
            "0x{}",
            const_hex::encode(format!("tx_{order_id}_{block_number}").as_bytes())
        );

        let tx_db = TransactionInfoDb {
            transaction_bytes: tx_bytes.clone(),
            inner: TransactionInfoDbInner {
                finalized_tx: Some(FinalizedTxDb {
                    block_number,
                    position_in_block,
                }),
                finalized_tx_timestamp: Some("2023-11-14T22:13:20Z".to_string()),
                sender: format!("sender_{order_id}"),
                recipient: format!("recipient_{order_id}"),
                amount: Amount::Exact(amount),
                currency: currency.clone(),
                status: TxStatus::Finalized,
                kind: TxKind::Payment,
            },
        };

        let key = (
            order_id.to_string(),
            block_number,
            position_in_block,
        );
        (key, tx_db)
    }

    /// Helper to create test pending transaction
    fn create_test_pending_transaction(
        order_id: &str,
        tx_id: usize,
        amount: f64,
        currency: &CurrencyInfo,
    ) -> ((String, String), TransactionInfoDbInner) {
        let tx_bytes = format!(
            "0x{}",
            const_hex::encode(format!("pending_tx_{order_id}_{tx_id}").as_bytes())
        );

        let tx_inner = TransactionInfoDbInner {
            finalized_tx: None,
            finalized_tx_timestamp: None,
            sender: format!("sender_{order_id}"),
            recipient: format!("recipient_{order_id}"),
            amount: Amount::Exact(amount),
            currency: currency.clone(),
            status: TxStatus::Pending,
            kind: TxKind::Withdrawal,
        };

        let key = (order_id.to_string(), tx_bytes);
        (key, tx_inner)
    }

    #[tokio::test]
    async fn test_migrate_finalized_transactions() {
        let temp_dir = TempDir::new().unwrap();
        let sled_path = temp_dir.path().to_path_buf();

        let currency = create_test_currency("USDC", Some(1337));

        // Create test data
        {
            let sled_db = sled::open(&sled_path).unwrap();

            // Create order
            populate_sled_with_orders(&sled_db, 1, &currency).unwrap();

            // Create finalized transaction
            let tx_tree = sled_db
                .open_tree(TRANSACTIONS_TABLE)
                .unwrap();
            let (key, tx_db) =
                create_test_finalized_transaction("order_0", 1000, 5, 100.0, &currency);
            tx_tree
                .insert(key.encode(), tx_db.encode())
                .unwrap();
        } // sled_db is dropped here

        let dao = create_test_dao().await;
        let currencies = create_test_currencies();

        let stats = migrate_sled_to_sqlite(sled_path, &dao, &currencies)
            .await
            .unwrap();

        // Verify statistics
        assert_eq!(stats.invoices_migrated, 1);
        assert_eq!(stats.finalized_transactions_migrated, 1);
        assert_eq!(stats.pending_transactions_migrated, 0);

        // Verify transaction exists in database
        // Note: We'd need a getter in DAO to fully verify, but we can check it
        // doesn't error
    }

    #[tokio::test]
    async fn test_migrate_pending_transactions() {
        let temp_dir = TempDir::new().unwrap();
        let sled_path = temp_dir.path().to_path_buf();

        let currency = create_test_currency("USDC", Some(1337));

        // Create test data
        {
            let sled_db = sled::open(&sled_path).unwrap();

            // Create order
            populate_sled_with_orders(&sled_db, 1, &currency).unwrap();

            // Create pending transaction
            let pending_tree = sled_db
                .open_tree(PENDING_TRANSACTIONS_TABLE)
                .unwrap();
            let (key, tx_inner) = create_test_pending_transaction("order_0", 1, 50.0, &currency);
            pending_tree
                .insert(key.encode(), tx_inner.encode())
                .unwrap();
        } // sled_db is dropped here

        let dao = create_test_dao().await;
        let currencies = create_test_currencies();

        let stats = migrate_sled_to_sqlite(sled_path, &dao, &currencies)
            .await
            .unwrap();

        // Verify statistics
        assert_eq!(stats.invoices_migrated, 1);
        assert_eq!(stats.finalized_transactions_migrated, 0);
        assert_eq!(stats.pending_transactions_migrated, 1);
    }

    #[tokio::test]
    async fn test_migrate_multiple_transactions() {
        let temp_dir = TempDir::new().unwrap();
        let sled_path = temp_dir.path().to_path_buf();

        let currency = create_test_currency("USDC", Some(1337));

        // Create test data with multiple transactions per order
        {
            let sled_db = sled::open(&sled_path).unwrap();

            // Create 2 orders
            populate_sled_with_orders(&sled_db, 2, &currency).unwrap();

            // Create finalized transactions
            let tx_tree = sled_db
                .open_tree(TRANSACTIONS_TABLE)
                .unwrap();

            // Order 0: 2 finalized transactions
            let (key1, tx1) =
                create_test_finalized_transaction("order_0", 1000, 1, 50.0, &currency);
            tx_tree
                .insert(key1.encode(), tx1.encode())
                .unwrap();

            let (key2, tx2) =
                create_test_finalized_transaction("order_0", 1001, 2, 50.0, &currency);
            tx_tree
                .insert(key2.encode(), tx2.encode())
                .unwrap();

            // Order 1: 1 finalized transaction
            let (key3, tx3) =
                create_test_finalized_transaction("order_1", 1002, 1, 100.0, &currency);
            tx_tree
                .insert(key3.encode(), tx3.encode())
                .unwrap();

            // Create pending transactions
            let pending_tree = sled_db
                .open_tree(PENDING_TRANSACTIONS_TABLE)
                .unwrap();

            // Order 0: 1 pending transaction
            let (key4, tx4) = create_test_pending_transaction("order_0", 1, 25.0, &currency);
            pending_tree
                .insert(key4.encode(), tx4.encode())
                .unwrap();
        } // sled_db is dropped here

        let dao = create_test_dao().await;
        let currencies = create_test_currencies();

        let stats = migrate_sled_to_sqlite(sled_path, &dao, &currencies)
            .await
            .unwrap();

        // Verify statistics
        assert_eq!(stats.invoices_migrated, 2);
        assert_eq!(stats.finalized_transactions_migrated, 3);
        assert_eq!(stats.pending_transactions_migrated, 1);
        assert_eq!(stats.transactions_skipped_duplicates, 0);
    }

    #[tokio::test]
    async fn test_idempotent_transactions() {
        let temp_dir = TempDir::new().unwrap();
        let sled_path = temp_dir.path().to_path_buf();

        let currency = create_test_currency("USDC", Some(1337));

        // Create test data
        {
            let sled_db = sled::open(&sled_path).unwrap();

            // Create order
            populate_sled_with_orders(&sled_db, 1, &currency).unwrap();

            // Create finalized transaction
            let tx_tree = sled_db
                .open_tree(TRANSACTIONS_TABLE)
                .unwrap();
            let (key, tx_db) =
                create_test_finalized_transaction("order_0", 1000, 5, 100.0, &currency);
            tx_tree
                .insert(key.encode(), tx_db.encode())
                .unwrap();

            // Create pending transaction
            let pending_tree = sled_db
                .open_tree(PENDING_TRANSACTIONS_TABLE)
                .unwrap();
            let (key2, tx_inner) = create_test_pending_transaction("order_0", 1, 50.0, &currency);
            pending_tree
                .insert(key2.encode(), tx_inner.encode())
                .unwrap();
        } // sled_db is dropped here

        let dao = create_test_dao().await;
        let currencies = create_test_currencies();

        // First migration
        let stats1 = migrate_sled_to_sqlite(sled_path.clone(), &dao, &currencies)
            .await
            .unwrap();
        assert_eq!(
            stats1.finalized_transactions_migrated,
            1
        );
        assert_eq!(stats1.pending_transactions_migrated, 1);
        assert_eq!(
            stats1.transactions_skipped_duplicates,
            0
        );

        // Second migration (should skip all transactions)
        let stats2 = migrate_sled_to_sqlite(sled_path, &dao, &currencies)
            .await
            .unwrap();
        assert_eq!(
            stats2.finalized_transactions_migrated,
            0
        );
        assert_eq!(stats2.pending_transactions_migrated, 0);
        assert_eq!(
            stats2.transactions_skipped_duplicates, 2,
            "Both transactions should be skipped as duplicates"
        );
    }

    #[tokio::test]
    async fn test_transaction_deduplication() {
        let temp_dir = TempDir::new().unwrap();
        let sled_path = temp_dir.path().to_path_buf();

        let currency = create_test_currency("USDC", Some(1337));

        // Create test data with same transaction in both finalized and pending
        {
            let sled_db = sled::open(&sled_path).unwrap();

            // Create order
            populate_sled_with_orders(&sled_db, 1, &currency).unwrap();

            // Same transaction bytes for both finalized and pending
            let tx_bytes = "0xdeadbeef";

            // Create finalized transaction
            let tx_tree = sled_db
                .open_tree(TRANSACTIONS_TABLE)
                .unwrap();
            let tx_db = TransactionInfoDb {
                transaction_bytes: tx_bytes.to_string(),
                inner: TransactionInfoDbInner {
                    finalized_tx: Some(FinalizedTxDb {
                        block_number: 1000,
                        position_in_block: 5,
                    }),
                    finalized_tx_timestamp: Some("2023-11-14T22:13:20Z".to_string()),
                    sender: "sender_0".to_string(),
                    recipient: "recipient_0".to_string(),
                    amount: Amount::Exact(100.0),
                    currency: currency.clone(),
                    status: TxStatus::Finalized,
                    kind: TxKind::Payment,
                },
            };
            let key = ("order_0".to_string(), 1000u32, 5u32);
            tx_tree
                .insert(key.encode(), tx_db.encode())
                .unwrap();

            // Create pending transaction with SAME transaction_bytes
            let pending_tree = sled_db
                .open_tree(PENDING_TRANSACTIONS_TABLE)
                .unwrap();
            let tx_inner = TransactionInfoDbInner {
                finalized_tx: None,
                finalized_tx_timestamp: None,
                sender: "sender_0".to_string(),
                recipient: "recipient_0".to_string(),
                amount: Amount::Exact(100.0),
                currency: currency.clone(),
                status: TxStatus::Pending,
                kind: TxKind::Payment,
            };
            let key2 = (
                "order_0".to_string(),
                tx_bytes.to_string(),
            );
            pending_tree
                .insert(key2.encode(), tx_inner.encode())
                .unwrap();
        } // sled_db is dropped here

        let dao = create_test_dao().await;
        let currencies = create_test_currencies();

        let stats = migrate_sled_to_sqlite(sled_path, &dao, &currencies)
            .await
            .unwrap();

        // Should migrate finalized transaction, skip pending duplicate
        assert_eq!(stats.finalized_transactions_migrated, 1);
        assert_eq!(stats.pending_transactions_migrated, 0);
        assert_eq!(
            stats.transactions_skipped_duplicates, 1,
            "Pending transaction should be skipped as duplicate"
        );
    }

    #[tokio::test]
    async fn test_transaction_without_order() {
        let temp_dir = TempDir::new().unwrap();
        let sled_path = temp_dir.path().to_path_buf();

        let currency = create_test_currency("USDC", Some(1337));

        // Create transaction WITHOUT corresponding order
        {
            let sled_db = sled::open(&sled_path).unwrap();

            // DON'T create order, but create transaction
            let tx_tree = sled_db
                .open_tree(TRANSACTIONS_TABLE)
                .unwrap();
            let (key, tx_db) = create_test_finalized_transaction(
                "nonexistent_order",
                1000,
                5,
                100.0,
                &currency,
            );
            tx_tree
                .insert(key.encode(), tx_db.encode())
                .unwrap();
        } // sled_db is dropped here

        let dao = create_test_dao().await;
        let currencies = create_test_currencies();

        // Should fail because order_id not found in mapping
        let result = migrate_sled_to_sqlite(sled_path, &dao, &currencies).await;
        assert!(result.is_err());

        if let Err(MigrationError::OrderIdNotFound(order_id)) = result {
            assert_eq!(order_id, "nonexistent_order");
        } else {
            panic!("Expected OrderIdNotFound error");
        }
    }

    #[tokio::test]
    async fn test_transaction_with_amount_all() {
        let temp_dir = TempDir::new().unwrap();
        let sled_path = temp_dir.path().to_path_buf();

        let currency = create_test_currency("USDC", Some(1337));

        // Create transaction with Amount::All
        {
            let sled_db = sled::open(&sled_path).unwrap();

            // Create order
            populate_sled_with_orders(&sled_db, 1, &currency).unwrap();

            // Create transaction with Amount::All
            let tx_tree = sled_db
                .open_tree(TRANSACTIONS_TABLE)
                .unwrap();
            let tx_db = TransactionInfoDb {
                transaction_bytes: "0xcafebabe".to_string(),
                inner: TransactionInfoDbInner {
                    finalized_tx: Some(FinalizedTxDb {
                        block_number: 1000,
                        position_in_block: 5,
                    }),
                    finalized_tx_timestamp: Some("2023-11-14T22:13:20Z".to_string()),
                    sender: "sender_0".to_string(),
                    recipient: "recipient_0".to_string(),
                    amount: Amount::All, // Special case!
                    currency: currency.clone(),
                    status: TxStatus::Finalized,
                    kind: TxKind::Withdrawal,
                },
            };
            let key = ("order_0".to_string(), 1000u32, 5u32);
            tx_tree
                .insert(key.encode(), tx_db.encode())
                .unwrap();
        } // sled_db is dropped here

        let dao = create_test_dao().await;
        let currencies = create_test_currencies();

        let stats = migrate_sled_to_sqlite(sled_path, &dao, &currencies)
            .await
            .unwrap();

        // Should succeed with warning
        assert_eq!(stats.finalized_transactions_migrated, 1);
        assert!(
            !stats.warnings.is_empty(),
            "Should have warning about Amount::All"
        );
        assert!(
            stats
                .warnings
                .iter()
                .any(|w| w.contains("Amount::All"))
        );
    }

    // ============================================================================
    // Statistics Tests
    // ============================================================================

    #[tokio::test]
    async fn test_migration_stats_accuracy() {
        let temp_dir = TempDir::new().unwrap();
        let sled_path = temp_dir.path().to_path_buf();

        {
            let sled_db = sled::open(&sled_path).unwrap();
            let currency = create_test_currency("USDC", Some(1337));
            populate_sled_with_orders(&sled_db, 7, &currency).unwrap();
        } // sled_db is dropped here

        let dao = create_test_dao().await;
        let currencies = create_test_currencies();

        let stats = migrate_sled_to_sqlite(sled_path, &dao, &currencies)
            .await
            .unwrap();

        // Verify statistics match actual data
        assert_eq!(stats.invoices_migrated, 7);
        assert_eq!(stats.invoices_skipped_existing, 0);
        assert_eq!(stats.finalized_transactions_migrated, 0);
        assert_eq!(stats.pending_transactions_migrated, 0);
        assert_eq!(stats.transactions_skipped_duplicates, 0);
    }

    #[test]
    fn test_migration_stats_display() {
        let stats = MigrationStats {
            invoices_migrated: 10,
            invoices_skipped_existing: 5,
            finalized_transactions_migrated: 20,
            pending_transactions_migrated: 3,
            transactions_skipped_duplicates: 2,
            server_info_migrated: true,
            warnings: vec!["Test warning".to_string()],
        };

        let display = format!("{stats}");

        // Verify display format includes all fields
        assert!(display.contains("Invoices migrated: 10"));
        assert!(display.contains("Invoices skipped (already exist): 5"));
        assert!(display.contains("Finalized transactions migrated: 20"));
        assert!(display.contains("Pending transactions migrated: 3"));
        assert!(display.contains("Transactions skipped (duplicates): 2"));
        assert!(display.contains("Server info migrated: true"));
        assert!(display.contains("Warnings (1)"));
        assert!(display.contains("Test warning"));
    }
}

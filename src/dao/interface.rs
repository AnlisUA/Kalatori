//! High-level DAO interface traits for easy mocking and dependency injection.
//!
//! This module provides two main traits:
//! - `DaoInterface`: For regular DAO operations
//! - `DaoTransactionInterface`: For transactional operations
//!
//! Both traits can be easily mocked using mockall's `#[automock]` attribute.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::chain_client::GeneralTransactionId;
use crate::legacy_types::WithdrawalStatus;
use crate::types::*;

use super::invoice::{DaoInvoiceError, DaoInvoiceMethods};
use super::payout::{DaoPayoutError, DaoPayoutMethods};
use super::transaction::{DaoTransactionError, DaoTransactionMethods};

use super::{
    DaoResult,
    DAO,
    DaoTransaction,
};

/// High-level interface for database operations.
///
/// This trait defines the public API for the DAO and can be easily mocked for testing.
/// All methods delegate to the existing trait implementations.
///
/// # Example
///
/// ```rust
/// use kalatori::dao::{DaoInterface, MockDaoInterface};
///
/// #[tokio::test]
/// async fn test_with_mock() {
///     let mut mock = MockDaoInterface::new();
///
///     mock.expect_create_invoice()
///         .returning(|inv| Ok(inv));
///
///     // Use mock in your test
/// }
/// ```
#[cfg_attr(test, mockall::automock(type Transaction = MockDaoTransactionInterface;))]
#[trait_variant::make(Send)]
pub trait DaoInterface: Send + Sync {
    type Transaction: DaoTransactionInterface;

    async fn begin_transaction(&self) -> DaoResult<Self::Transaction>;

    // === Invoice Methods ===

    /// Create a new invoice in the database.
    async fn create_invoice(&self, invoice: Invoice) -> Result<Invoice, DaoInvoiceError>;

    /// Get an invoice by its unique ID.
    async fn get_invoice_by_id(&self, invoice_id: Uuid) -> Result<Option<Invoice>, DaoInvoiceError>;

    /// Get an invoice by its order ID (external identifier).
    async fn get_invoice_by_order_id(&self, order_id: &str) -> Result<Option<Invoice>, DaoInvoiceError>;

    /// Get all active invoices (Waiting or PartiallyPaid status).
    async fn get_active_invoices(&self) -> Result<Vec<Invoice>, DaoInvoiceError>;

    /// Update an invoice's status.
    async fn update_invoice_status(
        &self,
        invoice_id: Uuid,
        status: InvoiceStatus,
    ) -> Result<Invoice, DaoInvoiceError>;

    /// Update invoice data (amount, cart, valid_till).
    /// Requires expected version for optimistic locking.
    async fn update_invoice_data(
        &self,
        data: UpdateInvoiceData,
    ) -> Result<Invoice, DaoInvoiceError>;

    /// Update an invoice's withdrawal status.
    async fn update_invoice_withdrawal_status(
        &self,
        invoice_id: Uuid,
        status: WithdrawalStatus,
    ) -> Result<Invoice, DaoInvoiceError>;

    // === Transaction Methods ===

    /// Create a new transaction record.
    async fn create_transaction(&self, transaction: Transaction) -> Result<Transaction, DaoTransactionError>;

    /// Mark a transaction as successful with blockchain coordinates.
    async fn update_transaction_successful(
        &self,
        transaction_id: Uuid,
        chain_transaction_id: GeneralTransactionId,
        confirmed_at: DateTime<Utc>,
    ) -> Result<Transaction, DaoTransactionError>;

    /// Mark a transaction as failed with error details.
    async fn update_transaction_failed(
        &self,
        transaction_id: Uuid,
        chain_transaction_id: GeneralTransactionId,
        failure_message: String,
        failed_at: DateTime<Utc>,
    ) -> Result<Transaction, DaoTransactionError>;

    /// Update a transaction with full Transaction object.
    async fn update_transaction(
        &self,
        transaction: Transaction,
    ) -> Result<Transaction, DaoTransactionError>;

    /// Get all transactions for a specific invoice.
    async fn get_invoice_transactions(&self, invoice_id: Uuid) -> Result<Vec<Transaction>, DaoTransactionError>;

    // === Payout Methods ===

    /// Create a new payout record.
    async fn create_payout(&self, payout: Payout) -> Result<Payout, DaoPayoutError>;

    /// Get a payout by its ID.
    async fn get_payout_by_id(&self, payout_id: Uuid) -> Result<Option<Payout>, DaoPayoutError>;

    /// Get all pending payouts (Waiting status) and mark them as InProgress.
    /// Returns up to `limit` payouts.
    async fn get_pending_payouts(&self, limit: u32) -> Result<Vec<Payout>, DaoPayoutError>;

    /// Update a payout's status.
    async fn update_payout_status(
        &self,
        payout_id: Uuid,
        new_status: PayoutStatus,
    ) -> Result<Payout, DaoPayoutError>;

    /// Update payout retry metadata and status.
    async fn update_payout_retry(
        &self,
        payout_id: Uuid,
        retry_meta: RetryMeta,
        is_retriable: bool,
    ) -> Result<Payout, DaoPayoutError>;

    // === Utility Methods ===

    /// Check if a transaction with given bytes already exists.
    async fn transaction_exists_by_bytes(&self, transaction_bytes: &str) -> DaoResult<bool>;
}

/// Interface for database transaction operations.
///
/// Provides the same high-level methods as DaoInterface but within a transaction context.
/// Must be committed or rolled back explicitly.
///
/// # Example
///
/// ```rust
/// let tx = dao.begin_transaction().await?;
/// tx.create_invoice(invoice).await?;
/// tx.create_transaction(transaction).await?;
/// tx.commit().await?;
/// ```
#[cfg_attr(test, mockall::automock)]
#[trait_variant::make(Send)]
pub trait DaoTransactionInterface {
    // === Invoice Methods ===

    async fn create_invoice(&self, invoice: Invoice) -> Result<Invoice, DaoInvoiceError>;
    async fn get_invoice_by_id(&self, invoice_id: Uuid) -> Result<Option<Invoice>, DaoInvoiceError>;
    async fn get_invoice_by_order_id(&self, order_id: &str) -> Result<Option<Invoice>, DaoInvoiceError>;
    async fn get_active_invoices(&self) -> Result<Vec<Invoice>, DaoInvoiceError>;
    async fn update_invoice_status(
        &self,
        invoice_id: Uuid,
        status: InvoiceStatus,
    ) -> Result<Invoice, DaoInvoiceError>;
    async fn update_invoice_data(
        &self,
        data: UpdateInvoiceData,
    ) -> Result<Invoice, DaoInvoiceError>;
    async fn update_invoice_withdrawal_status(
        &self,
        invoice_id: Uuid,
        status: WithdrawalStatus,
    ) -> Result<Invoice, DaoInvoiceError>;

    // === Transaction Methods ===

    async fn create_transaction(&self, transaction: Transaction) -> Result<Transaction, DaoTransactionError>;
    async fn update_transaction_successful(
        &self,
        transaction_id: Uuid,
        chain_transaction_id: GeneralTransactionId,
        confirmed_at: DateTime<Utc>,
    ) -> Result<Transaction, DaoTransactionError>;
    async fn update_transaction_failed(
        &self,
        transaction_id: Uuid,
        chain_transaction_id: GeneralTransactionId,
        failure_message: String,
        failed_at: DateTime<Utc>,
    ) -> Result<Transaction, DaoTransactionError>;
    async fn update_transaction(
        &self,
        transaction: Transaction,
    ) -> Result<Transaction, DaoTransactionError>;
    async fn get_invoice_transactions(&self, invoice_id: Uuid) -> Result<Vec<Transaction>, DaoTransactionError>;

    // === Payout Methods ===

    async fn create_payout(&self, payout: Payout) -> Result<Payout, DaoPayoutError>;
    async fn get_payout_by_id(&self, payout_id: Uuid) -> Result<Option<Payout>, DaoPayoutError>;
    async fn get_pending_payouts(&self, limit: u32) -> Result<Vec<Payout>, DaoPayoutError>;
    async fn update_payout_status(
        &self,
        payout_id: Uuid,
        new_status: PayoutStatus,
    ) -> Result<Payout, DaoPayoutError>;
    async fn update_payout_retry(
        &self,
        payout_id: Uuid,
        retry_meta: RetryMeta,
        is_retriable: bool,
    ) -> Result<Payout, DaoPayoutError>;

    // === Transaction Control ===

    /// Commit the transaction, persisting all changes.
    async fn commit(self) -> DaoResult<()>;

    /// Rollback the transaction, discarding all changes.
    async fn rollback(self) -> DaoResult<()>;
}

// ============================================================================
// Implementation for DAO (delegates to existing trait methods)
// ============================================================================

impl DaoInterface for DAO {
    type Transaction = DaoTransaction;

    async fn begin_transaction(&self) -> DaoResult<Self::Transaction> {
        DAO::begin_transaction(self).await
    }

    async fn create_invoice(&self, invoice: Invoice) -> Result<Invoice, DaoInvoiceError> {
        DaoInvoiceMethods::create_invoice(self, invoice).await
    }

    async fn get_invoice_by_id(&self, invoice_id: Uuid) -> Result<Option<Invoice>, DaoInvoiceError> {
        DaoInvoiceMethods::get_invoice_by_id(self, invoice_id).await
    }

    async fn get_invoice_by_order_id(&self, order_id: &str) -> Result<Option<Invoice>, DaoInvoiceError> {
        DaoInvoiceMethods::get_invoice_by_order_id(self, order_id).await
    }

    async fn get_active_invoices(&self) -> Result<Vec<Invoice>, DaoInvoiceError> {
        DaoInvoiceMethods::get_active_invoices(self).await
    }

    async fn update_invoice_status(
        &self,
        invoice_id: Uuid,
        status: InvoiceStatus,
    ) -> Result<Invoice, DaoInvoiceError> {
        DaoInvoiceMethods::update_invoice_status(self, invoice_id, status).await
    }

    async fn update_invoice_data(
        &self,
        data: UpdateInvoiceData,
    ) -> Result<Invoice, DaoInvoiceError> {
        DaoInvoiceMethods::update_invoice_data(self, data).await
    }

    async fn update_invoice_withdrawal_status(
        &self,
        invoice_id: Uuid,
        status: WithdrawalStatus,
    ) -> Result<Invoice, DaoInvoiceError> {
        DaoInvoiceMethods::update_invoice_withdrawal_status(self, invoice_id, status).await
    }

    async fn create_transaction(&self, transaction: Transaction) -> Result<Transaction, DaoTransactionError> {
        DaoTransactionMethods::create_transaction(self, transaction).await
    }

    async fn update_transaction_successful(
        &self,
        transaction_id: Uuid,
        chain_transaction_id: GeneralTransactionId,
        confirmed_at: DateTime<Utc>,
    ) -> Result<Transaction, DaoTransactionError> {
        DaoTransactionMethods::update_transaction_successful(
            self,
            transaction_id,
            chain_transaction_id,
            confirmed_at,
        ).await
    }

    async fn update_transaction_failed(
        &self,
        transaction_id: Uuid,
        chain_transaction_id: GeneralTransactionId,
        failure_message: String,
        failed_at: DateTime<Utc>,
    ) -> Result<Transaction, DaoTransactionError> {
        DaoTransactionMethods::update_transaction_failed(
            self,
            transaction_id,
            chain_transaction_id,
            failure_message,
            failed_at,
        ).await
    }

    async fn update_transaction(
        &self,
        transaction: Transaction,
    ) -> Result<Transaction, DaoTransactionError> {
        DaoTransactionMethods::update_transaction(self, transaction).await
    }

    async fn get_invoice_transactions(&self, invoice_id: Uuid) -> Result<Vec<Transaction>, DaoTransactionError> {
        DaoTransactionMethods::get_invoice_transactions(self, invoice_id).await
    }

    async fn create_payout(&self, payout: Payout) -> Result<Payout, DaoPayoutError> {
        DaoPayoutMethods::create_payout(self, payout).await
    }

    async fn get_payout_by_id(&self, payout_id: Uuid) -> Result<Option<Payout>, DaoPayoutError> {
        DaoPayoutMethods::get_payout_by_id(self, payout_id).await
    }

    async fn get_pending_payouts(&self, limit: u32) -> Result<Vec<Payout>, DaoPayoutError> {
        DaoPayoutMethods::get_pending_payouts(self, limit).await
    }

    async fn update_payout_status(
        &self,
        payout_id: Uuid,
        new_status: PayoutStatus,
    ) -> Result<Payout, DaoPayoutError> {
        DaoPayoutMethods::update_payout_status(self, payout_id, new_status).await
    }

    async fn update_payout_retry(
        &self,
        payout_id: Uuid,
        retry_meta: RetryMeta,
        is_retriable: bool,
    ) -> Result<Payout, DaoPayoutError> {
        DaoPayoutMethods::update_payout_retry(self, payout_id, retry_meta, is_retriable).await
    }

    async fn transaction_exists_by_bytes(&self, transaction_bytes: &str) -> DaoResult<bool> {
        DAO::transaction_exists_by_bytes(self, transaction_bytes).await
    }
}

// ============================================================================
// Implementation for DaoTransaction (delegates to existing trait methods)
// ============================================================================

impl DaoTransactionInterface for DaoTransaction {
    async fn create_invoice(&self, invoice: Invoice) -> Result<Invoice, DaoInvoiceError> {
        DaoInvoiceMethods::create_invoice(self, invoice).await
    }

    async fn get_invoice_by_id(&self, invoice_id: Uuid) -> Result<Option<Invoice>, DaoInvoiceError> {
        DaoInvoiceMethods::get_invoice_by_id(self, invoice_id).await
    }

    async fn get_invoice_by_order_id(&self, order_id: &str) -> Result<Option<Invoice>, DaoInvoiceError> {
        DaoInvoiceMethods::get_invoice_by_order_id(self, order_id).await
    }

    async fn get_active_invoices(&self) -> Result<Vec<Invoice>, DaoInvoiceError> {
        DaoInvoiceMethods::get_active_invoices(self).await
    }

    async fn update_invoice_status(
        &self,
        invoice_id: Uuid,
        status: InvoiceStatus,
    ) -> Result<Invoice, DaoInvoiceError> {
        DaoInvoiceMethods::update_invoice_status(self, invoice_id, status).await
    }

    async fn update_invoice_data(
        &self,
        data: UpdateInvoiceData,
    ) -> Result<Invoice, DaoInvoiceError> {
        DaoInvoiceMethods::update_invoice_data(self, data).await
    }

    async fn update_invoice_withdrawal_status(
        &self,
        invoice_id: Uuid,
        status: WithdrawalStatus,
    ) -> Result<Invoice, DaoInvoiceError> {
        DaoInvoiceMethods::update_invoice_withdrawal_status(self, invoice_id, status).await
    }

    async fn create_transaction(&self, transaction: Transaction) -> Result<Transaction, DaoTransactionError> {
        DaoTransactionMethods::create_transaction(self, transaction).await
    }

    async fn update_transaction_successful(
        &self,
        transaction_id: Uuid,
        chain_transaction_id: GeneralTransactionId,
        confirmed_at: DateTime<Utc>,
    ) -> Result<Transaction, DaoTransactionError> {
        DaoTransactionMethods::update_transaction_successful(
            self,
            transaction_id,
            chain_transaction_id,
            confirmed_at,
        ).await
    }

    async fn update_transaction_failed(
        &self,
        transaction_id: Uuid,
        chain_transaction_id: GeneralTransactionId,
        failure_message: String,
        failed_at: DateTime<Utc>,
    ) -> Result<Transaction, DaoTransactionError> {
        DaoTransactionMethods::update_transaction_failed(
            self,
            transaction_id,
            chain_transaction_id,
            failure_message,
            failed_at,
        ).await
    }

    async fn update_transaction(
        &self,
        transaction: Transaction,
    ) -> Result<Transaction, DaoTransactionError> {
        DaoTransactionMethods::update_transaction(self, transaction).await
    }

    async fn get_invoice_transactions(&self, invoice_id: Uuid) -> Result<Vec<Transaction>, DaoTransactionError> {
        DaoTransactionMethods::get_invoice_transactions(self, invoice_id).await
    }

    async fn create_payout(&self, payout: Payout) -> Result<Payout, DaoPayoutError> {
        DaoPayoutMethods::create_payout(self, payout).await
    }

    async fn get_payout_by_id(&self, payout_id: Uuid) -> Result<Option<Payout>, DaoPayoutError> {
        DaoPayoutMethods::get_payout_by_id(self, payout_id).await
    }

    async fn get_pending_payouts(&self, limit: u32) -> Result<Vec<Payout>, DaoPayoutError> {
        DaoPayoutMethods::get_pending_payouts(self, limit).await
    }

    async fn update_payout_status(
        &self,
        payout_id: Uuid,
        new_status: PayoutStatus,
    ) -> Result<Payout, DaoPayoutError> {
        DaoPayoutMethods::update_payout_status(self, payout_id, new_status).await
    }

    async fn update_payout_retry(
        &self,
        payout_id: Uuid,
        retry_meta: RetryMeta,
        is_retriable: bool,
    ) -> Result<Payout, DaoPayoutError> {
        DaoPayoutMethods::update_payout_retry(self, payout_id, retry_meta, is_retriable).await
    }

    async fn commit(self) -> DaoResult<()> {
        DaoTransaction::commit(self).await
    }

    async fn rollback(self) -> DaoResult<()> {
        DaoTransaction::rollback(self).await
    }
}

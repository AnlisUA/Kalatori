use sqlx::types::{
    Json,
    Text,
};
use uuid::Uuid;

use crate::legacy_types::WithdrawalStatus;
use crate::types::{
    Invoice,
    InvoiceRow,
    InvoiceStatus,
    UpdateInvoiceData,
};

use super::{
    DaoExecutor,
    DaoResult,
};

pub trait DaoInvoiceMethods: DaoExecutor + 'static {
    async fn create_invoice(
        &self,
        invoice: Invoice,
    ) -> DaoResult<Invoice> {
        let query = sqlx::query_as::<_, InvoiceRow>(
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
            .bind(invoice.version);

        self.fetch_one(query)
            .await
            .map(From::from)
    }

    async fn get_invoice_by_id(
        &self,
        invoice_id: Uuid,
    ) -> DaoResult<Option<Invoice>> {
        let query = sqlx::query_as::<_, InvoiceRow>(
            "SELECT *
            FROM invoices
            WHERE id = ?",
        )
        .bind(invoice_id);

        let result = self
            .fetch_optional(query)
            .await?
            .map(From::from);

        Ok(result)
    }

    async fn get_invoice_by_order_id(
        &self,
        order_id: &str,
    ) -> DaoResult<Option<Invoice>> {
        let query = sqlx::query_as::<_, InvoiceRow>(
            "SELECT *
            FROM invoices
            WHERE order_id = ?",
        )
        .bind(order_id);

        let result = self
            .fetch_optional(query)
            .await?
            .map(From::from);

        Ok(result)
    }

    /// Get all active invoices that need to be monitored
    /// Returns invoices with status 'Waiting' or '`PartiallyPaid`'
    async fn get_active_invoices(&self) -> DaoResult<Vec<Invoice>> {
        let query = sqlx::query_as::<_, InvoiceRow>(
            "SELECT *
            FROM invoices
            WHERE status IN ('Waiting', 'PartiallyPaid')
            ORDER BY created_at ASC",
        );

        // TODO: it might be optimized, we already iterate over all values in
        // DaoExecutor trait
        let result = self
            .fetch_all(query)
            .await?
            .into_iter()
            .map(From::from)
            .collect();

        Ok(result)
    }

    async fn update_invoice_status(
        &self,
        invoice_id: Uuid,
        status: InvoiceStatus,
    ) -> DaoResult<Invoice> {
        // TODO: add status transition validation
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
            .map(From::from)
    }

    async fn update_invoice_data(
        &self,
        data: UpdateInvoiceData,
    ) -> DaoResult<Invoice> {
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

        self.fetch_one(query)
            .await
            .map(From::from)
    }

    async fn update_invoice_withdrawal_status(
        &self,
        invoice_id: Uuid,
        status: WithdrawalStatus,
    ) -> DaoResult<Invoice> {
        let query = sqlx::query_as::<_, InvoiceRow>(
            "UPDATE invoices
            SET withdrawal_status = ?,
                updated_at = datetime('now'),
                version = version + 1
            WHERE id = ? AND withdrawal_status == 'Waiting'
            RETURNING *",
        )
        .bind(status)
        .bind(invoice_id);

        self.fetch_one(query)
            .await
            .map(From::from)
    }
}

impl<T: DaoExecutor + 'static> DaoInvoiceMethods for T {}

#[cfg(test)]
mod tests {
    use crate::dao::create_test_dao;
    use crate::types::{
        default_invoice,
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
}

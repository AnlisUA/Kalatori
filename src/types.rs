/// Module defining various types used across the Kalatori application.
/// Each domain-specific type (or collection of types) is organized into its own submodule.
///
/// For testing purposes, it's also recommended to create fixtures functions within each submodule
/// to facilitate easy generation of test data. For example:
/// ```ignore
/// // In invoice.rs
/// #[cfg(test)]
/// fn default_invoice() -> Invoice {
///    // Create and return a default Invoice instance for testing
/// }
/// ```

mod common;
mod invoice;
mod payout;
mod refund;
mod transaction;

// Re-export commonly used types for convenience
#[expect(unused_imports)]
pub use common::*;
pub use invoice::*;
#[expect(unused_imports)]
pub use payout::*;
#[expect(unused_imports)]
pub use refund::*;
pub use transaction::*;

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::Utc;
    use rust_decimal::Decimal;
    use sqlx::types::{Json, Text};
    use uuid::Uuid;

    use crate::legacy_types::WithdrawalStatus;

    #[tokio::test]
    async fn test_sql_query() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();

        let invoice = Invoice {
            id: Uuid::new_v4(),
            order_id: "order123".to_string(),
            asset_id: Some(1),
            chain: "TestNet".to_string(),
            amount: Decimal::new(1000, 2),
            payment_address: "addr_test".to_string(),
            status: InvoiceStatus::Waiting,
            withdrawal_status: WithdrawalStatus::Waiting,
            callback: "http://callback.url".to_string(),
            cart: InvoiceCart::empty(),
            valid_till: Utc::now(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            version: 1,
        };

        let result = sqlx::query(
            "INSERT INTO invoices (id, order_id, asset_id, chain, amount, payment_address, status, withdrawal_status, callback, cart, valid_till, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
            .execute(&pool)
            .await
            .unwrap();

        println!("Insert result: {result:?}");

        let query = sqlx::query_as::<sqlx::Sqlite, invoice::InvoiceRow>(
            "SELECT id, order_id, asset_id, chain, amount, payment_address, status, withdrawal_status, callback, valid_till, cart, created_at, updated_at, version FROM invoices",
        )
            .fetch_all(&pool)
            .await
            .unwrap()
            .into_iter()
            .map(Invoice::from)
            .collect::<Vec<_>>();

        println!("Results: {query:?}");
    }
}

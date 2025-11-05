//! New data types for SQLite schema
//!
//! This module contains the new data structures used with the SQLite database,
//! including conversion traits for backward compatibility with the old sled-based types.
mod invoice;
mod refund;
mod common;
mod transaction;
mod payout;

// Re-export commonly used types for convenience
pub use common::InitiatorType;
pub use invoice::{Invoice, InvoiceStatus};
pub use payout::{Payout, PayoutStatus};
pub use refund::{Refund, RefundStatus};
pub use transaction::{
    OutgoingTransactionMeta,
    Transaction,
    TransactionOrigin,
    TransactionStatus,
    TransactionType,
};

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::Utc;
    use rust_decimal::Decimal;
    use sqlx::types::Text;
    use uuid::Uuid;

    use crate::definitions::api_v2::WithdrawalStatus;

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
            valid_till: Utc::now(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let result = sqlx::query(
            "INSERT INTO invoices (id, order_id, asset_id, chain, amount, payment_address, status, withdrawal_status, callback, valid_till, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
            .bind(invoice.valid_till)
            .bind(invoice.created_at)
            .bind(invoice.updated_at)
            .execute(&pool)
            .await
            .unwrap();

        println!("Insert result: {:?}", result);

        let query = sqlx::query_as::<sqlx::Sqlite, invoice::InvoiceRow>(
            "SELECT id, order_id, asset_id, chain, amount, payment_address, status, withdrawal_status, callback, valid_till, created_at, updated_at FROM invoices",
        )
            .fetch_all(&pool)
            .await
            .unwrap()
            .into_iter()
            .map(Invoice::from)
            .collect::<Vec<_>>();

        println!("Results: {:?}", query);
    }
}

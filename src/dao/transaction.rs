use chrono::{
    DateTime,
    Utc,
};
use sqlx::types::{
    Json,
    Text,
};
use uuid::Uuid;

use crate::chain_client::GeneralTransactionId;
use crate::types::{
    Transaction,
    TransactionRow,
};

use super::{
    DaoExecutor,
    DaoResult,
};

pub trait DaoTransactionMethods: DaoExecutor + 'static {
    async fn create_transaction(
        &self,
        transaction: Transaction,
    ) -> DaoResult<Transaction> {
        let query = sqlx::query_as::<_, TransactionRow>(
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
            .bind(&transaction.transaction_bytes);

        self.fetch_one(query)
            .await
            .map(From::from)
    }

    async fn update_transaction_successful(
        &self,
        transaction_id: Uuid,
        chain_transaction_id: GeneralTransactionId,
        confirmed_at: DateTime<Utc>,
    ) -> DaoResult<Transaction> {
        // TODO: add additional check that transaction is `Outgoing`? Check it's status?
        // TODO: add updated_at field?
        let query = sqlx::query_as::<_, TransactionRow>(
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
        .bind(transaction_id);

        self.fetch_one(query)
            .await
            .map(From::from)
    }

    async fn update_transaction_failed(
        &self,
        transaction_id: Uuid,
        chain_transaction_id: GeneralTransactionId,
        failure_message: String,
        failed_at: DateTime<Utc>,
    ) -> DaoResult<Transaction> {
        // TODO: add additional check that transaction is `Outgoing`? Check it's status?
        let query = sqlx::query_as::<_, TransactionRow>(
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
        .bind(transaction_id);

        self.fetch_one(query)
            .await
            .map(From::from)
    }

    #[cfg_attr(not(test), expect(dead_code))]
    async fn update_transaction(
        &self,
        transaction: Transaction,
    ) -> DaoResult<Transaction> {
        let query = sqlx::query_as::<_, TransactionRow>(
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
        .bind(transaction.id);

        self.fetch_one(query)
            .await
            .map(From::from)
    }

    // TODO: Implement create_transaction_outgoing when OutgoingTransaction type is
    // defined async fn create_transaction_outgoing(&self, transaction:
    // OutgoingTransaction) -> DaoResult<Uuid> {     todo!("Implement outgoing
    // transaction creation") }

    async fn get_invoice_transactions(
        &self,
        invoice_id: Uuid,
    ) -> DaoResult<Vec<Transaction>> {
        let query = sqlx::query_as::<_, TransactionRow>(
        "SELECT id, invoice_id, asset_id, chain, amount, sender, recipient, block_number, position_in_block, tx_hash, origin, status, transaction_type, outgoing_meta, created_at, transaction_bytes
            FROM transactions
            WHERE invoice_id = ?
            ORDER BY created_at ASC",
        )
            .bind(invoice_id);

        let transactions = self
            .fetch_all(query)
            .await?
            .into_iter()
            .map(From::from)
            .collect();

        Ok(transactions)
    }
}

impl<T: DaoExecutor + 'static> DaoTransactionMethods for T {}

#[cfg(test)]
mod tests {
    use crate::dao::{
        DaoInvoiceMethods,
        create_test_dao,
    };

    use crate::types::{
        OutgoingTransactionMeta,
        Transaction,
        TransactionOrigin,
        TransactionStatus,
        TransactionType,
        default_invoice,
        default_transaction,
    };

    use super::*;

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

        let now1 = Utc::now();

        let updated1 = dao
            .update_transaction_failed(
                transaction_id,
                chain_transaction_id.clone(),
                "Network error".to_string(),
                now1,
            )
            .await
            .unwrap();

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

        let now2 = Utc::now();

        let updated2 = dao
            .update_transaction_successful(
                transaction_id,
                chain_transaction_id,
                now2,
            )
            .await
            .unwrap();

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
}

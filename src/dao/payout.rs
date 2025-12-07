use sqlx::types::Text;
use uuid::Uuid;

use crate::types::{
    Payout,
    PayoutRow,
    PayoutStatus,
    RetryMeta,
};

use super::{
    DaoExecutor,
    DaoResult,
};

pub trait DaoPayoutMethods: DaoExecutor + 'static {
    async fn create_payout(
        &self,
        payout: Payout,
    ) -> DaoResult<Payout> {
        let query = sqlx::query_as::<_, PayoutRow>(
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
            .bind(&payout.retry_meta.failure_message);

        self.fetch_one(query)
            .await
            .map(From::from)
    }

    #[cfg_attr(not(test), expect(dead_code))]
    async fn get_payout_by_id(
        &self,
        payout_id: Uuid,
    ) -> DaoResult<Option<Payout>> {
        let query = sqlx::query_as::<_, PayoutRow>(
            "SELECT *
            FROM payouts
            WHERE id = ?",
        )
        .bind(payout_id);

        let result = self
            .fetch_optional(query)
            .await?
            .map(From::from);

        Ok(result)
    }

    /// Fetch pending payouts and mark them as `InProgress`
    // TODO: besides of Payouts it should also return associated outgoing
    // Transactions
    async fn get_pending_payouts(
        &self,
        limit: u32,
    ) -> DaoResult<Vec<Payout>> {
        // TODO: in future versions of sqlite (bundled in sqlx) we'll probably be able
        // to use UPDATE ... ORDER BY LIMIT directly
        let query = sqlx::query_as::<_, PayoutRow>(
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
        .bind(limit);

        let result = self
            .fetch_all(query)
            .await?
            .into_iter()
            .map(From::from)
            .collect();

        Ok(result)
    }

    async fn update_payout_status(
        &self,
        payout_id: Uuid,
        status: PayoutStatus,
    ) -> DaoResult<Payout> {
        // TODO: add status transition validation
        let query = sqlx::query_as::<_, PayoutRow>(
            "UPDATE payouts
            SET status = ?, updated_at = datetime('now')
            WHERE id = ?
            RETURNING *",
        )
        .bind(status)
        .bind(payout_id);

        self.fetch_one(query)
            .await
            .map(From::from)
    }

    async fn update_payout_retry(
        &self,
        payout_id: Uuid,
        retry_meta: RetryMeta,
        is_retriable: bool,
    ) -> DaoResult<Payout> {
        let status = if is_retriable {
            PayoutStatus::FailedRetriable
        } else {
            PayoutStatus::Failed
        };

        let query = sqlx::query_as::<_, PayoutRow>(
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
        .bind(payout_id);

        self.fetch_one(query)
            .await
            .map(From::from)
    }
}

impl<T: DaoExecutor + 'static> DaoPayoutMethods for T {}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use crate::dao::{
        DaoInvoiceMethods,
        create_test_dao,
    };
    use crate::types::{
        default_invoice,
        default_payout,
    };

    use super::*;

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
        let updated = dao
            .update_payout_status(payout_id, PayoutStatus::InProgress)
            .await
            .unwrap();

        assert_eq!(updated.status, PayoutStatus::InProgress);

        // Update to Completed
        let completed = dao
            .update_payout_status(payout_id, PayoutStatus::Completed)
            .await
            .unwrap();

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
        let now = Utc::now();
        let next_retry = now + chrono::Duration::minutes(1);

        let retry_meta = RetryMeta {
            retry_count: 1,
            last_attempt_at: Some(now),
            next_retry_at: Some(next_retry),
            failure_message: Some("Network error".to_string()),
        };

        let updated = dao
            .update_payout_retry(payout_id, retry_meta, true)
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
            Some("Network error".to_string())
        );
        assert_eq!(
            updated.status,
            PayoutStatus::FailedRetriable
        );

        // Second retry
        let now2 = Utc::now();
        let next_retry2 = now2 + chrono::Duration::minutes(5);

        let retry_meta2 = RetryMeta {
            retry_count: 2,
            last_attempt_at: Some(now2),
            next_retry_at: Some(next_retry2),
            failure_message: Some("Connection timeout".to_string()),
        };

        let updated2 = dao
            .update_payout_retry(payout_id, retry_meta2, false)
            .await
            .unwrap();

        assert_eq!(updated2.retry_meta.retry_count, 2);
        assert_eq!(
            updated2.retry_meta.failure_message,
            Some("Connection timeout".to_string())
        );
        assert_eq!(updated2.status, PayoutStatus::Failed);
    }
}

use std::time::Duration;
use std::pin::Pin;

use tokio::time::interval;
use tokio_util::sync::CancellationToken;
use futures::stream::{FuturesUnordered, StreamExt};

use crate::chain::InvoiceRegistry;
use crate::types::Invoice;
use crate::dao::DaoInterface;

const EXPIRATION_CHECK_INTERVAL_MILLIS: u64 = 1000;

async fn send_webhook(client: reqwest::Client, invoice: Invoice) {
    if invoice.callback.is_empty() {
        tracing::warn!(
            invoice_id = %invoice.id,
            error.category = "expiration_detector",
            error.operation = "send_expiration_webhook",
            "Invoice has no callback URL, skipping expiration webhook"
        );

        return;
    }

    if let Err(e) = client.get(invoice.callback).send().await {
        tracing::warn!(
            invoice_id = %invoice.id,
            error.category = "expiration_detector",
            error.operation = "send_expiration_webhook",
            error.source = ?e,
            "Failed to send expiration webhook for invoice"
        );
    } else {
        tracing::info!(
            invoice_id = %invoice.id,
            "Sent expiration webhook for invoice"
        )
    }
}

pub struct ExpirationDetector<D: DaoInterface + 'static> {
    client: reqwest::Client,
    dao: D,
    registry: InvoiceRegistry,
}

impl<D: DaoInterface + 'static> ExpirationDetector<D> {
    pub fn new(dao: D, registry: InvoiceRegistry) -> Self {
        ExpirationDetector {
            client: reqwest::Client::new(),
            dao,
            registry,
        }
    }

    async fn fetch_expired_invoices(&self) -> Vec<Invoice> {
        let expired_unpaid = self.dao
            .update_invoices_expired()
            .await
            .inspect_err(|_| {
                tracing::warn!(
                    error.category = "expiration_detector",
                    error.operation = "fetch_expired_invoices",
                    "Failed to fetch expired invoices from database"
                );
            })
            .unwrap_or_default();

        // TODO: fetch partially paid expired invoices as well and return them together

        expired_unpaid
    }

    fn build_future(&self, invoice: Invoice) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        let client = self.client.clone();
        Box::pin(send_webhook(client, invoice))
    }

    // 1. Update statuses in the database for expired and partially paid expired invoices
    // 2. Notify tracker, it should remove them from tracking
    // 3. TODO: Check balances one last time (ensure that we didn't miss any transfers)
    // 4. Schedule webhooks for expired invoices
    async fn handle_expirations(&self) -> Vec<Pin<Box<dyn Future<Output = ()> + Send + 'static>>> {
        let expired_invoices = self.fetch_expired_invoices().await;

        let expired_invoices_ids: Vec<_> = expired_invoices
            .iter()
            .map(|inv| inv.id)
            .collect();

        // TODO: send notification to remove expired invoices to tracker after it's refactoring.
        // For now it's not necessary cause tracker will check if invoice is expired on each balance check.

        if !expired_invoices_ids.is_empty() {
            tracing::info!(
                invoice_ids = ?expired_invoices_ids,
                expired_count = expired_invoices_ids.len(),
                "Marked invoices as expired"
            );

            self.registry.remove_invoices(&expired_invoices_ids).await;
        }

        // TODO: later we'll build futures which will check balances one last time for partially paid invoices
        // and return them from this function. Also we'll have to schedule webhooks for expired invoices here
        // using transactional outbox pattern. But for now, as we don't have time for that, we'll just return
        // futures which actually send webhooks for the updated invoices.
        expired_invoices
            .into_iter()
            .map(|invoice| self.build_future(invoice))
            .collect()
    }

    async fn perform(
        self,
        token: CancellationToken,
    ) {
        let mut interval = interval(Duration::from_millis(EXPIRATION_CHECK_INTERVAL_MILLIS));

        let mut shutdown_expected = false;
        let mut futures_set = FuturesUnordered::new();

        loop {
            tokio::select! {
                _ = interval.tick(), if !shutdown_expected => {
                    futures_set.extend(self.handle_expirations().await);
                }
                _ = futures_set.next(), if !futures_set.is_empty() => {
                    if futures_set.is_empty() && shutdown_expected {
                        tracing::info!(
                            "All pending tasks finished, expiration detector is shutting down"
                        );

                        break;
                    }
                }
                _ = token.cancelled() => {
                    tracing::info!(
                        "Expiration detector received shutdown signal, finishing pending tasks before shutting down"
                    );

                    shutdown_expected = true;

                    if futures_set.is_empty() {
                        tracing::info!(
                            "No pending tasks, expiration detector is shutting down"
                        );

                        break;
                    }
                }
            }
        }
    }

    pub fn ignite(self, token: CancellationToken) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            self.perform(token).await;
        })
    }
}

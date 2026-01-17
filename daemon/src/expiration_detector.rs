use std::time::Duration;

use kalatori_client::types::KalatoriEventExt;
use rust_decimal::Decimal;
use tokio::time::interval;
use tokio_util::sync::CancellationToken;

use crate::chain::InvoiceRegistry;
use crate::configs::PaymentsConfig;
use crate::dao::DaoInterface;
use crate::types::{Invoice, InvoiceEventType};

const EXPIRATION_CHECK_INTERVAL_MILLIS: u64 = 1000;

pub struct ExpirationDetector<D: DaoInterface + 'static> {
    dao: D,
    registry: InvoiceRegistry,
    config: PaymentsConfig,
}

impl<D: DaoInterface + 'static> ExpirationDetector<D> {
    pub fn new(
        dao: D,
        registry: InvoiceRegistry,
        config: PaymentsConfig,
    ) -> Self {
        ExpirationDetector {
            dao,
            registry,
            config,
        }
    }

    async fn fetch_expired_invoices(&self) -> Vec<Invoice> {
        // TODO: fetch partially paid expired invoices as well and return them together

        self.dao
            .update_invoices_expired()
            .await
            .inspect_err(|_| {
                tracing::warn!(
                    error.category = "expiration_detector",
                    error.operation = "fetch_expired_invoices",
                    "Failed to fetch expired invoices from database"
                );
            })
            .unwrap_or_default()
    }

    // 1. Update statuses in the database for expired and partially paid expired
    //    invoices
    // 2. Notify tracker, it should remove them from tracking
    // 3. TODO: Check balances one last time (ensure that we didn't miss any
    //    transfers)
    // 4. Schedule webhooks for expired invoices
    async fn handle_expirations(&self) {
        let expired_invoices = self.fetch_expired_invoices().await;

        let expired_invoices_ids: Vec<_> = expired_invoices
            .iter()
            .map(|inv| inv.id)
            .collect();

        if !expired_invoices_ids.is_empty() {
            tracing::info!(
                invoice_ids = ?expired_invoices_ids,
                expired_count = expired_invoices_ids.len(),
                "Marked invoices as expired"
            );

            self.registry
                .remove_invoices(&expired_invoices_ids)
                .await;
        }

        // TODO: later we'll build futures which will check balances one last time for
        // partially paid invoices and return them from this function. Also
        // we'll have to schedule webhooks for expired invoices here
        // using transactional outbox pattern. But for now, as we don't have time for
        // that, we'll just return futures which actually send webhooks for the
        // updated invoices.

        for invoice in expired_invoices {
            let invoice_id = invoice.id;

            let event = invoice
                // TODO: amount should be set properly when we have partially paid invoices
                .with_amount(Decimal::ZERO)
                .into_public_invoice(&self.config.payment_url_base)
                .build_event(InvoiceEventType::Expired)
                .into();

            // TODO: handle errors properly, maybe set back invoice status if webhook
            // creation fails? Have to think about it. We don't really want to
            // use transaction here cause we'll make relatively slow requests to
            // RPC endpoints.
            if let Err(e) = self
                .dao
                .create_webhook_event(event)
                .await
            {
                tracing::warn!(
                    invoice_id = %invoice_id,
                    error.category = "expiration_detector",
                    error.operation = "handle_expirations",
                    error.source = ?e,
                    "Failed to create expiration webhook event for invoice"
                );
            };
        }
    }

    async fn perform(
        self,
        token: CancellationToken,
    ) {
        let mut interval = interval(Duration::from_millis(
            EXPIRATION_CHECK_INTERVAL_MILLIS,
        ));

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    self.handle_expirations().await;
                }
                () = token.cancelled() => {
                    tracing::info!(
                        "Expiration detector received shutdown signal, finishing pending tasks before shutting down"
                    );

                    break
                }
            }
        }
    }

    pub fn ignite(
        self,
        token: CancellationToken,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            self.perform(token).await;
        })
    }
}

use std::collections::HashSet;
use std::pin::Pin;

use futures::stream::{FuturesUnordered, StreamExt};
use tokio::time::{Duration, interval};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use kalatori_client::utils::{HmacConfig, add_headers_to_reqwest};

use crate::dao::DaoInterface;
use crate::types::WebhookEvent;

const WEBHOOK_SENDER_INTERVAL_MILLIS: u64 = 100;
const WEBHOOK_SENDER_MAX_CONCURRENT_REQUESTS: usize = 10;

#[derive(Debug)]
struct SendWebhookResult {
    event_id: Uuid,
    is_ok: bool,
}

#[tracing::instrument(skip(client, request))]
async fn send_webhook(
    client: reqwest::Client,
    request: reqwest::Request,
    event_id: Uuid,
) -> SendWebhookResult {
    match client.execute(request).await {
        Ok(response) if response.status().is_success() => {
            tracing::debug!(
                event_id = %event_id,
                "Successfully sent webhook event"
            );

            SendWebhookResult {
                event_id,
                is_ok: true,
            }
        },
        Ok(response) => {
            let status = response.status();
            let response_text = response.text().await;
            println!(
                "Response non200 text: {:#?}",
                response_text
            );

            tracing::warn!(
                event_id = %event_id,
                response.status = %status,
                response.text = ?response_text,
                "Failed to send webhook event, non-success status code received",
            );
            SendWebhookResult {
                event_id,
                is_ok: false,
            }
        },
        Err(e) => {
            tracing::warn!(
                event_id = %event_id,
                error = %e,
                "Failed to send webhook event, request error occurred",
            );

            SendWebhookResult {
                event_id,
                is_ok: false,
            }
        },
    }
}

pub struct WebhookSender<D: DaoInterface + 'static> {
    client: reqwest::Client,
    dao: D,
    webhook_url: String,
    hmac_config: HmacConfig,
    processing_events_ids: HashSet<Uuid>,
}

impl<D: DaoInterface + 'static> WebhookSender<D> {
    pub fn new(
        dao: D,
        webhook_url: String,
        hmac_config: HmacConfig,
    ) -> Self {
        WebhookSender {
            client: reqwest::Client::new(),
            dao,
            webhook_url,
            hmac_config,
            processing_events_ids: HashSet::new(),
        }
    }

    fn build_future(
        &self,
        event: WebhookEvent,
    ) -> Pin<Box<dyn Future<Output = SendWebhookResult> + Send + 'static>> {
        let mut request = self
            .client
            .post(&self.webhook_url)
            .json(&event.payload)
            .build()
            // TODO: anything can really go wrong here? Need to research
            .unwrap();

        add_headers_to_reqwest(&self.hmac_config, &mut request);

        Box::pin(send_webhook(
            self.client.clone(),
            request,
            event.id,
        ))
    }

    async fn prepare_webhook_events(
        &mut self
    ) -> Vec<Pin<Box<dyn Future<Output = SendWebhookResult> + Send + 'static>>> {
        let limit = WEBHOOK_SENDER_MAX_CONCURRENT_REQUESTS - self.processing_events_ids.len();

        if limit == 0 {
            return Vec::new();
        }

        let events = self
            .dao
            .get_webhook_events_to_send(u32::try_from(limit).unwrap_or_default())
            .await
            .inspect_err(|_| {
                tracing::warn!(
                    error.category = "webhook_sender",
                    error.operation = "prepare_webhook_events",
                    "Failed to fetch pending webhook events from database"
                );
            })
            .unwrap_or_default();

        events
            .into_iter()
            .filter_map(|event| {
                self.processing_events_ids
                    .insert(event.id)
                    .then_some(self.build_future(event))
            })
            .collect()
    }

    async fn handle_send_webhook_result(
        &mut self,
        result: SendWebhookResult,
    ) {
        self.processing_events_ids
            .remove(&result.event_id);

        if result.is_ok
            && self
                .dao
                .mark_webhook_event_as_sent(result.event_id)
                .await
                .is_err()
        {
            tracing::warn!(
                event_id = %result.event_id,
                error.category = "webhook_sender",
                error.operation = "handle_send_webhook_result",
                "Failed to mark webhook event as sent in database. It might be resent"
            )
        };
        // TODO: for now we do nothing on failure, the event will be retried
        // later. Later we might want to implement some retry strategy
        // with backoff and max attempts count
    }

    async fn perform(
        mut self,
        token: CancellationToken,
    ) {
        let mut interval = interval(Duration::from_millis(
            WEBHOOK_SENDER_INTERVAL_MILLIS,
        ));

        let mut shutdown_expected = false;
        let mut futures_set = FuturesUnordered::new();

        loop {
            tokio::select! {
                _ = interval.tick(), if !shutdown_expected => {
                    futures_set.extend(self.prepare_webhook_events().await);
                }
                future_result = futures_set.next(), if !futures_set.is_empty() => {
                    if let Some(data) = future_result {
                        self.handle_send_webhook_result(data).await;
                    }

                    if futures_set.is_empty() && shutdown_expected {
                        tracing::info!(
                            "All pending tasks finished, expiration detector is shutting down"
                        );

                        break;
                    }
                }
                () = token.cancelled() => {
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

    pub fn ignite(
        self,
        token: CancellationToken,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            self.perform(token).await;
        })
    }
}

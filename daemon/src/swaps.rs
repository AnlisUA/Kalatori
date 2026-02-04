mod executor;
mod one_inch_client;

use std::collections::HashMap;

use alloy::primitives::B256;
use tokio::time::{Duration, interval};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::dao::{DaoInterface, DaoSwapError};
use crate::types::OneInchSwap;

use one_inch_client::{OrderStatusResponse, OrderStatus, OneInchError};

pub use one_inch_client::{OneInchClient, OrderSubmitRequest, UnsignedOrderData};
pub use executor::{SwapsExecutor, SwapsExecutorError};

const SWAPS_EXECUTOR_API_POLLING_INTERVAL_MILLIS: u64 = 3000;
const SWAPS_EXECUTOR_DATABASE_POLLING_INTERVAL_MILLIS: u64 = 100;

type MonitoredSwapsInnerMap = HashMap<Uuid, OneInchSwap>;

#[derive(Debug, Clone)]
struct MonitoredSwaps {
    cross_chain_swaps: MonitoredSwapsInnerMap,
    on_chain_swaps: MonitoredSwapsInnerMap,
}

impl MonitoredSwaps {
    pub fn new() -> Self {
        Self {
            cross_chain_swaps: HashMap::new(),
            on_chain_swaps: HashMap::new(),
        }
    }

    pub fn has_on_chain_swaps(&self) -> bool {
        !self.on_chain_swaps.is_empty()
    }

    pub fn has_cross_swaps(&self) -> bool {
        !self.cross_chain_swaps.is_empty()
    }

    pub fn has_any_swaps(&self) -> bool {
        self.has_cross_swaps() || self.has_on_chain_swaps()
    }

    pub fn add_swaps(&mut self, swaps: Vec<OneInchSwap>) {
        for swap in swaps {
            if swap.is_cross_chain() {
                self.cross_chain_swaps.insert(swap.id, swap);
            } else {
                self.on_chain_swaps.insert(swap.id, swap);
            }
        }
    }

    pub fn remove_cross_swap(&mut self, id: &Uuid) {
        self.cross_chain_swaps.remove(id);
    }

    pub fn get_cross_swap(&self, swap_id: &Uuid) -> Option<&OneInchSwap> {
        self.cross_chain_swaps.get(swap_id)
    }

    pub fn get_cross_swaps_hashes(&self) -> HashMap<B256, Uuid> {
        self.cross_chain_swaps
            .iter()
            .map(|(id, swap)| (swap.order_hash, *id))
            .collect()
    }

}

#[derive(Debug, thiserror::Error)]
pub enum SwapsMonitorError {
    #[error("1Inch client error")]
    OneInchClientError(#[from] OneInchError),
    #[error("No secret with index {0}")]
    NoSecretIndex(u64),
    // TODO: refactor it, we really want to handle different DAO errors in different ways
    #[error("Database error")]
    DatabaseError(#[from] DaoSwapError),
}

pub struct SwapsMonitor<D: DaoInterface + 'static> {
    one_inch_client: OneInchClient,
    dao: D,
    monitored_swaps: MonitoredSwaps,
}

impl<D: DaoInterface + 'static> SwapsMonitor<D> {
    pub fn new(
        dao: D,
        one_inch_client: OneInchClient,
    ) -> Self {
        Self {
            one_inch_client,
            dao,
            monitored_swaps: MonitoredSwaps::new(),
        }
    }

    #[tracing::instrument(skip_all)]
    async fn get_ready_to_fill_escrows_and_submit_secrets(
        &self,
        swap_id: Uuid,
    ) -> Result<(), SwapsMonitorError> {
        let swap = self.monitored_swaps.get_cross_swap(&swap_id).unwrap();

        let ready_fills = self.one_inch_client
            .get_ready_to_accept_secret_fills(swap.order_hash)
            .await?;

        for fill in ready_fills.fills {
            let idx = fill.idx;

            let secret = swap.secrets
                .get(idx as usize)
                .ok_or(SwapsMonitorError::NoSecretIndex(idx))?;

            self.one_inch_client.submit_secret(swap.order_hash, *secret).await?;

            tracing::info!(
                secret_idx = idx,
                secrets_count = swap.secrets.len(),
                "Secret for cross-chain swap has been submitted successfully"
            );
        }

        Ok(())
    }

    #[tracing::instrument(skip(self, order), fields(order_hash = ?order.order_hash))]
    async fn check_monitored_cross_order(
        &mut self,
        swap_id: Uuid,
        order: OrderStatusResponse,
    ) -> Result<(), SwapsMonitorError> {
        match order.status {
            OrderStatus::Executed => {
                self.dao.update_swap_completed(swap_id).await?;
                self.monitored_swaps.remove_cross_swap(&swap_id);
                tracing::info!(
                    "Cross chain order has been executed successfully and marked as Completed"
                );
            },
            OrderStatus::Expired
            | OrderStatus::Cancelled
            | OrderStatus::Refunded
            | OrderStatus::NotFound
            | OrderStatus::Unknown => {
                // TODO: add real reason
                self.dao.update_swap_failed(swap_id, "".to_string()).await?;
                self.monitored_swaps.remove_cross_swap(&swap_id);
                // TODO: add logs. For not found and unknown statusers - warnings (for unknown probably even error)
            },
            OrderStatus::Pending
            | OrderStatus::Filled => {
                tracing::trace!("Cross chain order has non-final status, continue monitoring");

                if order.has_dst_deployed_escrow() {
                    self.get_ready_to_fill_escrows_and_submit_secrets(swap_id).await?;
                }
            },
        }

        return Ok(());
    }

    async fn check_monitored_cross_orders(&mut self) {
        // let cross_swaps = self.monitored_swaps.get_cross_swaps();
        let hashes_with_ids = self.monitored_swaps.get_cross_swaps_hashes();

        let hashes: Vec<_> = hashes_with_ids.keys().copied().collect();

        tracing::trace!(
            swaps_count = hashes_with_ids.len(),
            swaps_hashes = ?hashes,
            swaps_ids = ?hashes_with_ids.values(),
            "Check cross chain swaps statuses"
        );

        println!("Request hashes: {:?}", hashes);

        let cross_orders = self.one_inch_client
            .get_orders_by_hashes(&hashes)
            .await
            // TODO: handle error
            .unwrap();

        for order in cross_orders {
            let order_hash = order.order_hash;

            let swap_id = hashes_with_ids
                .get(&order_hash)
                .copied()
                .unwrap();

            if let Err(e) = self.check_monitored_cross_order(swap_id, order).await {
                // TODO: add error handling. We want to handle different errors in different ways
                tracing::debug!(
                    ?swap_id,
                    ?order_hash,
                    error = ?e,
                    "Error while check cross order"
                );
            }
        }
    }

    async fn perform(
        mut self,
        token: CancellationToken,
    ) {
        let mut api_polling_interval = interval(Duration::from_millis(
            SWAPS_EXECUTOR_API_POLLING_INTERVAL_MILLIS
        ));

        let mut database_polling_interval = interval(Duration::from_millis(
            SWAPS_EXECUTOR_DATABASE_POLLING_INTERVAL_MILLIS
        ));

        loop {
            tokio::select! {
                _ = api_polling_interval.tick(), if self.monitored_swaps.has_any_swaps() => {
                    if self.monitored_swaps.has_cross_swaps() {
                        self.check_monitored_cross_orders().await;
                    }
                },
                _ = database_polling_interval.tick() => {
                    match self.dao.get_submitted_swaps().await {
                        Ok(swaps) => self.monitored_swaps.add_swaps(swaps),
                        Err(e) => tracing::warn!(
                            error = ?e,
                            "Error while fetching submitted swaps for monitoring"
                        ),
                    };
                },
                () = token.cancelled() => {
                    tracing::info!(
                        "Swaps executor received shutdown signal, shutting down immediately"
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

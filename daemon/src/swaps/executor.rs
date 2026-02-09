use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use alloy::primitives::{B256, Address};
use chrono::{TimeDelta, Utc};
use tokio::time::interval;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use tokio::sync::RwLock;
use rust_decimal::Decimal;

use crate::api::ApiErrorExt;
use crate::dao::{DaoInterface, DaoSwapError};

use crate::types::{
    CreateOneInchSwapData,
    OneInchPreparedSwap,
    OneInchSwap,
    OneInchSupportedChain,
    GetPricesResponse,
};

use super::one_inch_client::OneInchError;
use super::OneInchClient;

const AUCTION_TIME_BUFFER_SECS: u64 = 60 * 5; // 5 mins
const CACHE_CLEAN_INTERVAL_MILLIS: u64 = 1_000; // 1 sec

#[derive(Debug, Clone)]
struct OrdersCache {
    orders: Arc<RwLock<HashMap<Uuid, OneInchPreparedSwap>>>,
}

impl OrdersCache {
    pub fn new() -> Self {
        Self {
            orders: Arc::new(RwLock::new(HashMap::new()))
        }
    }

    pub async fn add(&self, swap: OneInchPreparedSwap) {
        self.orders
            .write()
            .await
            .insert(swap.id, swap);
    }

    pub async fn remove(
        &self,
        swap_id: &Uuid,
    ) -> Option<OneInchPreparedSwap> {
        self.orders
            .write()
            .await
            .remove(swap_id)
    }

    pub async fn cleanup(&self) {
        let mut orders = self.orders.write().await;
        let now = Utc::now().timestamp();
        orders.retain(|_, val| val.valid_till.timestamp() > now)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SwapsExecutorError {
    #[error("1Inch API error")]
    OneInchClientError(#[from] OneInchError),
    #[error("Swap not found in cache")]
    SwapNotFound {
        swap_id: Uuid,
    },
    // TODO: refactor error
    #[error("Database error")]
    Database(#[from] DaoSwapError),
}

impl ApiErrorExt for SwapsExecutorError {
    fn category(&self) -> &str {
        match self {
            SwapsExecutorError::OneInchClientError(_)
            | SwapsExecutorError::Database(_) => "INTERNAL_SERVER_ERROR",
            SwapsExecutorError::SwapNotFound { .. } => "ENTITY_NOT_FOUND",
        }
    }

    fn code(&self) -> &str {
        match self {
            SwapsExecutorError::OneInchClientError(_)
            | SwapsExecutorError::Database(_) => "INTERNAL_SERVER_ERROR",
            SwapsExecutorError::SwapNotFound { .. } => "SWAP_NOT_FOUND",
        }
    }

    fn message(&self) -> &str {
        match self {
            SwapsExecutorError::OneInchClientError(_)
            | SwapsExecutorError::Database(_) => "Internal server error",
            SwapsExecutorError::SwapNotFound { .. } => "Requested swap not found in cache."
        }
    }

    fn http_status_code(&self) -> reqwest::StatusCode {
        match self {
            SwapsExecutorError::OneInchClientError(_)
            | SwapsExecutorError::Database(_) => reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            SwapsExecutorError::SwapNotFound { .. } => reqwest::StatusCode::NOT_FOUND,
        }
    }
}

pub struct SwapsExecutor<D: DaoInterface + 'static> {
    dao: D,
    one_inch_client: OneInchClient,
    orders_cache: OrdersCache,
}

impl<D: DaoInterface + 'static> SwapsExecutor<D> {
    pub fn new(dao: D, one_inch_client: OneInchClient) -> Self {
        Self {
            dao,
            one_inch_client,
            orders_cache: OrdersCache::new(),
        }
    }

    pub async fn get_prices(
        &self,
        chain: OneInchSupportedChain,
        asset_ids: &[Address],
    ) -> Result<GetPricesResponse, SwapsExecutorError> {
        let result = self.one_inch_client
            .get_prices(chain.chain_id(), asset_ids)
            .await?;

        Ok(result.into())
    }

    pub async fn build_order(
        &self,
        request: CreateOneInchSwapData,
    ) -> Result<OneInchPreparedSwap, SwapsExecutorError> {
        let unsigned_order = self.one_inch_client
            .build_order_from_request_data(request, AUCTION_TIME_BUFFER_SECS)
            .await?;

        // TODO: get real decimals, handle parsing error
        let to_amount = Decimal::new(
            unsigned_order.order.taking_amount.parse().unwrap(),
            6,
        );

        let created_at = Utc::now();

        let valid_till = created_at
            .checked_add_signed(
                TimeDelta::seconds(AUCTION_TIME_BUFFER_SECS as i64)
            )
            .unwrap();

        let swap = OneInchPreparedSwap {
            id: Uuid::new_v4(),
            request,
            unsigned_order,
            to_amount,
            created_at,
            valid_till,
        };

        self.orders_cache.add(swap.clone()).await;

        Ok(swap)
    }

    #[tracing::instrument(skip(self, signature))]
    pub async fn submit_order(
        &self,
        swap_id: Uuid,
        invoice_id: Uuid,
        order_hash: B256,
        signature: String,
    ) -> Result<OneInchSwap, SwapsExecutorError> {
        let prepared_swap = self.orders_cache
            .remove(&swap_id)
            .await
            .ok_or(SwapsExecutorError::SwapNotFound {
                swap_id,
            })?;

        let swap = prepared_swap.to_signed(signature);

        let is_cross_chain = swap.is_cross_chain();

        tracing::info!(
            is_cross_chain,
            "Trying to create and submit swap...",
        );

        let swap = self.dao.create_swap(swap).await?;

        let result = if swap.is_cross_chain() {
            self.one_inch_client.submit_cross_order(swap.raw_order).await
        } else {
            self.one_inch_client.submit_intent_order(swap.request.from_chain.chain_id(), swap.raw_order.into()).await
        };

        match result {
            Ok(()) => {
                let swap = self.dao.update_swap_submitted(swap.id).await?;

                tracing::info!(
                    is_cross_chain,
                    "Swap has been created and submitted succesfully"
                );

                Ok(swap)
            },
            Err(e) => {
                tracing::info!(
                    is_cross_chain,
                    error.source = ?e,
                    "Got error while "
                );

                self.dao.update_swap_failed(swap.id, e.to_string()).await?;
                Err(e.into())
            }
        }
    }

    pub fn ignite_background_cache_cleaner(
        &self,
        token: CancellationToken,
    ) -> tokio::task::JoinHandle<()> {
        let cache = self.orders_cache.clone();

        tokio::spawn(async move {
            let mut interval = interval(Duration::from_millis(CACHE_CLEAN_INTERVAL_MILLIS));

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        cache.cleanup().await;
                    },
                    () = token.cancelled() => {
                        // TODO: add logs
                        break
                    }
                }
            }
        })
    }
}

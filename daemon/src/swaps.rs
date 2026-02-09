mod executor;
mod one_inch_client;

use std::collections::HashMap;

use alloy::primitives::B256;
use tokio::time::{Duration, interval};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::dao::{DaoInterface, DaoSwapError};
use crate::swaps::one_inch_client::IntentOrderStatusResponse;
use crate::types::OneInchSwap;

use one_inch_client::{CrossOrderStatusResponse, OrderStatus, OneInchError};

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

    pub fn get_intent_swaps_hashes(&self) -> HashMap<B256, Uuid> {
        self.on_chain_swaps
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

    async fn check_order_status(
        &mut self,
        swap_id: Uuid,
        order_status: OrderStatus
    ) -> Result<(), SwapsMonitorError> {
        match order_status {
            OrderStatus::Executed => {
                self.dao.update_swap_completed(swap_id).await?;
                self.monitored_swaps.remove_cross_swap(&swap_id);
                tracing::info!(
                    "Swap has been executed successfully and marked as Completed"
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
                tracing::trace!("Swap has non-final status, continue monitoring");
            },
        }

        return Ok(());
    }

    #[tracing::instrument(skip(self, order), fields(order_hash = ?order.order_hash))]
    async fn check_monitored_cross_order(
        &mut self,
        swap_id: Uuid,
        order: CrossOrderStatusResponse,
    ) -> Result<(), SwapsMonitorError> {
        self.check_order_status(swap_id, order.status).await?;

        if matches!(order.status, OrderStatus::Pending | OrderStatus::Filled) {
            tracing::trace!("Cross chain order has non-final status, continue monitoring");

            if order.has_dst_deployed_escrow() {
                self.get_ready_to_fill_escrows_and_submit_secrets(swap_id).await?;
            }
        }

        return Ok(());
    }

    async fn check_monitored_cross_orders(&mut self) {
        let hashes_with_ids = self.monitored_swaps.get_cross_swaps_hashes();

        let hashes: Vec<_> = hashes_with_ids.keys().copied().collect();

        tracing::trace!(
            swaps_count = hashes_with_ids.len(),
            swaps_hashes = ?hashes,
            swaps_ids = ?hashes_with_ids.values(),
            "Check cross chain swaps statuses"
        );

        let cross_orders = match self.one_inch_client
            .get_cross_orders_by_hashes(&hashes)
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                tracing::warn!(
                    error.source = ?e,
                    swaps_hashes = ?hashes,
                    swaps_ids = ?hashes_with_ids.values(),
                    "Error while request cross orders by hashes from 1Inch API"
                );

                return;
            }
        };

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

    async fn check_monitored_intent_orders(&mut self) {
        let hashes_with_ids = self.monitored_swaps.get_intent_swaps_hashes();
        // TODO: change it
        let chain_id = 137;
        let hashes: Vec<_> = hashes_with_ids.keys().copied().collect();

        tracing::trace!(
            swaps_count = hashes_with_ids.len(),
            swaps_hashes = ?hashes,
            swaps_ids = ?hashes_with_ids.values(),
            "Check intent swaps statuses"
        );

        let intent_orders = self.one_inch_client
            .get_intent_orders_by_hashes(chain_id, &hashes)
            .await
            // TODO: handle error
            .unwrap();

        for order in intent_orders {
            let order_hash = order.order_hash;

            let swap_id = hashes_with_ids
                .get(&order_hash)
                .copied()
                .unwrap();

            if let Err(e) = self.check_order_status(swap_id, order.status).await {
                tracing::debug!(
                    ?swap_id,
                    ?order_hash,
                    error = ?e,
                    "Error while check intent order"
                );
            };
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

                    if self.monitored_swaps.has_on_chain_swaps() {
                        self.check_monitored_intent_orders().await;
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

#[cfg(test)]
mod tests {
    use alloy::primitives::{Address, address, U256};
    use alloy::network::EthereumWallet;
    use alloy::signers::SignerSync;
    use alloy::providers::ProviderBuilder;
    use alloy::signers::local::PrivateKeySigner;
    use alloy::sol;
    use serde::Deserialize;

    use crate::types::{CreateOneInchSwapParams, PublicOneInchPreparedSwap, PublicOneInchSwap, SubmitOneInchSwapParams};

    use super::*;

    const USDC_BASE: Address = address!("0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913");
    const USDC_POLYGON: Address = address!("0x3c499c542cEF5E3811e1192ce70d8cC03d5c3359");
    const USDT_POLYGON: Address = address!("0xc2132D05D31c914a87C6611C10748AEb04B58e8F");
    const EURC_BASE: Address = address!("0x60a3E35Cc302bFA44Cb288Bc5a4F316Fdb1adb42");
    // const SRC_CHAIN_ID: u64 = 137;  // Polygon
    const SRC_CHAIN_ID: u64 = 8453;  // Base
    const SOURCE_WALLET_ADDRESS: Address = address!("0x0E3Ca7fD040144900AdaA5f9B8917f3933A4F5e9");
    const AMOUNT_FROM_FRONT_END: u128 = 1_000_000; // 1 USDC (6 decimals)
    const BASE_RPC: &str = "https://base-rpc.publicnode.com";
    const POLYGON_RPC: &str = "https://polygon-bor-rpc.publicnode.com";
    const INVOICE_ID: Uuid = uuid::uuid!("3612cb08-cef9-474c-973b-8fa7b72e710f");

    sol! {
        #[sol(rpc)]
        contract IERC20 {
            function allowance(address owner, address spender) external view returns (uint256);
            function approve(address spender, uint256 amount) external returns (bool);
            function balanceOf(address account) external view returns (uint256);
        }
    }

    #[derive(Debug, Clone, Deserialize)]
    struct ApiResponseResult<T> {
        result: T,
    }

    #[tokio::test]
    async fn test_swaps() {
        let rpc_node_url = if SRC_CHAIN_ID == 137 {
            POLYGON_RPC
        } else {
            BASE_RPC
        };

        let request_params = CreateOneInchSwapParams {
            invoice_id: INVOICE_ID,
            from_chain: SRC_CHAIN_ID,
            from_token_address: USDC_BASE,
            from_address: SOURCE_WALLET_ADDRESS,
            from_amount_units: AMOUNT_FROM_FRONT_END,
        };

        let client = reqwest::Client::new();

        let response = client
            .post("http://localhost:8080/public/swap/create")
            .json(&request_params)
            .send()
            .await
            .unwrap();

        let response_text = response.text().await.unwrap();

        println!("Create swap response text: {:#?}", response_text);

        let response = serde_json::from_str::<ApiResponseResult<PublicOneInchPreparedSwap>>(&response_text).unwrap().result;

        println!("Create swap response: {:#?}", response);

        let base_private_key = std::env::var("TEST_BASE_PRIVATE_KEY").unwrap();
        let signer: PrivateKeySigner = base_private_key.parse().unwrap();
        let signature = const_hex::encode_prefixed(&signer.sign_hash_sync(&response.order_hash).unwrap().as_bytes());

        let wallet = EthereumWallet::from(signer.clone());

        let provider = ProviderBuilder::new()
            .wallet(wallet)
            .connect_http(rpc_node_url.parse().unwrap());

        let token_address = IERC20::new(response.from_token_address, &provider);

        let allowance = token_address.allowance(response.from_address, response.verifying_protocol).call().await.unwrap();
        let amount_needed = U256::from(response.from_amount_units);

        if allowance < amount_needed {
            println!("Approving token spending...");

            let tx = token_address.approve(response.verifying_protocol, amount_needed);
            let receipt = tx.send().await.unwrap().get_receipt().await.unwrap();

            println!("Approval tx: {}", receipt.transaction_hash);
            println!("Approval confirmed!");
        } else {
            println!("Sufficient allowance already exists");
        }

        println!("Order signed, allowance permited");
        println!("Sending order hash: {:?}", const_hex::encode_prefixed(response.order_hash));
        let swap_submit_request = SubmitOneInchSwapParams {
            swap_id: response.id,
            invoice_id: response.invoice_id,
            order_hash: response.order_hash,
            signature,
        };

        let swap_submit_response = client
            .post("http://localhost:8080/public/swap/submit")
            .json(&swap_submit_request)
            .send()
            .await
            .unwrap()
            .json::<ApiResponseResult<PublicOneInchSwap>>()
            .await
            .unwrap();

        println!("Swap submit response: {:#?}", swap_submit_response);
    }
}

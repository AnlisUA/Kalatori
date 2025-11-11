//! Everything related to actual interaction with blockchain

use runtime::runtime_types::staging_xcm::v3::multilocation::MultiLocation;
use std::collections::HashMap;
use subxt::config::{Config, DefaultExtrinsicParams, SubstrateConfig};
use subxt::utils::AccountId32;
use subxt_signer::SecretString;
use tokio::{
    sync::{mpsc, oneshot},
    time::{Duration, timeout},
};
use tokio_util::sync::CancellationToken;

#[subxt::subxt(
    runtime_metadata_path = "./metadata.scale",
    generate_docs,
    // derive_for_all_types = "Clone, PartialEq, Eq",
    derive_for_type(
        path = "staging_xcm::v3::multilocation::MultiLocation",
        derive = "Clone, codec::Encode",
        recursive
    )
)]
pub mod runtime {}

// We don't need to construct this at runtime, so an empty enum is appropriate.
#[derive(Debug)]
pub enum AssetHubConfig {}

impl Config for AssetHubConfig {
    type AccountId = <SubstrateConfig as Config>::AccountId;
    type Address = <SubstrateConfig as Config>::Address;
    type Signature = <SubstrateConfig as Config>::Signature;
    type Hasher = <SubstrateConfig as Config>::Hasher;
    type Header = <SubstrateConfig as Config>::Header;
    type ExtrinsicParams = DefaultExtrinsicParams<AssetHubConfig>;
    // Here we use the MultiLocation from the metadata as a part of the config:
    // The `ChargeAssetTxPayment` signed extension that is part of the ExtrinsicParams above, now uses the type:
    type AssetId = MultiLocation;
}

use crate::configs::ChainConfig;
use crate::{
    error::{ChainError, Error},
    legacy_types::OrderInfo,
    state::State,
    utils::task_tracker::TaskTracker,
};

pub mod definitions;
pub mod payout;
pub mod tracker;
pub mod utils;

use crate::legacy_types::{Health, RpcInfo};
use definitions::{ChainRequest, ChainTrackerRequest, WatchAccount};
use tracker::start_chain_watch;

/// Logging filter
#[expect(dead_code)]
pub const MODULE: &str = module_path!();

/// Wait this long before forgetting about stuck chain watcher
const SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(120_000);

pub type AssetHubOnlineClient = subxt::OnlineClient<AssetHubConfig>;

/// RPC server handle
#[derive(Clone, Debug)]
pub struct ChainManager {
    pub tx: mpsc::Sender<ChainRequest>,
}

impl ChainManager {
    /// Run once to start all chain connections; this should be very robust, if manager fails
    /// - all modules should be restarted, probably.
    #[expect(clippy::too_many_lines)]
    pub fn ignite(
        seed_secret: SecretString,
        chain_info: ChainConfig,
        state: &State,
        task_tracker: &TaskTracker,
        cancellation_token: &CancellationToken,
    ) -> Result<Self, Error> {
        let (tx, mut rx) = mpsc::channel(1024);

        // TODO: get rid of this, unnecessery if we use single chain
        let mut watch_chain = HashMap::new();

        let mut currency_map = HashMap::new();

        // Create a channel for receiving RPC status updates
        let (rpc_update_tx, mut rpc_update_rx) = mpsc::channel(1024);

        // start network monitors
        if chain_info.endpoints.is_empty() {
            return Err(Error::EmptyEndpoints(chain_info.name));
        }
        let (chain_tx, chain_rx) = mpsc::channel(1024);
        watch_chain.insert(chain_info.name.clone(), chain_tx.clone());

        for a in &chain_info.assets {
            if currency_map
                .insert(a.name.clone(), chain_info.name.clone())
                .is_some()
            {
                return Err(Error::DuplicateCurrency(a.name.clone()));
            }
        }

        start_chain_watch(
            seed_secret,
            chain_info,
            chain_tx.clone(),
            chain_rx,
            state.interface(),
            task_tracker.clone(),
            cancellation_token.clone(),
            rpc_update_tx.clone(),
        );

        task_tracker
            .clone()
            .spawn("Blockchain connections manager", async move {
                let mut rpc_statuses: HashMap<(String, String), Health> = HashMap::new();

                // start requests engine
                loop {
                    tokio::select! {
                        Some(chain_request) = rx.recv() => {
                            match chain_request {
                                ChainRequest::WatchAccount(request) => {
                                    if let Some(chain) = currency_map.get(&request.currency.currency) {
                                        if let Some(receiver) = watch_chain.get(chain) {
                                            let _unused = receiver
                                                .send(ChainTrackerRequest::WatchAccount(request))
                                                .await;
                                        } else {
                                            let _unused = request
                                                .res
                                                .send(Err(ChainError::InvalidChain(chain.to_string())));
                                        }
                                    } else {
                                        let _unused = request
                                            .res
                                            .send(Err(ChainError::InvalidCurrency(request.currency.currency)));
                                    }
                                }
                                ChainRequest::Reap(request) => {
                                    if let Some(chain) = currency_map.get(&request.currency.currency) {
                                        if let Some(receiver) = watch_chain.get(chain) {
                                            let _unused =
                                                receiver.send(ChainTrackerRequest::Reap(request)).await;
                                        } else {
                                            let _unused = request
                                                .res
                                                .send(Err(ChainError::InvalidChain(chain.to_string())));
                                        }
                                    } else {
                                        let _unused = request
                                            .res
                                            .send(Err(ChainError::InvalidCurrency(request.currency.currency)));
                                    }
                                }
                                ChainRequest::Shutdown(res) => {
                                    for (name, chain) in watch_chain.drain() {
                                        let (oneshot_tx, oneshot_rx) = oneshot::channel();
                                        if chain.send(ChainTrackerRequest::Shutdown(oneshot_tx)).await.is_ok() &&
                                            timeout(SHUTDOWN_TIMEOUT, oneshot_rx).await.is_err()
                                        {
                                            tracing::error!("Chain monitor for {name} took too much time to wind down, probably it was frozen. Discarding it.");
                                        }
                                    }
                                    let _ = res.send(());
                                    break;
                                }
                                ChainRequest::GetConnectedRpcs(res_tx) => {
                                    // Collect the RpcInfo from rpc_statuses
                                    let connected_rpcs: Vec<RpcInfo> = rpc_statuses.iter().map(|((chain_name, rpc_url), status)| {
                                        RpcInfo {
                                            chain_name: chain_name.clone(),
                                            rpc_url: rpc_url.clone(),
                                            status: *status,
                                        }
                                    }).collect();
                                    // TODO: handle error?
                                    drop(res_tx.send(connected_rpcs));
                                }
                            }
                        }
                        Some(rpc_update) = rpc_update_rx.recv() => {
                            rpc_statuses.insert(
                                (rpc_update.chain_name.clone(), rpc_update.rpc_url.clone()),
                                rpc_update.status,
                            );
                        }
                        else => break,
                    }
                }

                Ok("Chain manager is shutting down")
            });

        Ok(Self { tx })
    }

    pub async fn add_invoice(
        &self,
        invoice_id: uuid::Uuid,
        order: OrderInfo,
        recipient: AccountId32,
    ) -> Result<(), ChainError> {
        let (res, rx) = oneshot::channel();
        self.tx
            .send(ChainRequest::WatchAccount(WatchAccount::new(
                invoice_id, order, recipient, res,
            )?))
            .await
            .map_err(|_| ChainError::MessageDropped)?;
        rx.await.map_err(|_| ChainError::MessageDropped)?
    }

    pub async fn get_connected_rpcs(&self) -> Result<Vec<RpcInfo>, Error> {
        let (res_tx, res_rx) = oneshot::channel();
        self.tx
            .send(ChainRequest::GetConnectedRpcs(res_tx))
            .await
            .map_err(|_| Error::Fatal)?;
        res_rx.await.map_err(|_| Error::Fatal)
    }

    pub async fn reap(
        &self,
        invoice_id: uuid::Uuid,
        order: OrderInfo,
        recipient: AccountId32,
    ) -> Result<(), ChainError> {
        let (res, rx) = oneshot::channel();
        self.tx
            .send(ChainRequest::Reap(WatchAccount::new(
                invoice_id, order, recipient, res,
            )?))
            .await
            .map_err(|_| ChainError::MessageDropped)?;
        rx.await.map_err(|_| ChainError::MessageDropped)?
    }

    pub async fn shutdown(&self) -> () {
        let (tx, rx) = oneshot::channel();
        let _unused = self.tx.send(ChainRequest::Shutdown(tx)).await;
        let _ = rx.await;
    }
}

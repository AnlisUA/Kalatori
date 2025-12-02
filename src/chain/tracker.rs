//! A tracker that follows individual chain

use crate::{
    chain::{
        definitions::{ChainTrackerRequest, Invoice},
        payout::payout, utils::to_base58_string,
    },
    chain_client::ClientError,
    configs::ChainConfig, error::ChainError, legacy_types::{CurrencyProperties, Health, RpcInfo, TokenKind, TxKind, TxStatus}, state::State, types::{OutgoingTransactionMeta, Transaction, TransactionOrigin}, utils::task_tracker::TaskTracker
};
use crate::chain_client::{AssetHubClient, BlockChainClient, KeyringClient};
use std::collections::HashMap;
use rust_decimal::Decimal;
use chrono::{DateTime, Utc};
use futures::{StreamExt, pin_mut};
use subxt::utils::AccountId32;
use tokio::{
    sync::mpsc,
    time::{Duration, timeout},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Extract transaction hash from hex-encoded extrinsic bytes
fn extract_tx_hash(transaction_bytes: &str) -> Option<String> {
    // Remove 0x prefix if present
    let bytes_str = transaction_bytes
        .strip_prefix("0x")
        .unwrap_or(transaction_bytes);

    // Decode hex to bytes
    let bytes = const_hex::decode(bytes_str).ok()?;

    // Calculate blake2 256-bit hash (standard for Substrate tx hashes)
    let mut hasher = blake2b_simd::Params::new().hash_length(32).to_state();
    hasher.update(&bytes);
    let hash = hasher.finalize();

    // Return as 0x-prefixed hex string
    Some(format!("0x{}", const_hex::encode(hash.as_bytes())))
}

/// Convert `TxKind` to `TransactionType`
fn tx_kind_to_transaction_type(kind: TxKind) -> crate::types::TransactionType {
    match kind {
        TxKind::Payment => crate::types::TransactionType::Incoming,
        TxKind::Withdrawal => crate::types::TransactionType::Outgoing,
    }
}

/// Convert `TxStatus` to `TransactionStatus`
fn tx_status_to_transaction_status(status: TxStatus) -> crate::types::TransactionStatus {
    match status {
        TxStatus::Pending => crate::types::TransactionStatus::Waiting,
        TxStatus::Finalized => crate::types::TransactionStatus::Completed,
        TxStatus::Failed => crate::types::TransactionStatus::Failed,
    }
}

/// Convert `f64` amount to `Decimal`
fn amount_to_decimal(amount: f64) -> rust_decimal::Decimal {
    // This should not fail for normal f64 values
    Decimal::try_from(amount).unwrap_or_else(|e| {
        tracing::error!("Failed to convert amount {amount} to Decimal: {e}");
        Decimal::ZERO
    })
}

/// Parse RFC3339 timestamp string to `DateTime<Utc>`
fn parse_timestamp(timestamp_str: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(timestamp_str)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|e| {
            tracing::error!("Failed to parse timestamp {timestamp_str}: {e}");
            Utc::now()
        })
}

/// Build a Transaction object from the available data
#[expect(clippy::too_many_arguments)]
fn build_transaction(
    invoice_id: Uuid,
    asset_id: u32,
    chain: String,
    amount: Decimal,
    sender: AccountId32,
    recipient: AccountId32,
    block_number: u32,
    position_in_block: u32,
    transaction_bytes: String,
    timestamp: u64,
    tx_kind: TxKind,
    tx_status: TxStatus,
) -> Transaction {
    let tx_hash = extract_tx_hash(&transaction_bytes);
    let transaction_type = tx_kind_to_transaction_type(tx_kind);
    let created_at = DateTime::from_timestamp_millis(timestamp as i64).unwrap_or_else(|| Utc::now());
    let status = tx_status_to_transaction_status(tx_status);
    let sender = to_base58_string(sender.0, 42);
    let recipient = to_base58_string(recipient.0, 42);

    Transaction {
        id: Uuid::new_v4(),
        invoice_id,
        asset_id,
        chain,
        amount,
        sender,
        recipient,
        block_number: Some(block_number),
        position_in_block: Some(position_in_block),
        tx_hash,
        origin: TransactionOrigin::default(), // No origin for detected payments
        status,
        transaction_type,
        outgoing_meta: OutgoingTransactionMeta::default(),
        created_at,
        transaction_bytes: Some(transaction_bytes),
    }
}

#[expect(clippy::too_many_lines, clippy::too_many_arguments)]
pub fn start_chain_watch(
    keyring_client: KeyringClient,
    chain: ChainConfig,
    chain_tx: mpsc::Sender<ChainTrackerRequest>,
    mut chain_rx: mpsc::Receiver<ChainTrackerRequest>,
    state: State,
    task_tracker: TaskTracker,
    cancellation_token: CancellationToken,
    rpc_update_tx: mpsc::Sender<RpcInfo>,
) {
    task_tracker
        .clone()
        .spawn(format!("Chain {} watcher", chain.name.clone()), async move {
            let watchdog = 120_000;
            let mut watched_accounts: HashMap<Uuid, Invoice> = HashMap::new();
            let mut shutdown = false;

            if chain.allow_insecure_endpoints {
                tracing::warn!("Connection to insecure endpoints allowed! It's strongly unrecommended to use this option in production environment.");
            }

            for endpoint in chain.endpoints.iter().cycle() {
                // not restarting chain if shutdown is in progress
                if shutdown || cancellation_token.is_cancelled() {
                    tracing::info!("Received {} signal, shut down ChainWatch", if shutdown { "shutdown" } else { "task cancellation" });
                    break;
                }

                // TODO: handle error?
                drop(rpc_update_tx.send(RpcInfo {
                    chain_name: chain.name.clone(),
                    rpc_url: endpoint.clone(),
                    status: Health::Degraded,
                }).await);

                tracing::info!("Trying to establish connection to endpoint {:?}...", endpoint);

                let assets: Vec<_> = chain.assets
                    .iter()
                    .map(|asset| asset.id)
                    .collect();

                let asset_hub_client_result: Result<_, ClientError> = async {
                    let client = AssetHubClient::new(&chain).await?;
                    client.init_asset_info(&assets).await?;

                    Ok(client)
                }.await;

                let asset_hub_client = match asset_hub_client_result {
                    Ok(client) => client,
                    Err(error) => {
                        tracing::error!("Error while initialize asset hub WS client for endpoint {:?}: {:?}", endpoint, error);

                        drop(rpc_update_tx.send(RpcInfo {
                            chain_name: chain.name.clone(),
                            rpc_url: endpoint.clone(),
                            status: Health::Critical,
                        }).await);

                        continue
                    }
                };

                tracing::info!("Connection to endpoint {:?} established, start watching", endpoint);
                // TODO: handle error?
                drop(rpc_update_tx.send(RpcInfo {
                    chain_name: chain.name.clone(),
                    rpc_url: endpoint.clone(),
                    status: Health::Ok,
                }).await);

                // prepare chain
                let watcher = match ChainWatcher::prepare_chain(
                    &asset_hub_client,
                    chain.clone(),
                    &mut watched_accounts,
                    endpoint,
                    state.interface(),
                )
                    .await
                {
                    Ok(a) => a,
                    Err(e) => {
                        tracing::warn!(
                            "Failed to connect to chain {}, due to {} switching RPC server...",
                            chain.name,
                            e
                        );
                        continue;
                    }
                };

                tracing::info!("Start monitoring on {} rpc", endpoint);

                let transfers_sub_result = asset_hub_client.subscribe_transfers(&assets).await;

                let transfers_sub = match transfers_sub_result {
                    Ok(sub) => sub,
                    Err(e) => {
                        tracing::warn!(
                            "Failed to subscribe blocks using asset hub client on {}, due to {} switching RPC server...",
                            chain.name,
                            e
                        );
                        continue;
                    }
                };

                pin_mut!(transfers_sub);

                tracing::info!("Start monitoring...");
                // fulfill requests
                while let Ok(Some(req)) =
                    timeout(Duration::from_millis(watchdog), async {
                        tokio::select! {
                            transfer = transfers_sub.next() => {
                                transfer
                                    .map(|result| result
                                        .inspect_err(|e| tracing::warn!("Got error in tranfsers subscription: {:?}", e))
                                        .ok()
                                        .map(ChainTrackerRequest::Transfers)
                                    )
                                    .flatten()
                            },
                            req = chain_rx.recv() => {
                                req
                            }
                        }
                    }).await
                {
                    match req {
                        ChainTrackerRequest::Transfers(transfers) => {
                            tracing::debug!("Got transfers for processing: {:?}", transfers);
                            let mut id_remove_list = Vec::new();

                            for transfer in transfers {
                                for (id, invoice) in &watched_accounts {
                                    if invoice.address == transfer.recipient {
                                        let transaction = build_transaction(
                                            *id,
                                            transfer.asset_id,
                                            invoice.currency.chain_name.clone(),
                                            transfer.amount,
                                            transfer.sender.clone(),
                                            transfer.recipient.clone(),
                                            transfer.transaction_id.0,
                                            transfer.transaction_id.1,
                                            String::new(),
                                            transfer.timestamp,
                                            TxKind::Payment,
                                            TxStatus::Finalized,
                                        );

                                        state.record_transaction_v2(*id, transaction).await?;
                                    }
                                }
                            }

                            let now = Utc::now().timestamp_millis() as u64;

                            for (id, invoice) in &watched_accounts {
                                match invoice.check(&asset_hub_client, &watcher).await {
                                    Ok(true) => {
                                        state.order_paid(id.clone()).await;
                                    },
                                    Err(e) => {
                                        tracing::warn!("account fetch error: {0:?}", e);
                                    }
                                    _ => {}
                                }

                                if invoice.death.0 <= now {
                                    match state.is_order_paid(id.clone()).await {
                                        Ok(paid_db) => {
                                            if !paid_db {
                                                match invoice.check(&asset_hub_client, &watcher).await {
                                                    Ok(paid) => {
                                                        if paid {
                                                            state.order_paid(id.clone()).await;
                                                        }
                                                    }
                                                    Err(e) => {
                                                        tracing::warn!("account fetch error: {0:?}", e);
                                                    }
                                                }
                                            }

                                            tracing::debug!("Removing an account {id:?} due to passing its death timestamp");
                                            id_remove_list.push(id.to_owned());
                                        }
                                        Err(e) => {
                                            tracing::warn!("account read error: {e:?}");
                                        }
                                    }
                                }
                            }

                            for id in id_remove_list {
                                watched_accounts.remove(&id);
                            };
                        }
                        ChainTrackerRequest::WatchAccount(request) => {
                            watched_accounts.insert(request.id.clone(), Invoice::from_request(request));
                        }
                        ChainTrackerRequest::Reap(request) => {
                            let id = request.id.clone();
                            let reap_state_handle = state.interface();
                            let watcher_for_reaper = watcher.clone();
                            let keyring_client_cloned = keyring_client.clone();
                            let client_cloned = asset_hub_client.clone();

                            task_tracker.clone().spawn(format!("Initiate payout for order {}", id.clone()), async move {
                                let () = payout(
                                    client_cloned,
                                    Invoice::from_request(request),
                                    reap_state_handle,
                                    watcher_for_reaper,
                                    keyring_client_cloned,
                                ).await?;

                                Ok(format!("Payout attempt for order {id} terminated"))
                            });
                        }
                        ChainTrackerRequest::ForceReap(request) => {
                            let id = request.id.clone();
                            let reap_state_handle = state.interface();
                            let watcher_for_reaper = watcher.clone();
                            let keyring_client_cloned = keyring_client.clone();
                            let client_cloned = asset_hub_client.clone();

                            task_tracker.clone().spawn(format!("Initiate forced payout for order {}", id.clone()), async move {
                                let () = payout(
                                    client_cloned,
                                    Invoice::from_request(request),
                                    reap_state_handle,
                                    watcher_for_reaper,
                                    keyring_client_cloned,
                                ).await?;

                                Ok(format!("Forced payout attempt for order {id} terminated"))
                            });
                        }
                        ChainTrackerRequest::Shutdown(res) => {
                            shutdown = true;
                            let _ = res.send(());
                            break;
                        }
                    }
                };
            }

            Ok(format!("Chain {} monitor shut down", chain.name))
        });
}

#[derive(Debug, Clone)]
pub struct ChainWatcher {
    pub assets: HashMap<String, CurrencyProperties>,
    // TODO: version parameter removed. Earlier it was checked in each block.
    // Subxt docs recommends use updater() for similiar purpose, need to implement
    // https://docs.rs/subxt/latest/subxt/client/struct.OnlineClient.html#method.updater
}

impl ChainWatcher {
    #[expect(clippy::too_many_arguments)]
    pub async fn prepare_chain(
        client: &AssetHubClient,
        chain: ChainConfig,
        watched_accounts: &mut HashMap<Uuid, Invoice>,
        rpc_url: &str,
        state: State,
    ) -> Result<Self, ChainError> {
        let name = client.chain_name();

        if name != chain.name {
            return Err(ChainError::WrongNetwork {
                expected: chain.name,
                actual: name.to_string(),
                rpc: rpc_url.to_string(),
            });
        }

        // TODO: in future we plan to use single asset, won't need to iterate over all of them.
        // It can be optimized using futures::iter and request values concurrently.
        // Also if we'll need to fetch many assets (or even all available on chain)
        // it's gonna be easier to use `metadata_iter` storage method
        let mut assets = HashMap::new();

        // TODO: add check that there is at least one asset? Seems to be better have that check on config validation
        for asset in chain.assets {
            let response = client
                .asset_info_store()
                .get_asset_info(&asset.id)
                .await
                // unwrap is safe here cause we already initialized those assets right before this function call
                .unwrap();

            let properties = CurrencyProperties {
                chain_name: chain.name.clone(),
                kind: TokenKind::Asset, // TODO: this field can be removed in future as long as we work only with assets on Asset Hub
                decimals: response.decimals,
                rpc_url: rpc_url.to_string(), // TODO: this property seems to be unused
                asset_id: Some(asset.id),
                ss58: 0, // TODO: this property seems to be unused
            };

            assets.insert(asset.name, properties);
        }
        // this MUST assert that assets match exactly before reporting it

        state.connect_chain(assets.clone()).await;

        let chain_watcher = ChainWatcher { assets };

        // check monitored accounts
        let mut id_remove_list = Vec::new();
        for (id, account) in watched_accounts.iter() {
            let result = account.check(client, &chain_watcher).await;

            match result {
                Ok(true) => {
                    state.order_paid(id.clone()).await;
                    id_remove_list.push(id.to_owned());
                }
                Ok(false) => (),
                Err(e) => {
                    tracing::warn!("account fetch error: {0}", e);
                }
            }
        }

        for id in id_remove_list {
            watched_accounts.remove(&id);
        }

        Ok(chain_watcher)
    }
}

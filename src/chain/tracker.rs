//! A tracker that follows individual chain

use crate::{
    chain::{
        AssetHubConfig, AssetHubOnlineClient,
        definitions::{ChainTrackerRequest, Invoice},
        payout::payout,
        utils::to_base58_string,
    },
    chain_client::{AssetHubChainConfig, ChainTransfer, ChainResult},
    configs::ChainConfig, definitions::Balance, error::ChainError, legacy_types::{CurrencyProperties, Health, RpcInfo, TokenKind, TxKind, TxStatus}, state::State, types::{OutgoingTransactionMeta, Transaction, TransactionOrigin}, utils::task_tracker::TaskTracker
};
use crate::chain_client::{PolkadotAssetHubClient, BlockChainClient};
use std::{collections::HashMap, time::SystemTime};
use std::str::FromStr;
use rust_decimal::Decimal;
use chrono::{DateTime, Utc};
use futures::{StreamExt, pin_mut};
use subxt::blocks::Block;
use subxt::blocks::{ExtrinsicDetails, FoundExtrinsic};
use subxt::utils::AccountId32;
use subxt_signer::SecretString;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tokio::{
    sync::mpsc,
    time::{Duration, timeout},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use zeroize::Zeroize;

type Extrinsics = subxt::blocks::Extrinsics<AssetHubConfig, AssetHubOnlineClient>;
type TransferExtrinsic = crate::chain::runtime::assets::calls::types::Transfer;
type TransferAllExtrinsic = crate::chain::runtime::assets::calls::types::TransferAll;
type TransferredEvent = crate::chain::runtime::assets::events::Transferred;

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
    sender: String,
    recipient: String,
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

enum AnyTransferExtrinsic {
    Transfer(FoundExtrinsic<AssetHubConfig, AssetHubOnlineClient, TransferExtrinsic>),
    TransferAll(FoundExtrinsic<AssetHubConfig, AssetHubOnlineClient, TransferAllExtrinsic>),
}

impl AnyTransferExtrinsic {
    pub fn details(&self) -> &ExtrinsicDetails<AssetHubConfig, AssetHubOnlineClient> {
        match self {
            AnyTransferExtrinsic::Transfer(e) => &e.details,
            AnyTransferExtrinsic::TransferAll(e) => &e.details,
        }
    }
}

async fn transfer_events(
    block: &Block<AssetHubConfig, AssetHubOnlineClient>,
) -> Result<(u64, Extrinsics), subxt::Error> {
    let timestamp_address = crate::chain::runtime::storage().timestamp().now();

    let timestamp = block
        .storage()
        .fetch(&timestamp_address)
        .await?
        .ok_or_else(|| subxt::Error::Other("Timestamp is empty".into()))?;

    let extrinsics = block.extrinsics().await?;

    Ok((timestamp, extrinsics))
}

async fn parse_transfer_event(
    account_id: &AccountId32,
    extrinsic: &AnyTransferExtrinsic,
) -> Option<(TxKind, AccountId32, Balance)> {
    let acc_id = subxt::utils::AccountId32::from(account_id.0);
    let events = extrinsic.details().events().await.ok()?;

    let mut found_events = events.find::<TransferredEvent>().filter_map(Result::ok);

    found_events.find_map(|event| {
        // if event.from == acc_id {
        //     Some((TxKind::Withdrawal, event.to, Balance(event.amount)))
        // } else if event.to == acc_id {
        //     Some((TxKind::Payment, event.from, Balance(event.amount)))
        // } else {
        //     None
        // }

        if event.to == acc_id {
            Some((TxKind::Payment, event.from, Balance(event.amount)))
        } else {
            None
        }
    })
}

#[expect(clippy::too_many_lines, clippy::too_many_arguments)]
pub fn start_chain_watch(
    mut seed_secret: SecretString,
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

                let subxt_client_initializer = if chain.allow_insecure_endpoints {
                    AssetHubOnlineClient::from_insecure_url(endpoint).await
                } else {
                    AssetHubOnlineClient::from_url(endpoint).await
                };

                let subxt_client = match subxt_client_initializer {
                    Ok(client) => client,
                    Err(error) => {
                        tracing::error!("Error while initialize subxt WS client for endpoint {:?}: {:?}", endpoint, error);

                        drop(rpc_update_tx.send(RpcInfo {
                            chain_name: chain.name.clone(),
                            rpc_url: endpoint.clone(),
                            status: Health::Critical,
                        }).await);

                        continue
                    }
                };

                let assets: Vec<_> = chain.assets
                    .iter()
                    .map(|asset| asset.id)
                    .collect();

                let asset_hub_client_result: Result<_, crate::chain_client::ChainError> = async {
                    let client = PolkadotAssetHubClient::new(&chain).await?;
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
                    &subxt_client,
                    chain.clone(),
                    &mut watched_accounts,
                    endpoint,
                    chain_tx.clone(),
                    state.interface(),
                    task_tracker.clone(),
                    cancellation_token.clone(),
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
                        let req = tokio::select! {
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
                        };

                        if req.is_none() {
                            tracing::info!("Got None req");
                        } else {
                            tracing::info!("Got some request in tokio select");
                        }

                        req
                    }).await
                {
                    tracing::info!("Got request for processing");

                    match req {
                        ChainTrackerRequest::NewBlock(block) => {
                            let block_hash = block.hash();
                            tracing::debug!("Block hash {} from {}", block_hash, chain.name);
                            tracing::debug!("Watched accounts: {watched_accounts:?}");

                            #[expect(clippy::cast_possible_truncation)]
                            let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis() as u64;

                            let mut id_remove_list = Vec::new();

                            match transfer_events(&block).await {
                                Ok((timestamp, extrinsics)) => {
                                    tracing::debug!("Got a block with timestamp {timestamp:?} & {} extrinsics", extrinsics.len());

                                    // TODO: handle Err results? Log them at least?
                                    let transfer_extrinsics = extrinsics.find::<TransferExtrinsic>()
                                        .filter_map(Result::ok)
                                        .map(AnyTransferExtrinsic::Transfer);

                                    let transfer_all_extrinsics = extrinsics.find::<TransferAllExtrinsic>()
                                        .filter_map(Result::ok)
                                        .map(AnyTransferExtrinsic::TransferAll);

                                    let all_transfer_extrinsics: Vec<_> = transfer_extrinsics.chain(transfer_all_extrinsics).collect();

                                    // TODO: Current implementation is quite unoptimized for work with subxt, need to be refactored
                                    for (id, invoice) in &watched_accounts {
                                        for extrinsic in &all_transfer_extrinsics {
                                            if let Some((tx_kind, another_account, transfer_amount)) = parse_transfer_event(&invoice.address, extrinsic).await {
                                                tracing::debug!("Found {tx_kind:?} from/to {another_account:?} with {transfer_amount:?} token(s).");
                                                let position_in_block = extrinsic.details().index();
                                                let raw_extrinsic = extrinsic.details().bytes();

                                                let status = TxStatus::Finalized;
                                                let transaction_bytes = const_hex::encode_prefixed(raw_extrinsic);
                                                let amount_f64 = transfer_amount.format(invoice.currency.decimals);

                                                // Extract asset_id from currency
                                                let asset_id = invoice.currency.asset_id.ok_or_else(|| {
                                                    ChainError::InvalidCurrency(invoice.currency.currency.clone())
                                                })?;

                                                let (sender, recipient) = match tx_kind {
                                                    TxKind::Payment => (
                                                        to_base58_string(another_account.0, 42),
                                                        to_base58_string(invoice.address.0, 42),
                                                    ),
                                                    TxKind::Withdrawal => (
                                                        to_base58_string(invoice.address.0, 42),
                                                        to_base58_string(another_account.0, 42),
                                                    ),
                                                };

                                                let transaction = build_transaction(
                                                    *id,
                                                    asset_id,
                                                    invoice.currency.chain_name.clone(),
                                                    amount_to_decimal(amount_f64),
                                                    sender,
                                                    recipient,
                                                    block.number(),
                                                    position_in_block,
                                                    transaction_bytes,
                                                    timestamp,
                                                    tx_kind,
                                                    status,
                                                );

                                                state.record_transaction_v2(*id, transaction).await?;
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!("events fetch error: {0:?}", e);
                                }
                            }

                            // Important! There used to be a significant oprimisation that
                            // watched events and checked only accounts that have tranfers into
                            // them in given block; this was found to be unreliable: there are
                            // ways to transfer funds without emitting a transfer event (one
                            // notable example is through asset exchange procedure directed
                            // straight into invoice account), and probably even without any
                            // reliably expected event (through XCM). Thus we just scan all
                            // accounts, every time. Please submit a PR or an issue if you
                            // figure out a reliable optimization for this.
                            for (id, invoice) in &watched_accounts {
                                match invoice.check(&subxt_client, &watcher).await {
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
                                                match invoice.check(&subxt_client, &watcher).await {
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

                            tracing::debug!("Block {} from {} processed successfully", block_hash, chain.name);
                        }
                        ChainTrackerRequest::Transfers(transfers) => {
                            tracing::debug!("Got transfers for processing: {:?}", transfers);
                            let mut id_remove_list = Vec::new();

                            for transfer in transfers {
                                for (id, invoice) in &watched_accounts {
                                    if invoice.address == AccountId32::from_str(&transfer.recipient).unwrap() {
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
                                match invoice.check(&subxt_client, &watcher).await {
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
                                                match invoice.check(&subxt_client, &watcher).await {
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
                            let seed = seed_secret.clone();
                            let client_cloned = subxt_client.clone();

                            task_tracker.clone().spawn(format!("Initiate payout for order {}", id.clone()), async move {
                                let () = payout(
                                    client_cloned,
                                    Invoice::from_request(request),
                                    reap_state_handle,
                                    watcher_for_reaper,
                                    seed,
                                ).await?;

                                Ok(format!("Payout attempt for order {id} terminated"))
                            });
                        }
                        ChainTrackerRequest::ForceReap(request) => {
                            let id = request.id.clone();
                            let reap_state_handle = state.interface();
                            let watcher_for_reaper = watcher.clone();
                            let client_cloned = subxt_client.clone();
                            let seed = seed_secret.clone();

                            task_tracker.clone().spawn(format!("Initiate forced payout for order {}", id.clone()), async move {
                                let () = payout(
                                    client_cloned,
                                    Invoice::from_request(request),
                                    reap_state_handle,
                                    watcher_for_reaper,
                                    seed,
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

            seed_secret.zeroize();

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
        client: &AssetHubOnlineClient,
        chain: ChainConfig,
        watched_accounts: &mut HashMap<Uuid, Invoice>,
        rpc_url: &str,
        chain_tx: mpsc::Sender<ChainTrackerRequest>,
        state: State,
        task_tracker: TaskTracker,
        cancellation_token: CancellationToken,
    ) -> Result<Self, ChainError> {
        // Have to perform separate call to get spec name cause `client.runtime_version()` returns a struct
        // which doesn't contain that info. Please watch out for a possible subxt update that may add it.
        let version_call = crate::chain::runtime::apis().core().version();

        let name = client
            .runtime_api()
            .at_latest()
            .await?
            .call(version_call)
            .await?
            .spec_name;

        if name != chain.name {
            return Err(ChainError::WrongNetwork {
                expected: chain.name,
                actual: name,
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
            let request_data = crate::chain::runtime::storage().assets().metadata(asset.id);

            let Some(response) = client
                .storage()
                .at_latest()
                .await?
                .fetch(&request_data)
                .await?
            else {
                // TODO: panic or work without this asset? Need to notify user about error somehow
                panic!(
                    "Asset {} with id {} not found on chain {}",
                    asset.name, asset.id, chain.name
                )
            };

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

        let rpc = rpc_url.to_string();
        let mut blocks = client.blocks().subscribe_finalized().await?;

        task_tracker.spawn(format!("watching blocks at {rpc}"), async move {
            tracing::info!("Start watching blocks task for {:?}", rpc);

            // TODO: task doesn't terminate cause not listen for the termination signal
            loop {
                tokio::select! {
                    () = cancellation_token.cancelled() => {
                        tracing::info!("Received task cancellation signal, shut down ChainWatch");
                        break
                    },
                    received_block = blocks.next() => {
                        let next_block = {
                            received_block.ok_or_else(|| ChainError::BlockSubscriptionTerminated)?
                        };

                        match next_block {
                            Ok(block) => {
                                let block_number = block.number();
                                tracing::debug!("received block {block_number} from {rpc}");

                                // let result = chain_tx
                                //     .send(ChainTrackerRequest::NewBlock(block))
                                //     .await;

                                // if let Err(e) = result {
                                //     tracing::warn!(
                                //         "Block watch internal communication error: {e} at {rpc}"
                                //     );
                                //     break;
                                // }
                            }
                            Err(e) => {
                                tracing::warn! {"Block watch error: {e} at {rpc}"};
                                break;
                            }
                        }
                    }
                }
            }
            // this should reset chain monitor on timeout;
            // but if this breaks, it means that the latter is already down either way
            Ok(format!("Block watch at {rpc} stopped"))
        });

        Ok(chain_watcher)
    }
}

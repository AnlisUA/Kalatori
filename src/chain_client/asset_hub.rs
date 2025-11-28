use std::collections::HashMap;

use rust_decimal::prelude::{Decimal, ToPrimitive};
use futures::{stream, StreamExt};
use subxt::{Config, SubstrateConfig};
use subxt::blocks::Block;
use subxt::blocks::{ExtrinsicDetails, FoundExtrinsic};
use subxt::config::{DefaultExtrinsicParams, DefaultExtrinsicParamsBuilder};
use subxt::utils::H256;
use tracing::{Level, info, instrument, warn, debug};

use crate::chain_client::Encodeable;

use super::{
    AssetInfoStore,
    ChainConfig,
    ChainError,
    BlockChainClient,
    ChainResult,
    AssetInfo,
    ChainTransfer,
    KeyringClient,
    UnsignedTransaction,
    SignedTransaction,
    TransactionResult,
    TransactionError,
};

use super::keyring::SignTransactionRequestData;

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

use runtime::runtime_types::staging_xcm::v3::multilocation::MultiLocation;
use runtime::runtime_types::xcm::v3::{junction::Junction, junctions::Junctions};

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

type AssetHubOnlineClient = subxt::OnlineClient<AssetHubConfig>;

// Runtime type aliases for Asset Hub transfer operations
type TransferExtrinsic = runtime::assets::calls::types::Transfer;
type TransferAllExtrinsic = runtime::assets::calls::types::TransferAll;
type TransferredEvent = runtime::assets::events::Transferred;

pub type AssetHubUnsignedTransaction = subxt::tx::PartialTransaction<AssetHubConfig, AssetHubOnlineClient>;
pub type AssetHubSignedTransaction = subxt::tx::SubmittableTransaction<AssetHubConfig, AssetHubOnlineClient>;
pub type AssetHubAccountId = subxt::utils::AccountId32;

impl Encodeable for AssetHubSignedTransaction {
    fn to_hex_string(&self) -> String {
        const_hex::encode_prefixed(self.encoded())
    }
}

#[derive(Debug, Clone)]
pub enum AssetHubChainConfig {}

impl ChainConfig for AssetHubChainConfig {
    type AssetId = u32;
    type TransactionId = (u32, u32); // (block number, position in block)
    type TransactionHash = H256;
    type BlockHash = H256;
    type UnsignedTransaction = AssetHubUnsignedTransaction;
    type SignedTransaction = AssetHubSignedTransaction;
    type AccountId = AssetHubAccountId;
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

#[derive(Clone)]
pub struct PolkadotAssetHubClient {
    client: AssetHubOnlineClient,
    asset_info_store: AssetInfoStore<AssetHubChainConfig>,
}

impl PolkadotAssetHubClient {
    async fn from_config(
        config: &crate::configs::ChainConfig,
        asset_info_store: AssetInfoStore<AssetHubChainConfig>,
    ) -> ChainResult<Self> {
        // TODO: change error
        // TODO: get random endpoint
        // TODO: implement circuit breaker for endpoints
        // (should be another wrapper structure with endpoints hidden behind sync primitives with error counters and usage timeouts)
        let client = if config.allow_insecure_endpoints {
            AssetHubOnlineClient::from_insecure_url(config.endpoints.first().unwrap()).await
        } else {
            AssetHubOnlineClient::from_url(config.endpoints.first().unwrap()).await
        }.unwrap();

        Ok(PolkadotAssetHubClient {
            client,
            asset_info_store,
        })
    }

    async fn process_block(
        &self,
        block: Block<AssetHubConfig, AssetHubOnlineClient>,
        assets: &HashMap<u32, AssetInfo<AssetHubChainConfig>>,
    ) -> ChainResult<Vec<ChainTransfer<AssetHubChainConfig>>> {
        // Implementation for processing a block
        let block_number = block.number();

        // Extract timestamp from storage
        // TODO: return current timestamp in case of failure, not 0
        let timestamp = match block.storage().fetch(
            &runtime::storage().timestamp().now()
        ).await {
            Ok(Some(ts)) => ts,
            Ok(None) => {
                tracing::warn!("Block {block_number} missing timestamp, using 0");
                0
            }
            Err(e) => {
                tracing::warn!("Failed to fetch timestamp for block {block_number}: {e}");
                0
            }
        };

        // Get extrinsics
        let extrinsics = match block.extrinsics().await {
            Ok(e) => e,
            Err(e) => {
                tracing::error!("Failed to fetch extrinsics for block {block_number}: {e}");
                return Err(ChainError::ExtrinsicsFetchFailed);
            }
        };

        // Find transfer and transfer_all extrinsics
        // TODO: Handle errors in decoding extrinsics
        let transfer_extrinsics = extrinsics.find::<TransferExtrinsic>()
            .filter_map(Result::ok)
            .map(AnyTransferExtrinsic::Transfer);

        let transfer_all_extrinsics = extrinsics.find::<TransferAllExtrinsic>()
            .filter_map(Result::ok)
            .map(AnyTransferExtrinsic::TransferAll);

        let all_transfer_extrinsics = transfer_extrinsics.chain(transfer_all_extrinsics);

        let events = stream::iter(all_transfer_extrinsics)
            .filter_map(|ext| async move {
                let index = ext.details().index();

                ext.details()
                    .events()
                    .await
                    .ok()
                    .map(|evs| (index, evs))
            })
            .collect::<Vec<_>>()
            .await;

        let transfers = events
            .into_iter()
            .flat_map(|(index, events)| {
                events.find::<TransferredEvent>()
                    .filter_map(Result::ok)
                    .filter_map(|event| {
                        let Some(asset_info) = assets.get(&event.asset_id) else {
                            return None
                        };

                        let transaction_bytes = String::new(); // Placeholder

                        Some(ChainTransfer {
                            asset_id: event.asset_id,
                            amount: Decimal::new(event.amount as i64, asset_info.decimals as u32),
                            sender: event.from,
                            recipient: event.to,
                            transaction_id: (block_number, index),
                            transaction_bytes,
                            timestamp,
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .collect();

        Ok(transfers)
    }
}

impl BlockChainClient<AssetHubChainConfig> for PolkadotAssetHubClient {
    // TODO: need to add validation on startup.
    // Iterate over all provided RPC URLs and ensure they all belongs to the configured chain
    fn chain_name(&self) -> &'static str {
        "statemint"
    }

    fn asset_info_store(&self) -> &AssetInfoStore<AssetHubChainConfig> {
        &self.asset_info_store
    }

    async fn new(config: &crate::configs::ChainConfig) -> ChainResult<Self> {
        PolkadotAssetHubClient::from_config(config, AssetInfoStore::new()).await
    }

    async fn new_with_store(
        config: &crate::configs::ChainConfig,
        asset_info_store: AssetInfoStore<AssetHubChainConfig>,
    ) -> ChainResult<Self> {
        PolkadotAssetHubClient::from_config(config, asset_info_store).await
    }

    #[instrument(skip(self))]
    async fn fetch_asset_info(&self, asset_id: &u32) -> ChainResult<AssetInfo<AssetHubChainConfig>> {
        debug!(message = "Trying to fetch asset info...");
        let request_data = runtime::storage().assets().metadata(*asset_id);

        // TODO: change errors
        self.client
            .storage()
            .at_latest()
            .await
            .inspect_err(|error| tracing::warn!(
                message = "Received an error while request storage",
                ?error
            ))
            .map_err(|e| ChainError::BlockFetchFailed)?
            .fetch(&request_data)
            .await
            .inspect_err(|error| tracing::warn!(
                message = "Received an error while request asset metadata",
                ?error
            ))
            .map_err(|e| ChainError::BlockFetchFailed)?
            .ok_or_else(|| ChainError::BlockFetchFailed)
            .inspect_err(|_| warn!(
                message = "Asset metadata wasn't found (None returned)"
            ))
            .map(|resp| {
                AssetInfo {
                    id: *asset_id,
                    name: String::from_utf8(resp.symbol.0).unwrap(),
                    decimals: resp.decimals,
                }
            })
            .inspect(|val| debug!(message = "Asset info fetched successfully", asset_info = ?val))
    }

    // TODO: replace account id with generic
    // TODO: replace errors
    // TODO: probably will be better to return some `Balance` structure with asset id and account id
    async fn fetch_asset_balance(
        &self,
        asset_id: &u32,
        account_id: &AssetHubAccountId,
    ) -> ChainResult<Decimal> {
        let decimals = self.asset_info_store
            .get_asset_info(asset_id)
            .await
            .ok_or_else(|| ChainError::BlockFetchFailed)?
            .decimals;

        let request_data = runtime::storage()
            .assets()
            .account(*asset_id, account_id.clone());

        let amount = self.client
            .storage()
            .at_latest()
            .await
            .inspect_err(|e| {

            })
            .map_err(|e| ChainError::BlockFetchFailed)?
            .fetch(&request_data)
            .await
            .inspect_err(|e| {

            })
            .map_err(|e| ChainError::BlockFetchFailed)?
            .map_or(Decimal::ZERO, |acc| Decimal::new(acc.balance as i64, decimals as u32));

        Ok(amount)
    }

    async fn subscribe_transfers(
        &self,
        asset_ids: &[u32],
    ) -> ChainResult<impl stream::Stream<Item = ChainResult<Vec<ChainTransfer<AssetHubChainConfig>>>>> {
        let client = self.clone();

        let assets = self.asset_info_store.get_assets_info(asset_ids).await;
        // TODO: check if all required assets_ids are presented in `assets` map. Return an error if they're not

        // Subscribe to finalized blocks
        let mut blocks = client.client
            .blocks()
            .subscribe_finalized()
            .await
            .map_err(|e| ChainError::BlockSubscriptionFailed)?;

        let stream = async_stream::try_stream! {
            // Process each block
            while let Some(block_result) = blocks.next().await {
                let block = block_result.map_err(|e| ChainError::BlockFetchFailed)?;
                let result = client.process_block(block, &assets).await.map_err(|e| ChainError::BlockFetchFailed)?;

                if !result.is_empty() {
                    yield result
                }
            }

            // Stream ended naturally (connection closed)
            tracing::info!("Block subscription stream ended");
        };

        Ok(stream)
    }

    async fn build_transfer(
        &self,
        sender: &AssetHubAccountId,
        recipient: &AssetHubAccountId,
        asset_id: &u32,
        amount: Decimal,
    ) -> ChainResult<UnsignedTransaction<AssetHubChainConfig>> {
        // TODO: unwrap doesn't seem good here... need some checks at least to prevent errors
        let transaction_amount = (amount / Decimal::new(1, 6)).to_u128().unwrap();

        let location = MultiLocation {
            parents: 0,
            interior: Junctions::X2(
                Junction::PalletInstance(50),
                Junction::GeneralIndex(u128::from(*asset_id)),
            ),
        };

        let tx_config = DefaultExtrinsicParamsBuilder::<AssetHubConfig>::new()
            .tip_of(0, location)
            // TODO: move mortality to consts? or to config?
            .mortal(32)
            .build();

        let call = runtime::tx()
            .assets()
            .transfer(*asset_id, recipient.clone().into(), transaction_amount);

        let transaction = self.client
            .tx()
            .create_partial(&call, sender, tx_config)
            .await
            .map_err(|e| ChainError::BlockFetchFailed)?;

        Ok(UnsignedTransaction { transaction })
    }

    async fn build_transfer_all(
        &self,
        sender: &AssetHubAccountId,
        recipient: &AssetHubAccountId,
        asset_id: &u32,
    ) -> ChainResult<UnsignedTransaction<AssetHubChainConfig>> {
        let location = MultiLocation {
            parents: 0,
            interior: Junctions::X2(
                Junction::PalletInstance(50),
                Junction::GeneralIndex(u128::from(*asset_id)),
            ),
        };

        let tx_config = DefaultExtrinsicParamsBuilder::<AssetHubConfig>::new()
            .tip_of(0, location)
            // TODO: move mortality to consts? or to config?
            .mortal(32)
            .build();

        let call = runtime::tx()
            .assets()
            .transfer_all(*asset_id, recipient.clone().into(), false);

        let transaction = self.client
            .tx()
            .create_partial(&call, sender, tx_config)
            .await
            .map_err(|e| ChainError::BlockFetchFailed)?;

        Ok(UnsignedTransaction { transaction })
    }

    async fn sign_transaction(
        &self,
        transaction: UnsignedTransaction<AssetHubChainConfig>,
        derivation_params: Vec<String>,
        keyring_client: &KeyringClient,
    ) -> ChainResult<SignedTransaction<AssetHubChainConfig>> {
        let data = SignTransactionRequestData {
            transaction: transaction.transaction,
            derivation_params,
        };

        let transaction = keyring_client.sign_asset_hub_transaction(data).await?;
        Ok(SignedTransaction { transaction })
    }

    async fn submit_and_watch_transaction(
        &self,
        transaction: SignedTransaction<AssetHubChainConfig>,
    ) -> TransactionResult<AssetHubChainConfig> {
        let SignedTransaction { transaction } = transaction;

        let tx_hash = transaction.hash();

        // Submit the transaction and wait for it's finalization
        let tx_progress = transaction
            .submit_and_watch()
            .await
            .inspect_err(|e| {

            })
            .map_err(|_| TransactionError::SendRequestError(tx_hash))?;

        // Wait for tx finalization. We don't really know neither it's status or block info at this point
        let finalized_tx = tx_progress
            .wait_for_finalized()
            .await
            .inspect_err(|e| {

            })
            .map_err(|_| TransactionError::SendRequestError(tx_hash))?;

        // At this point we know that transaction was finalized and included in block
        let block_hash = finalized_tx.block_hash();

        // We still need to fetch some additional block info like it's number and timestamp
        let block = self.client
            .blocks()
            .at(block_hash)
            .await
            .inspect_err(|e| {

            })
            .map_err(|_| TransactionError::FetchBlockError(block_hash))?;

        let block_number = block.number();

        // Fetch extrinsic related events. Transaction considered successful
        // if there is no `ExtrinsicFailed` events related to this extrinsic.
        // It's still possible to face errors here but we'll
        // let events = finalized_tx
        //     .wait_for_success()
        //     .await
        //     .inspect_err(|e| {

        //     })
        //     .map_err(|e| {
        //         use subxt::error::{Error, DispatchError};

        //         match e {
        //             Error::Runtime(DispatchError::Module(error)) => {
        //                 match error.details_string() {
        //                     "<Assets::BalanceLow>" => TransactionError::NotEnoughBalance((block_number, ))
        //                 }
        //             },
        //             _ =>
        //         }
        //     })?;

        let events = finalized_tx
            .fetch_events()
            .await
            .inspect_err(|e| {

            })
            .map_err(|_| TransactionError::FetchTransactionInfoError((block_number, tx_hash)))?;

        // We finally have extrinsic index and it's events so we can find extrinsic status
        let extrinsic_index = events.extrinsic_index();

        let error_extrinsic = events
            .find_first::<runtime::system::events::ExtrinsicFailed>()
            .map_err(|_| TransactionError::TransactionInfoDecodeError((block_number, extrinsic_index)))?;

        if let Some(error) = error_extrinsic {
            // TODO: handle underneath errors somehow...
            return Err(TransactionError::UnknownError((block_number, extrinsic_index)));
        };

        let event = events
            .find_first::<TransferredEvent>()
            .inspect_err(|e| {
                // TODO: add logging
            })
            // We expect only decode error here, no need to handle any other errors
            .map_err(|_| TransactionError::TransactionInfoDecodeError((block_number, extrinsic_index)))?
            .ok_or_else(|| TransactionError::NoTransactionInfo((block_number, extrinsic_index)))?;

        let asset_info = self.asset_info_store()
            .get_asset_info(&event.asset_id)
            .await
            .ok_or_else(|| TransactionError::UnknownAsset(((block_number, extrinsic_index), event.asset_id)))?;

        let amount = Decimal::new(event.amount as i64, asset_info.decimals as u32);

        Ok(ChainTransfer {
            amount,
            asset_id: event.asset_id,
            sender: event.from,
            recipient: event.to,
            transaction_bytes: String::new(),
            transaction_id: (block_number, extrinsic_index),
            timestamp: chrono::Utc::now().timestamp_millis() as u64
        })
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use futures::pin_mut;

    use super::*;

    #[tokio::test]
    async fn test_polkadot_client() {
        let client = PolkadotAssetHubClient {
            client: AssetHubOnlineClient::from_url("wss://asset-hub-polkadot-rpc.n.dwellir.com").await.unwrap(),
            asset_info_store: AssetInfoStore::new(),
        };

        let assets = vec![1337, 1984];

        let _ = client.init_asset_info(&assets).await.unwrap();

        let amount = client.fetch_asset_balance(&1337, &AssetHubAccountId::from_str("15dikXxF1QwijxxU7wZBFmHy7HeCotHXa1LxzVu44KVKXCRC").unwrap()).await;

        println!("Result: {:?}", amount);

        // let transfer_stream = client.subscribe_transfers(assets).await;
        // pin_mut!(transfer_stream);

        // println!("Got stream");

        // while let Some(result) = transfer_stream.next().await {
        //     println!("Recevied processed block result");

        //     match result {
        //         Ok(transfers) => {
        //             for transfer in transfers {
        //                 println!("Transfer: {:?}", transfer);
        //             }
        //         }
        //         Err(e) => {
        //             println!("Error in transfer stream: {:?}", e);
        //         }
        //     }
        // }
    }
}

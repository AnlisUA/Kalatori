use std::str::FromStr;
use std::collections::HashMap;

use rust_decimal::prelude::{Decimal, ToPrimitive};
use futures::{stream, StreamExt};
use subxt::config::DefaultExtrinsicParamsBuilder;
use subxt::blocks::Block;
use subxt::blocks::{ExtrinsicDetails, FoundExtrinsic};
use subxt::utils::AccountId32;
use subxt_signer::sr25519::Keypair;

use crate::chain::{AssetHubConfig, AssetHubOnlineClient};
use crate::chain_client::AssetInfoStore;
use crate::chain::runtime::runtime_types::staging_xcm::v3::multilocation::MultiLocation;
use crate::chain::runtime::runtime_types::xcm::v3::{junction::Junction, junctions::Junctions};

use super::{
    ChainConfig,
    ChainError,
    BlockChainClient,
    ChainResult,
    AssetInfo,
    ChainTransfer
};

// Runtime type aliases for Asset Hub transfer operations
type TransferExtrinsic = crate::chain::runtime::assets::calls::types::Transfer;
type TransferAllExtrinsic = crate::chain::runtime::assets::calls::types::TransferAll;
type TransferredEvent = crate::chain::runtime::assets::events::Transferred;

type UnsignedTransaction = subxt::tx::PartialTransaction<AssetHubConfig, AssetHubOnlineClient>;
type SignedTransaction = subxt::tx::SubmittableTransaction<AssetHubConfig, AssetHubOnlineClient>;

#[derive(Debug, Clone)]
pub enum AssetHubChainConfig {}

impl ChainConfig for AssetHubChainConfig {
    type AssetId = u32;
    type TransactionId = (u32, u32); // (block number, position in block)
    type UnsignedTransaction = UnsignedTransaction;
    type SignedTransaction = SignedTransaction;
    type Signer = Keypair;
}

#[derive(Clone)]
pub struct PolkadotAssetHubClient {
    client: AssetHubOnlineClient,
    asset_info_store: AssetInfoStore<AssetHubChainConfig>,
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

impl PolkadotAssetHubClient {
    async fn from_config(
        config: &crate::configs::ChainConfig,
        asset_info_store: AssetInfoStore<AssetHubChainConfig>
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
            &crate::chain::runtime::storage().timestamp().now()
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

                        // Build transaction bytes and hash
                        // let transaction_bytes = format!("0x{}", const_hex::encode(event.details.bytes()));
                        // let tx_hash = extract_tx_hash(&transaction_bytes);
                        let transaction_bytes = String::new(); // Placeholder

                        // Convert AccountId32 to base58 string (Asset Hub prefix = 42)
                        let sender = crate::chain::utils::to_base58_string(event.from.0, 42);
                        let recipient = crate::chain::utils::to_base58_string(event.to.0, 42);

                        Some(ChainTransfer {
                            asset_id: event.asset_id,
                            amount: Decimal::new(event.amount as i64, asset_info.decimals as u32),
                            sender,
                            recipient,
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
    fn chain_name(&self) -> &'static str {
        "statemint"
    }

    fn asset_info_store(&self) -> &AssetInfoStore<AssetHubChainConfig> {
        &self.asset_info_store
    }

    async fn new(config: &crate::configs::ChainConfig) -> ChainResult<Self> {
        PolkadotAssetHubClient::from_config(config, AssetInfoStore::new()).await
    }

    async fn new_with_store(config: &crate::configs::ChainConfig, asset_info_store: AssetInfoStore<AssetHubChainConfig>) -> ChainResult<Self> {
        PolkadotAssetHubClient::from_config(config, asset_info_store).await
    }

    async fn fetch_asset_info(&self, asset_id: &u32) -> ChainResult<AssetInfo<AssetHubChainConfig>> {
        let request_data = crate::chain::runtime::storage().assets().metadata(*asset_id);

        // TODO: change errors
        self.client
            .storage()
            .at_latest()
            .await
            .map_err(|e| ChainError::BlockFetchFailed)?
            .fetch(&request_data)
            .await
            .map_err(|e| ChainError::BlockFetchFailed)?
            .ok_or_else(|| ChainError::BlockFetchFailed)
            .map(|resp| {
                AssetInfo {
                    id: *asset_id,
                    name: String::from_utf8(resp.symbol.0).unwrap(),
                    decimals: resp.decimals,
                }
            })
    }

    // TODO: replace account id with generic
    // TODO: replace errors
    // TODO: probably will be better to return some `Balance` structure with asset id and account id
    async fn fetch_asset_balance(&self, asset_id: &u32, account_id: &str) -> ChainResult<Decimal> {
        let decimals = self.asset_info_store
            .get_asset_info(asset_id)
            .await
            .ok_or_else(|| ChainError::BlockFetchFailed)?
            .decimals;

        let request_data = crate::chain::runtime::storage()
            .assets()
            // TODO: change stored type to subxt's account id
            .account(*asset_id, subxt::utils::AccountId32::from_str(account_id).unwrap());


        let amount = self.client
            .storage()
            .at_latest()
            .await
            .map_err(|e| ChainError::BlockFetchFailed)?
            .fetch(&request_data)
            .await
            .map_err(|e| ChainError::BlockFetchFailed)?
            .map_or(Decimal::ZERO, |acc| Decimal::new(acc.balance as i64, decimals as u32));

        Ok(amount)
    }

    async fn subscribe_transfers(&self, asset_ids: &[u32]) -> ChainResult<impl stream::Stream<Item = ChainResult<Vec<ChainTransfer<AssetHubChainConfig>>>>> {
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

    async fn build_transaction(
        &self,
        // TODO: replace with generic types
        sender: &str,
        recipient: &str,
        asset_id: &u32,
        amount: Decimal,
    ) -> ChainResult<UnsignedTransaction> {
        let sender_acc = AccountId32::from_str(sender).unwrap();
        let recipient_acc = AccountId32::from_str(recipient).unwrap();
        // TODO: unwrap doesn't seem good here... need some checks at least to prevent errors
        let transaction_amount = (amount / Decimal::new(1, 6)).to_u128().unwrap();

        let call = crate::chain::runtime::tx()
            .assets()
            .transfer(*asset_id, recipient_acc.into(), transaction_amount);

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

        self.client
            .tx()
            .create_partial(&call, &sender_acc, tx_config)
            .await
            .map_err(|e| ChainError::BlockFetchFailed)
    }

    async fn sign_transaction(
        &self,
        mut transaction: UnsignedTransaction,
        signer: &Keypair,
    ) -> ChainResult<SignedTransaction> {
        Ok(transaction.sign(signer))
    }
}

#[cfg(test)]
mod tests {
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

        let amount = client.fetch_asset_balance(&1337, "15dikXxF1QwijxxU7wZBFmHy7HeCotHXa1LxzVu44KVKXCRC").await;

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

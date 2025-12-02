mod asset_hub;
mod errors;
mod keyring;

use std::collections::HashMap;
use std::hash::Hash;
use std::str::FromStr;
use std::sync::Arc;

use futures::stream;
use rust_decimal::Decimal;
use tokio::sync::RwLock;
use tracing::{
    info,
    instrument,
};

pub use asset_hub::{
    AssetHubChainConfig,
    AssetHubClient,
};
pub use errors::{
    ClientError,
    QueryError,
    SubscriptionError,
    TransactionError,
};
pub use keyring::{
    Keyring,
    KeyringClient,
    KeyringError,
};

pub trait Encodeable {
    fn to_hex_string(&self) -> String;
}

pub trait ChainConfig: Clone + std::fmt::Debug {
    type AssetId: Hash + FromStr + Eq + Clone + std::fmt::Debug;
    type TransactionId: Hash + Eq + Clone + std::fmt::Debug;
    type TransactionHash: FromStr + ToString;
    type BlockHash: FromStr + ToString;
    type UnsignedTransaction;
    type SignedTransaction: Encodeable;
    type AccountId;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetInfo<T: ChainConfig> {
    pub name: String,
    pub id: T::AssetId,
    pub decimals: u8,
}

#[derive(Debug, Clone)]
pub struct ChainTransfer<T: ChainConfig> {
    pub asset_id: T::AssetId,
    pub amount: Decimal,
    pub sender: T::AccountId,    // base58 ss58 format
    pub recipient: T::AccountId, // base58 ss58 format
    pub transaction_id: T::TransactionId,
    pub timestamp: u64, // milliseconds since epoch
}

#[derive(Clone)]
pub struct AssetInfoStore<T: ChainConfig> {
    assets: Arc<RwLock<HashMap<T::AssetId, AssetInfo<T>>>>,
}

impl<T: ChainConfig> AssetInfoStore<T> {
    pub fn new() -> Self {
        Self {
            assets: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn get_asset_info(
        &self,
        asset_id: &T::AssetId,
    ) -> Option<AssetInfo<T>> {
        let assets = self.assets.read().await;
        assets.get(asset_id).cloned()
    }

    pub async fn get_assets_info(
        &self,
        assets_ids: &[T::AssetId],
    ) -> HashMap<T::AssetId, AssetInfo<T>> {
        let assets = self.assets.read().await;

        assets_ids
            .into_iter()
            .filter_map(|id| {
                assets
                    .get(id)
                    .cloned()
                    .map(|val| (id.clone(), val))
            })
            .collect()
    }
}

pub struct UnsignedTransaction<T: ChainConfig> {
    transaction: T::UnsignedTransaction,
}

pub struct SignedTransaction<T: ChainConfig> {
    transaction: T::SignedTransaction,
}

impl<T: ChainConfig> Encodeable for SignedTransaction<T> {
    fn to_hex_string(&self) -> String {
        self.transaction.to_hex_string()
    }
}

pub trait BlockChainClient<T: ChainConfig>: Sync + Send + Sized {
    fn chain_name(&self) -> &'static str;

    fn asset_info_store(&self) -> &AssetInfoStore<T>;

    async fn new(config: &crate::configs::ChainConfig) -> Result<Self, ClientError>;

    #[expect(dead_code)]
    async fn new_with_store(
        config: &crate::configs::ChainConfig,
        asset_info_store: AssetInfoStore<T>,
    ) -> Result<Self, ClientError>;

    async fn fetch_asset_info(
        &self,
        asset_id: &T::AssetId,
    ) -> Result<AssetInfo<T>, QueryError>;

    async fn fetch_asset_balance(
        &self,
        asset_id: &T::AssetId,
        account: &T::AccountId,
    ) -> Result<Decimal, QueryError>;

    async fn subscribe_transfers(
        &self,
        asset_ids: &[T::AssetId],
    ) -> Result<
        impl stream::Stream<Item = Result<Vec<ChainTransfer<T>>, SubscriptionError>>,
        SubscriptionError,
    >;

    #[expect(dead_code)]
    /// Build transaction to transfer exact amount to recipient
    async fn build_transfer(
        &self,
        sender: &T::AccountId,
        recipient: &T::AccountId,
        asset_id: &T::AssetId,
        amount: Decimal,
    ) -> Result<UnsignedTransaction<T>, TransactionError<T>>;

    /// Build transaction to sweep entire balance (all funds minus fees) to
    /// recipient
    async fn build_transfer_all(
        &self,
        sender: &T::AccountId,
        recipient: &T::AccountId,
        asset_id: &T::AssetId,
    ) -> Result<UnsignedTransaction<T>, TransactionError<T>>;

    async fn sign_transaction(
        &self,
        transaction: UnsignedTransaction<T>,
        derivation_params: Vec<String>,
        keyring_client: &KeyringClient,
    ) -> Result<SignedTransaction<T>, TransactionError<T>>;

    async fn submit_and_watch_transaction(
        &self,
        transaction: SignedTransaction<T>,
    ) -> Result<ChainTransfer<T>, TransactionError<T>>;

    // This method should be called at the very start of the program, right after
    // client initialization.
    #[instrument(skip(self))]
    async fn init_asset_info(
        &self,
        asset_ids: &[T::AssetId],
    ) -> Result<(), ClientError> {
        info!(message = "Initialize asset info store");
        let mut store = self
            .asset_info_store()
            .assets
            .write()
            .await;

        for id in asset_ids {
            let asset_info = self
                .fetch_asset_info(id)
                .await
                .map_err(|_e| ClientError::MetadataFetchFailed)?;
            store.insert(id.clone(), asset_info);
        }

        info!(message = "Asset info initialized successfully");

        Ok(())
    }
}

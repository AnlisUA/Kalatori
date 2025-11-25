mod asset_hub;

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;
use std::str::FromStr;

use tokio::sync::RwLock;
use rust_decimal::Decimal;
use futures::stream;

#[expect(clippy::enum_variant_names, reason = "All variants represent different failure points in the chain client")]
#[derive(Debug)]
pub enum ChainError {
    BlockSubscriptionFailed,
    BlockFetchFailed,
    ExtrinsicsFetchFailed,
}

pub type ChainResult<T> = Result<T, ChainError>;

pub trait ChainConfig: Clone + std::fmt::Debug {
    type AssetId: Hash + FromStr + Eq + Clone + std::fmt::Debug;
    type TransactionId: Hash + Eq + Clone + std::fmt::Debug;
    type UnsignedTransaction;
    type SignedTransaction;
    type Signer;
}

#[derive(Debug, Clone)]
pub struct AssetInfo<T: ChainConfig> {
    pub name: String,
    pub id: T::AssetId,
    pub decimals: u8,
}

#[derive(Debug, Clone)]
pub struct ChainTransfer<T: ChainConfig> {
    pub asset_id: T::AssetId,
    pub amount: Decimal,
    pub sender: String,        // base58 ss58 format
    pub recipient: String,     // base58 ss58 format
    pub transaction_bytes: String,  // hex-encoded
    pub transaction_id: T::TransactionId,
    pub timestamp: u64,        // milliseconds since epoch
}

#[derive(Clone)]
pub struct AssetInfoStore<T: ChainConfig> {
    assets: Arc<RwLock<HashMap<T::AssetId, AssetInfo<T>>>>,
}

impl<T: ChainConfig> AssetInfoStore<T> {
    pub fn new() -> Self {
        Self {
            assets: Arc::new(RwLock::new(HashMap::new()))
        }
    }

    pub async fn get_asset_info(&self, asset_id: &T::AssetId) -> Option<AssetInfo<T>> {
        let assets = self.assets.read().await;
        assets.get(asset_id).cloned()
    }

    pub async fn get_assets_info(&self, assets_ids: &[T::AssetId]) -> HashMap<T::AssetId, AssetInfo<T>> {
        let assets = self.assets.read().await;

        assets_ids
            .into_iter()
            .filter_map(|id| assets
                .get(id)
                .cloned()
                .map(|val| (id.clone(), val))
            )
            .collect()
    }
}

pub struct UnsignedTransaction<T: ChainConfig> {
    transaction: T::UnsignedTransaction,
}

pub struct SignedTransaction<T: ChainConfig> {
    transaction: T::SignedTransaction,
}

pub trait BlockChainClient<T: ChainConfig>: Sync + Send + Sized {
    fn chain_name(&self) -> &'static str;

    fn asset_info_store(&self) -> &AssetInfoStore<T>;

    async fn new(config: crate::configs::ChainConfig) -> ChainResult<Self>;

    async fn new_with_store(config: crate::configs::ChainConfig, asset_info_store: AssetInfoStore<T>) -> ChainResult<Self>;

    async fn fetch_asset_info(&self, asset_id: &T::AssetId) -> ChainResult<AssetInfo<T>>;

    async fn fetch_asset_balance(&self, asset_id: &T::AssetId, account: &str) -> ChainResult<Decimal>;

    async fn subscribe_transfers(&self, asset_ids: Vec<T::AssetId>) -> ChainResult<impl stream::Stream<Item = ChainResult<Vec<ChainTransfer<T>>>>>;

    async fn build_transaction(
        &self,
        // TODO: replace with generic types
        sender: &str,
        recipient: &str,
        asset_id: &T::AssetId,
        amount: Decimal,
    ) -> ChainResult<T::UnsignedTransaction>;

    async fn sign_transaction(
        &self,
        transaction: T::UnsignedTransaction,
        signer: &T::Signer,
    ) -> ChainResult<T::SignedTransaction>;

    // This method should be called at the very start of the program, right after client initialization.
    async fn init_asset_info(&self, asset_ids: &[T::AssetId]) -> ChainResult<()> {
        let mut store = self.asset_info_store().assets.write().await;

        for id in asset_ids {
            let asset_info = self.fetch_asset_info(id).await?;
            store.insert(id.clone(), asset_info);
        }

        Ok(())
    }
}

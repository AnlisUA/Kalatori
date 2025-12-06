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

pub trait SignedTransactionUtils {
    /// Encode transaction bytes to hex string
    fn to_hex_string(&self) -> String;

    /// Compute hash of the transaction
    fn hash(&self) -> String;
}

pub trait ChainConfig: Clone + std::fmt::Debug + Sync + Send {
    type AssetId: Hash + FromStr + ToString + Eq + Clone + std::fmt::Debug + Sync + Send;
    type TransactionId: Hash + Eq + Clone + std::fmt::Debug + Into<GeneralTransactionId> + Sync + Send;
    type TransactionHash: FromStr + ToString + Sync + Send;
    type BlockHash: FromStr + ToString + Sync + Send;
    type UnsignedTransaction;
    type SignedTransaction: SignedTransactionUtils + Sync + Send;
    type AccountId: FromStr + ToString + Sync + Send;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetInfo<T: ChainConfig> {
    pub name: String,
    pub id: T::AssetId,
    pub decimals: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneralTransactionId {
    pub block_number: Option<u32>,
    pub position_in_block: Option<u32>,
    pub hash: Option<String>,
}

impl GeneralTransactionId {
    pub fn empty() -> Self {
        Self {
            block_number: None,
            position_in_block: None,
            hash: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneralChainTransfer {
    pub chain: String,
    pub asset_id: String,
    pub amount: Decimal,
    pub sender: String,
    pub recipient: String,
    pub block_number: Option<u32>,
    pub position_in_block: Option<u32>,
    pub transaction_hash: Option<String>,
    pub timestamp: u64, // milliseconds since epoch
}

impl GeneralChainTransfer {
    pub fn general_transaction_id(&self) -> GeneralTransactionId {
        GeneralTransactionId {
            block_number: self.block_number,
            position_in_block: self.position_in_block,
            hash: self.transaction_hash.clone(),
        }
    }
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

impl<T: ChainConfig> From<ChainTransfer<T>> for GeneralChainTransfer {
    fn from(transfer: ChainTransfer<T>) -> Self {
        let trans_id: GeneralTransactionId = transfer.transaction_id.into();

        Self {
            chain: String::new(), // Chain name is not available here
            asset_id: transfer.asset_id.to_string(),
            amount: transfer.amount,
            sender: transfer.sender.to_string(),
            recipient: transfer.recipient.to_string(),
            block_number: trans_id.block_number,
            position_in_block: trans_id.position_in_block,
            transaction_hash: trans_id.hash,
            timestamp: transfer.timestamp,
        }
    }
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
            .iter()
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

impl<T: ChainConfig> SignedTransactionUtils for SignedTransaction<T> {
    fn to_hex_string(&self) -> String {
        self.transaction.to_hex_string()
    }

    fn hash(&self) -> String {
        self.transaction.hash()
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

    // async fn fetch_asset_balance(
    //     &self,
    //     asset_id: &T::AssetId,
    //     account: &T::AccountId,
    // ) -> Result<Decimal, QueryError>;
    fn fetch_asset_balance(
        &self,
        asset_id: &T::AssetId,
        account: &T::AccountId,
    ) -> impl Future<Output = Result<Decimal, QueryError>> + Send;

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


    fn submit_and_watch_transaction(
        &self,
        transaction: SignedTransaction<T>,
    ) -> impl Future<Output = Result<ChainTransfer<T>, TransactionError<T>>> + Send;

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

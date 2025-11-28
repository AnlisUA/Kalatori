mod asset_hub;
mod keyring;

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;
use std::str::FromStr;

use tracing::{instrument, info};
use tokio::sync::RwLock;
use rust_decimal::Decimal;
use futures::stream;

pub use asset_hub::{PolkadotAssetHubClient, AssetHubChainConfig};
pub use keyring::{Keyring, KeyringClient, KeyringError};

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum ChainError {
    BlockSubscriptionFailed,
    BlockFetchFailed,
    ExtrinsicsFetchFailed,
    SignFailed(KeyringError),
}

impl From<KeyringError> for ChainError {
    fn from(value: KeyringError) -> Self {
        ChainError::SignFailed(value)
    }
}

impl std::fmt::Display for ChainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl std::error::Error for ChainError {}

pub trait Encodeable {
    fn to_hex_string(&self) -> String;
}

pub type ChainResult<T> = Result<T, ChainError>;

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
    pub sender: T::AccountId,        // base58 ss58 format
    pub recipient: T::AccountId,     // base58 ss58 format
    pub transaction_bytes: String,   // hex-encoded
    pub transaction_id: T::TransactionId,
    pub timestamp: u64,              // milliseconds since epoch
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

impl<T: ChainConfig> Encodeable for SignedTransaction<T> {
    fn to_hex_string(&self) -> String {
        self.transaction.to_hex_string()
    }
}

pub type BlockNumber = u32;

// Trying to include the most actual info we have at the moment of error
pub enum TransactionError<T: ChainConfig> {
    // We sent transaction error but something went wrong. We don't really know
    // what happens with transaction. It still can either be finalized or failed.
    // At this stage we have only it's hash (we can get it for any built transaction)
    // but not any other info. Probably if we'll face that error, we'll have to
    // periodically check if transaction was included in any block along it's mortality time
    SendRequestError(T::TransactionHash),
    // An error occured while we was trying to get transaction info
    FetchTransactionInfoError((BlockNumber, T::TransactionHash)),
    // Transaction was finalized and we know it's block hash but faced an error while request
    // additional block info by this hash
    FetchBlockError(T::BlockHash),
    // Transaction was failed because of not enough money on the balance
    NotEnoughBalance(T::TransactionId),
    NoTransactionInfo(T::TransactionId),
    TransactionInfoDecodeError(T::TransactionId),
    UnknownAsset((T::TransactionId, T::AssetId)),
    UnknownError(T::TransactionId),
}

impl<T: ChainConfig> TransactionError<T> {
    pub fn is_finalized(&self) -> bool {
        use TransactionError::*;

        match self {
            // avoid using `_ =>` to avoid potential errors in case of adding new `TransactionErrorKind` variants
            NotEnoughBalance(_)
            | FetchTransactionInfoError(_)
            | NoTransactionInfo(_)
            | TransactionInfoDecodeError(_)
            | FetchBlockError(_)
            | UnknownError(_)
            | UnknownAsset(_) => true,
            SendRequestError(_) => false
        }
    }
}

pub type TransactionResult<T: ChainConfig> = Result<ChainTransfer<T>, TransactionError<T>>;

pub trait BlockChainClient<T: ChainConfig>: Sync + Send + Sized {
    fn chain_name(&self) -> &'static str;

    fn asset_info_store(&self) -> &AssetInfoStore<T>;

    async fn new(config: &crate::configs::ChainConfig) -> ChainResult<Self>;

    async fn new_with_store(config: &crate::configs::ChainConfig, asset_info_store: AssetInfoStore<T>) -> ChainResult<Self>;

    async fn fetch_asset_info(&self, asset_id: &T::AssetId) -> ChainResult<AssetInfo<T>>;

    async fn fetch_asset_balance(&self, asset_id: &T::AssetId, account: &T::AccountId) -> ChainResult<Decimal>;

    async fn subscribe_transfers(&self, asset_ids: &[T::AssetId]) -> ChainResult<impl stream::Stream<Item = ChainResult<Vec<ChainTransfer<T>>>>>;

    /// Build transaction to transfer exact amount to recipient
    async fn build_transfer(
        &self,
        sender: &T::AccountId,
        recipient: &T::AccountId,
        asset_id: &T::AssetId,
        amount: Decimal,
    ) -> ChainResult<UnsignedTransaction<T>>;

    /// Build transaction to sweep entire balance (all funds minus fees) to recipient
    async fn build_transfer_all(
        &self,
        sender: &T::AccountId,
        recipient: &T::AccountId,
        asset_id: &T::AssetId,
    ) -> ChainResult<UnsignedTransaction<T>>;

    async fn sign_transaction(
        &self,
        transaction: UnsignedTransaction<T>,
        derivation_params: Vec<String>,
        keyring_client: &KeyringClient,
    ) -> ChainResult<SignedTransaction<T>>;

    async fn submit_and_watch_transaction(
        &self,
        transaction: SignedTransaction<T>,
    ) -> TransactionResult<T>;

    // This method should be called at the very start of the program, right after client initialization.
    #[instrument(skip(self))]
    async fn init_asset_info(&self, asset_ids: &[T::AssetId]) -> ChainResult<()> {
        info!(message = "Initialize asset info store");
        let mut store = self.asset_info_store().assets.write().await;

        for id in asset_ids {
            let asset_info = self.fetch_asset_info(id).await?;
            store.insert(id.clone(), asset_info);
        }

        info!(message = "Asset info initialized successfully");

        Ok(())
    }
}

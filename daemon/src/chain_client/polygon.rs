//! Polygon (PoS) chain client implementation.
//!
//! This module provides a client for interacting with the Polygon PoS network,
//! implementing the `BlockChainClient` trait for ERC-20 token transfers (primarily USDC).

use std::str::FromStr;
use std::sync::Arc;

use alloy::eips::BlockNumberOrTag;
use alloy::network::Ethereum;
use alloy::network::TransactionBuilder;
use alloy::primitives::{Address, TxHash, U256};
use alloy::providers::{Provider, ProviderBuilder, WsConnect};
use alloy::rpc::types::{Filter, Log};
use alloy::sol;
use alloy::sol_types::SolEvent;
use futures::StreamExt;
use rust_decimal::prelude::{Decimal, ToPrimitive};
use tracing::{debug, instrument, warn};

use crate::types::ChainType;
use crate::utils::logging::category::CHAIN_CLIENT;

use super::{
    AssetInfo, AssetInfoStore, BlockChainClient, BlockChainClientExt, ChainConfig, ChainTransfer,
    ClientError, GeneralTransactionId, KeyringClient, QueryError, SignedTransaction,
    SignedTransactionUtils, SubscriptionError, TransactionError, TransfersStream,
    UnsignedTransaction,
};

use super::keyring::SignTransactionRequestData;

// ============================================================================
// ERC-20 Interface Definition
// ============================================================================

sol! {
    /// Standard ERC-20 interface for token interactions
    #[sol(rpc)]
    interface IERC20 {
        function name() external view returns (string memory);
        function symbol() external view returns (string memory);
        function decimals() external view returns (uint8);
        function balanceOf(address account) external view returns (uint256);
        function transfer(address to, uint256 amount) external returns (bool);

        event Transfer(address indexed from, address indexed to, uint256 value);
    }
}

// ============================================================================
// Type Definitions
// ============================================================================

/// Polygon account ID (Ethereum address)
pub type PolygonAccountId = Address;

/// Polygon asset ID (ERC-20 contract address)
pub type PolygonAssetId = Address;

/// Polygon transaction hash
pub type PolygonTransactionHash = TxHash;

/// Polygon block hash
pub type PolygonBlockHash = alloy::primitives::B256;

/// Unsigned transaction for Polygon (transaction request)
pub type PolygonUnsignedTransaction = alloy::rpc::types::TransactionRequest;

/// Signed transaction for Polygon
#[derive(Debug, Clone)]
pub struct PolygonSignedTransaction {
    /// The raw signed transaction bytes
    pub raw_transaction: alloy::primitives::Bytes,
    /// Transaction hash
    pub tx_hash: TxHash,
}

impl SignedTransactionUtils for PolygonSignedTransaction {
    fn to_hex_string(&self) -> String {
        format!(
            "0x{}",
            const_hex::encode(&self.raw_transaction)
        )
    }

    fn hash(&self) -> String {
        format!("{:?}", self.tx_hash)
    }
}

// ============================================================================
// Chain Configuration
// ============================================================================

/// Polygon chain configuration type marker
#[derive(Debug, Clone)]
pub enum PolygonChainConfig {}

impl ChainConfig for PolygonChainConfig {
    type AccountId = PolygonAccountId;
    type AssetId = PolygonAssetId;
    type TransactionId = (u64, u64); // (block_number, transaction_index)
    type TransactionHash = PolygonTransactionHash;
    type BlockHash = PolygonBlockHash;
    type UnsignedTransaction = PolygonUnsignedTransaction;
    type SignedTransaction = PolygonSignedTransaction;

    const CHAIN_TYPE: ChainType = ChainType::Polygon;
}

impl From<(u64, u64)> for GeneralTransactionId {
    fn from(value: (u64, u64)) -> Self {
        #[expect(clippy::cast_possible_truncation)]
        GeneralTransactionId {
            block_number: Some(value.0 as u32),
            position_in_block: Some(value.1 as u32),
            tx_hash: None,
        }
    }
}

// ============================================================================
// Utility Functions
// ============================================================================

/// Convert a U256 value to Decimal with the given number of decimals
fn u256_to_decimal(
    value: U256,
    decimals: u8,
) -> Decimal {
    // Convert U256 to string and parse as Decimal
    let value_str = value.to_string();
    let raw_decimal = Decimal::from_str(&value_str).unwrap_or(Decimal::ZERO);

    // Apply decimal places
    let scale = Decimal::new(1, u32::from(decimals));
    raw_decimal * scale
}

/// Convert a Decimal to U256 with the given number of decimals
fn decimal_to_u256(
    value: Decimal,
    decimals: u8,
) -> U256 {
    // Scale up by decimals
    let multiplier = Decimal::new(10_i64.pow(u32::from(decimals)), 0);
    #[expect(clippy::arithmetic_side_effects)]
    let scaled = value * multiplier;

    // Convert to U256
    scaled
        .to_u128()
        .map(U256::from)
        .unwrap_or(U256::ZERO)
}

/// Derive BIP44 path from derivation parameters
///
/// Uses SHA256 hash of parameters to generate a deterministic index
/// Path format: m/44'/60'/0'/0/{index}
pub fn derive_eth_path_from_params(params: &[String]) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    for param in params {
        hasher.update(param.as_bytes());
    }
    let hash = hasher.finalize();

    // Use first 4 bytes as index (allows ~4 billion unique addresses)
    let index = u32::from_be_bytes([hash[0], hash[1], hash[2], hash[3]]);

    format!("m/44'/60'/0'/0/{index}")
}

// ============================================================================
// Polygon Client
// ============================================================================

/// Inner client holding the actual provider
/// We use RwLock to allow recreation of the provider if needed
struct PolygonClientInner {
    /// RPC endpoint URL for reconnection
    #[expect(dead_code)]
    endpoint: String,
    /// Chain ID for transaction signing
    chain_id: u64,
}

/// Client for interacting with Polygon PoS network
#[derive(Clone)]
pub struct PolygonClient {
    /// Inner configuration
    inner: Arc<PolygonClientInner>,
    /// Store for asset information (ERC-20 metadata)
    asset_info_store: AssetInfoStore<PolygonChainConfig>,
    /// Cached provider - we rebuild when needed
    endpoint: String,
}

impl PolygonClient {
    /// Create a new Polygon client from configuration
    #[instrument(skip(config, asset_info_store))]
    async fn from_config(
        config: &crate::configs::ChainConfig,
        asset_info_store: AssetInfoStore<PolygonChainConfig>,
    ) -> Result<Self, ClientError> {
        let endpoint = config
            .endpoints
            .first()
            .ok_or(ClientError::InvalidConfiguration {
                field: "endpoints".to_string(),
            })?
            .clone();

        // Test connection and get chain ID
        let ws_connect = WsConnect::new(&endpoint);
        let provider = ProviderBuilder::new()
            .connect_ws(ws_connect)
            .await
            .inspect_err(|e| {
                tracing::debug!(
                    error.category = CHAIN_CLIENT,
                    error.operation = "connect_client",
                    error.source = ?e,
                    endpoint = %endpoint,
                    "Failed to connect to Polygon RPC endpoint"
                );
            })
            .map_err(|_| ClientError::AllEndpointsUnreachable)?;

        // Get chain ID for transaction signing
        let chain_id = provider
            .get_chain_id()
            .await
            .inspect_err(|e| {
                tracing::debug!(
                    error.category = CHAIN_CLIENT,
                    error.source = ?e,
                    "Failed to get chain ID"
                );
            })
            .map_err(|_| ClientError::MetadataFetchFailed)?;

        tracing::info!(
            chain_id = chain_id,
            endpoint = %endpoint,
            "Connected to Polygon network"
        );

        Ok(Self {
            inner: Arc::new(PolygonClientInner {
                endpoint: endpoint.clone(),
                chain_id,
            }),
            asset_info_store,
            endpoint,
        })
    }

    /// Create a fresh provider connection
    async fn create_provider(&self) -> Result<impl Provider<Ethereum> + Clone, ClientError> {
        let ws_connect = WsConnect::new(&self.endpoint);
        ProviderBuilder::new()
            .connect_ws(ws_connect)
            .await
            .map_err(|_| ClientError::AllEndpointsUnreachable)
    }

    /// Convert a log entry to a ChainTransfer
    async fn log_to_transfer(
        &self,
        log: &Log,
        event: &IERC20::Transfer,
    ) -> Result<ChainTransfer<PolygonChainConfig>, SubscriptionError> {
        let asset_id = log.address();

        let asset_info = self
            .asset_info_store
            .get_asset_info(&asset_id)
            .await
            .ok_or_else(|| {
                tracing::warn!(
                    asset_id = %asset_id,
                    "Received transfer event for unknown asset"
                );
                SubscriptionError::AssetNotFound {
                    asset_id: 0, // We don't have u32 for Polygon, using 0 as placeholder
                }
            })?;

        let block_number = log
            .block_number
            .ok_or(SubscriptionError::BlockProcessingFailed { block_number: 0 })?;

        let tx_index = log.transaction_index.ok_or(
            SubscriptionError::BlockProcessingFailed {
                #[expect(clippy::cast_possible_truncation)]
                block_number: block_number as u32,
            },
        )?;

        // Use current time for timestamp (we could fetch block, but it's expensive)
        #[expect(clippy::cast_sign_loss)]
        let timestamp = chrono::Utc::now().timestamp_millis() as u64;

        let amount = u256_to_decimal(event.value, asset_info.decimals);

        Ok(ChainTransfer {
            asset_id,
            asset_name: asset_info.name.clone(),
            amount,
            sender: event.from,
            recipient: event.to,
            transaction_id: (block_number, tx_index),
            timestamp,
        })
    }
}

impl BlockChainClient<PolygonChainConfig> for PolygonClient {
    fn chain_name(&self) -> &'static str {
        "polygon"
    }

    fn asset_info_store(&self) -> &AssetInfoStore<PolygonChainConfig> {
        &self.asset_info_store
    }

    #[instrument(skip(config))]
    async fn new(config: &crate::configs::ChainConfig) -> Result<Self, ClientError> {
        Self::from_config(config, AssetInfoStore::new()).await
    }

    #[instrument(skip(config, asset_info_store))]
    async fn new_with_store(
        config: &crate::configs::ChainConfig,
        asset_info_store: AssetInfoStore<PolygonChainConfig>,
    ) -> Result<Self, ClientError> {
        Self::from_config(config, asset_info_store).await
    }

    #[instrument(skip(self))]
    async fn recreate(&self) -> Result<Self, ClientError> {
        // For now, just return a clone
        // TODO: Implement proper reconnection logic
        Ok(self.clone())
    }

    #[instrument(skip(self))]
    async fn fetch_asset_info(
        &self,
        asset_id: &PolygonAssetId,
    ) -> Result<AssetInfo<PolygonChainConfig>, QueryError> {
        debug!(message = "Fetching ERC-20 token info...", asset_id = %asset_id);

        let provider = self
            .create_provider()
            .await
            .map_err(|_| QueryError::RpcRequestFailed)?;

        let contract = IERC20::new(*asset_id, provider);

        // Fetch symbol
        let symbol_result = contract
            .symbol()
            .call()
            .await
            .inspect_err(|e| {
                tracing::debug!(
                    error.category = CHAIN_CLIENT,
                    error.operation = "fetch_asset_info",
                    error.source = ?e,
                    asset_id = %asset_id,
                    "Failed to fetch token symbol"
                );
            })
            .map_err(|_| QueryError::RpcRequestFailed)?;

        // alloy 1.4 returns the value directly for single return values
        let symbol = symbol_result;

        // Fetch decimals
        let decimals_result = contract
            .decimals()
            .call()
            .await
            .inspect_err(|e| {
                tracing::debug!(
                    error.category = CHAIN_CLIENT,
                    error.operation = "fetch_asset_info",
                    error.source = ?e,
                    asset_id = %asset_id,
                    "Failed to fetch token decimals"
                );
            })
            .map_err(|_| QueryError::RpcRequestFailed)?;

        // alloy 1.4 returns the value directly
        let decimals = decimals_result;

        let info = AssetInfo {
            id: *asset_id,
            name: symbol,
            decimals,
        };

        debug!(message = "Asset info fetched successfully", asset_info = ?info);

        Ok(info)
    }

    #[instrument(skip(self))]
    async fn fetch_asset_balance(
        &self,
        asset_id: &PolygonAssetId,
        account: &PolygonAccountId,
    ) -> Result<Decimal, QueryError> {
        debug!(message = "Fetching ERC-20 balance...", asset_id = %asset_id, account = %account);

        let decimals = self
            .asset_info_store
            .get_asset_info(asset_id)
            .await
            .ok_or_else(|| {
                warn!("Asset info not found in local store");
                QueryError::NotFound {
                    query_type: format!("asset info for {asset_id}"),
                }
            })?
            .decimals;

        let provider = self
            .create_provider()
            .await
            .map_err(|_| QueryError::RpcRequestFailed)?;

        let contract = IERC20::new(*asset_id, provider);

        let balance_result = contract
            .balanceOf(*account)
            .call()
            .await
            .inspect_err(|e| {
                tracing::debug!(
                    error.category = CHAIN_CLIENT,
                    error.operation = "fetch_balance",
                    error.source = ?e,
                    asset_id = %asset_id,
                    account = %account,
                    "Failed to fetch token balance"
                );
            })
            .map_err(|_| QueryError::RpcRequestFailed)?;

        // alloy 1.4 returns the value directly
        let balance = balance_result;

        Ok(u256_to_decimal(balance, decimals))
    }

    #[instrument(skip(self))]
    async fn subscribe_transfers(
        &self,
        asset_ids: &[PolygonAssetId],
    ) -> Result<TransfersStream<PolygonChainConfig>, SubscriptionError> {
        // Verify all assets are in the store
        let assets = self
            .asset_info_store
            .get_assets_info(asset_ids)
            .await;

        for asset_id in asset_ids {
            if !assets.contains_key(asset_id) {
                return Err(SubscriptionError::AssetNotFound {
                    asset_id: 0, // Placeholder since Polygon uses Address not u32
                });
            }
        }

        // Build filter for Transfer events from all tracked ERC-20 contracts
        let filter = Filter::new()
            .address(asset_ids.to_vec())
            .event_signature(IERC20::Transfer::SIGNATURE_HASH)
            .from_block(BlockNumberOrTag::Latest);

        let client = self.clone();

        // Create provider for subscription
        let provider = self
            .create_provider()
            .await
            .map_err(|_| SubscriptionError::SubscriptionFailed)?;

        // Subscribe to logs
        let subscription = provider
            .subscribe_logs(&filter)
            .await
            .inspect_err(|e| {
                tracing::debug!(
                    error.category = CHAIN_CLIENT,
                    error.operation = "subscribe_transfers",
                    error.source = ?e,
                    "Failed to subscribe to Transfer events"
                );
            })
            .map_err(|_| SubscriptionError::SubscriptionFailed)?;

        tracing::info!(
            asset_count = asset_ids.len(),
            "Subscribed to ERC-20 Transfer events"
        );

        let stream = async_stream::try_stream! {
            let mut sub = subscription.into_stream();

            while let Some(log) = sub.next().await {
                // Decode Transfer event from log
                match log.log_decode::<IERC20::Transfer>() {
                    Ok(decoded) => {
                        let event = decoded.inner.data;
                        match client.log_to_transfer(&log, &event).await {
                            Ok(transfer) => {
                                tracing::debug!(
                                    from = %transfer.sender,
                                    to = %transfer.recipient,
                                    amount = %transfer.amount,
                                    asset = %transfer.asset_name,
                                    "Detected ERC-20 transfer"
                                );
                                yield vec![transfer];
                            }
                            Err(e) => {
                                tracing::warn!(
                                    error = ?e,
                                    "Failed to process transfer event"
                                );
                            }
                        }
                    }
                    Err(e) => {
                        tracing::debug!(
                            error = ?e,
                            "Failed to decode Transfer event from log"
                        );
                    }
                }
            }

            tracing::info!("Transfer event subscription stream ended");
        };

        Ok(Box::pin(stream))
    }

    #[instrument(skip(self))]
    async fn init_asset_info(
        &self,
        asset_ids: &[String],
    ) -> Result<(), ClientError> {
        BlockChainClientExt::init_asset_info_impl(self, asset_ids).await
    }

    #[instrument(skip(self), fields(asset_id = %asset_id, amount = %amount))]
    async fn build_transfer(
        &self,
        sender: &PolygonAccountId,
        recipient: &PolygonAccountId,
        asset_id: &PolygonAssetId,
        amount: Decimal,
    ) -> Result<UnsignedTransaction<PolygonChainConfig>, TransactionError<PolygonChainConfig>> {
        let decimals = self
            .asset_info_store
            .get_asset_info(asset_id)
            .await
            .ok_or_else(|| TransactionError::BuildFailed {
                reason: format!("Asset {asset_id} not found in asset info store"),
            })?
            .decimals;

        let amount_wei = decimal_to_u256(amount, decimals);

        let provider = self
            .create_provider()
            .await
            .map_err(|e| {
                tracing::debug!(error.source = ?e, "Failed to create provider for build_transfer");
                TransactionError::BuildFailed {
                    reason: "Failed to connect to RPC".to_string(),
                }
            })?;

        let contract = IERC20::new(*asset_id, provider);

        // Build transfer call
        let call = contract.transfer(*recipient, amount_wei);

        // Create transaction request
        let mut tx_request = call.into_transaction_request();
        tx_request.set_from(*sender);
        tx_request.set_chain_id(self.inner.chain_id);

        debug!(
            message = "Built ERC-20 transfer transaction",
            from = %sender,
            to = %recipient,
            amount_wei = %amount_wei
        );

        Ok(UnsignedTransaction {
            transaction: tx_request,
        })
    }

    #[instrument(skip(self), fields(asset_id = %asset_id))]
    async fn build_transfer_all(
        &self,
        sender: &PolygonAccountId,
        recipient: &PolygonAccountId,
        asset_id: &PolygonAssetId,
    ) -> Result<UnsignedTransaction<PolygonChainConfig>, TransactionError<PolygonChainConfig>> {
        // Fetch current balance
        let balance = self
            .fetch_asset_balance(asset_id, sender)
            .await
            .map_err(|e| {
                tracing::debug!(
                    error.source = ?e,
                    "Failed to fetch balance for transfer_all"
                );
                TransactionError::BuildFailed {
                    reason: "Failed to fetch balance".to_string(),
                }
            })?;

        if balance.is_zero() {
            return Err(TransactionError::BuildFailed {
                reason: "Zero balance, nothing to transfer".to_string(),
            });
        }

        debug!(
            message = "Building transfer_all transaction",
            balance = %balance
        );

        self.build_transfer(sender, recipient, asset_id, balance)
            .await
    }

    #[instrument(skip(self, transaction, keyring_client))]
    async fn sign_transaction(
        &self,
        transaction: UnsignedTransaction<PolygonChainConfig>,
        derivation_params: Vec<String>,
        keyring_client: &KeyringClient,
    ) -> Result<SignedTransaction<PolygonChainConfig>, TransactionError<PolygonChainConfig>> {
        let data = SignTransactionRequestData {
            transaction: transaction.transaction,
            derivation_params,
        };

        let signed = keyring_client
            .sign_polygon_transaction(data)
            .await
            .map_err(|e| {
                tracing::debug!(
                    error.source = ?e,
                    "Failed to sign Polygon transaction"
                );
                TransactionError::BuildFailed {
                    reason: format!("Signing failed: {e}"),
                }
            })?;

        Ok(SignedTransaction {
            transaction: signed,
        })
    }

    #[instrument(skip(self, transaction), fields(transaction_hash))]
    async fn submit_and_watch_transaction(
        &self,
        transaction: SignedTransaction<PolygonChainConfig>,
    ) -> Result<ChainTransfer<PolygonChainConfig>, TransactionError<PolygonChainConfig>> {
        let tx_hash = transaction.transaction.tx_hash;
        tracing::Span::current().record(
            "transaction_hash",
            format!("{tx_hash:?}"),
        );

        let provider = self
            .create_provider()
            .await
            .map_err(|e| {
                tracing::debug!(error.source = ?e, "Failed to create provider");
                TransactionError::SubmissionStatusUnknown
            })?;

        // Submit raw transaction
        let pending_tx = provider
            .send_raw_transaction(&transaction.transaction.raw_transaction)
            .await
            .inspect_err(|e| {
                tracing::debug!(
                    error.category = CHAIN_CLIENT,
                    error.operation = "submit_transaction",
                    error.source = ?e,
                    transaction_hash = %tx_hash,
                    "Transaction submission failed"
                );
            })
            .map_err(|_| TransactionError::SubmissionStatusUnknown)?;

        tracing::info!(
            transaction_hash = %tx_hash,
            "Transaction submitted, waiting for confirmation"
        );

        // Wait for receipt
        let receipt = pending_tx
            .get_receipt()
            .await
            .inspect_err(|e| {
                tracing::debug!(
                    error.category = CHAIN_CLIENT,
                    error.operation = "watch_transaction",
                    error.source = ?e,
                    transaction_hash = %tx_hash,
                    "Failed to get transaction receipt"
                );
            })
            .map_err(|_| TransactionError::SubmissionStatusUnknown)?;

        let block_number = receipt.block_number.unwrap_or(0);
        let tx_index = receipt.transaction_index.unwrap_or(0);
        let transaction_id = (block_number, tx_index);

        // Check if transaction was successful
        if !receipt.status() {
            tracing::warn!(
                transaction_hash = %tx_hash,
                block_number = block_number,
                "Transaction reverted on-chain"
            );

            return Err(TransactionError::ExecutionFailed {
                transaction_id,
                error_code: "Transaction reverted".to_string(),
            });
        }

        // Find Transfer event in logs
        let transfer_log = receipt
            .inner
            .logs()
            .iter()
            .find(|log| {
                log.topics()
                    .first()
                    .map(|t| *t == IERC20::Transfer::SIGNATURE_HASH)
                    .unwrap_or(false)
            })
            .ok_or_else(|| {
                tracing::debug!(
                    transaction_hash = %tx_hash,
                    "No Transfer event found in transaction logs"
                );
                TransactionError::TransactionInfoFetchFailed { transaction_id }
            })?;

        // Decode the Transfer event
        let decoded = transfer_log
            .log_decode::<IERC20::Transfer>()
            .map_err(|e| {
                tracing::debug!(
                    error.source = ?e,
                    "Failed to decode Transfer event"
                );
                TransactionError::TransactionInfoFetchFailed { transaction_id }
            })?;

        let event = decoded.inner.data;
        let asset_id = transfer_log.address();

        let asset_info = self
            .asset_info_store
            .get_asset_info(&asset_id)
            .await
            .ok_or(TransactionError::UnknownAsset {
                transaction_id,
                asset_id,
            })?;

        let amount = u256_to_decimal(event.value, asset_info.decimals);

        // Get current timestamp
        #[expect(clippy::cast_sign_loss)]
        let timestamp = chrono::Utc::now().timestamp_millis() as u64;

        tracing::info!(
            transaction_hash = %tx_hash,
            block_number = block_number,
            from = %event.from,
            to = %event.to,
            amount = %amount,
            "Transaction confirmed successfully"
        );

        Ok(ChainTransfer {
            asset_id,
            asset_name: asset_info.name.clone(),
            amount,
            sender: event.from,
            recipient: event.to,
            transaction_id,
            timestamp,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_u256_decimal_conversion() {
        // 1 USDC = 1_000_000 (6 decimals)
        let value = U256::from(1_000_000_u64);
        let decimal = u256_to_decimal(value, 6);
        assert_eq!(decimal, Decimal::new(1, 0)); // 1.0

        // Convert back
        let back = decimal_to_u256(decimal, 6);
        assert_eq!(back, value);
    }

    #[test]
    fn test_derive_eth_path() {
        let params = vec![
            "0x1234567890123456789012345678901234567890".to_string(),
            "order-123".to_string(),
        ];
        let path = derive_eth_path_from_params(&params);
        assert!(path.starts_with("m/44'/60'/0'/0/"));

        // Same params should produce same path
        let path2 = derive_eth_path_from_params(&params);
        assert_eq!(path, path2);

        // Different params should produce different path
        let different_params = vec!["other".to_string()];
        let different_path = derive_eth_path_from_params(&different_params);
        assert_ne!(path, different_path);
    }

    #[test]
    fn test_polygon_chain_config() {
        assert_eq!(
            PolygonChainConfig::CHAIN_TYPE,
            ChainType::Polygon
        );
    }
}

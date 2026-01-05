//! Legacy V2 API Types
//!
//! This module contains all structures, enums, and type aliases used by the V2
//! HTTP API endpoints. These types are preserved for backward compatibility
//! with existing API clients.
//!
//! According to the architectural vision in CLAUDE.md, new code should use
//! types from the `types` module which work with the `SQLite` database schema.
//! These legacy types will gradually be phased out as the API evolves, but the
//! V2 endpoints must remain unchanged for backward compatibility.

use std::collections::HashMap;

use codec::{
    Decode,
    Encode,
};
use serde::{
    Deserialize,
    Serialize,
    Serializer,
};

use crate::configs::ChainConfig;
use crate::error::OrderError;
use crate::types::{
    Invoice,
    Transaction,
    TransactionStatus,
};

pub const AMOUNT: &str = "amount";
pub const CURRENCY: &str = "currency";
pub type AssetId = u32;
pub type Decimals = u8;
pub type BlockNumber = u32;
pub type ExtrinsicIndex = u32;
pub type CurrenciesMap = HashMap<String, CurrencyProperties>;

#[derive(Clone)]
pub struct LegacyApiData {
    pub server_info: ServerInfo,
    pub currencies: CurrenciesMap,
    pub recipient: String,
    pub rpc_endpoints: Vec<String>,
}

#[derive(Encode, Decode, Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Timestamp(pub u64);

#[derive(Debug, Serialize)]
pub struct InvalidParameter {
    pub parameter: String,
    pub message: String,
}

#[expect(dead_code)]
#[derive(Debug, Clone)]
pub struct OrderQuery {
    pub order: String,
    pub amount: f64,
    pub callback: String,
    pub redirect_url: String,
    pub currency: String,
}

#[derive(Debug, Serialize)]
pub enum OrderResponse {
    NewOrder(OrderStatus),
    FoundOrder(OrderStatus),
    ModifiedOrder(OrderStatus),
    CollidedOrder(OrderStatus),
    NotFound,
}

#[derive(Debug, Serialize)]
pub struct OrderStatus {
    pub order: String,
    pub message: String,
    pub recipient: String,
    pub server_info: ServerInfo,
    #[serde(flatten)]
    pub order_info: OrderInfo,
    pub payment_page: String,
    pub redirect_url: String,
}

#[derive(Clone, Debug, Serialize, Encode, Decode)]
pub struct OrderInfo {
    pub withdrawal_status: WithdrawalStatus,
    pub payment_status: PaymentStatus,
    pub amount: f64,
    pub currency: CurrencyInfo,
    pub callback: String,
    pub transactions: Vec<TransactionInfo>,
    pub payment_account: String,
    pub death: Timestamp,
}

impl OrderInfo {
    #[expect(dead_code, reason = "Legacy type kept for backward compatibility")]
    pub fn new(
        query: OrderQuery,
        currency: CurrencyInfo,
        payment_account: String,
        death: Timestamp,
    ) -> Self {
        OrderInfo {
            withdrawal_status: WithdrawalStatus::Waiting,
            payment_status: PaymentStatus::Pending,
            amount: query.amount,
            currency,
            callback: query.callback,
            transactions: Vec::new(),
            payment_account,
            death,
        }
    }
}

#[expect(dead_code, reason = "Legacy type kept for backward compatibility")]
pub enum OrderCreateResponse {
    New(OrderInfo),
    Modified(OrderInfo),
    Collision(OrderInfo),
}

#[derive(Clone, Debug, Serialize, Decode, Encode, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PaymentStatus {
    Pending,
    Paid,
}

#[derive(Clone, Debug, Serialize, Deserialize, Decode, Encode, PartialEq, Eq, Copy, sqlx::Type)]
#[serde(rename_all = "lowercase")]
pub enum WithdrawalStatus {
    Waiting,
    Failed,
    Forced,
    Completed,
}

impl std::fmt::Display for WithdrawalStatus {
    fn fmt(
        &self,
        f: &mut std::fmt::Formatter<'_>,
    ) -> std::fmt::Result {
        match self {
            Self::Waiting => write!(f, "Waiting"),
            Self::Failed => write!(f, "Failed"),
            Self::Forced => write!(f, "Forced"),
            Self::Completed => write!(f, "Completed"),
        }
    }
}

impl std::str::FromStr for WithdrawalStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Waiting" => Ok(Self::Waiting),
            "Failed" => Ok(Self::Failed),
            "Forced" => Ok(Self::Forced),
            "Completed" => Ok(Self::Completed),
            _ => Err(format!(
                "Unknown withdrawal status: {s}"
            )),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServerStatus {
    pub server_info: ServerInfo,
    pub supported_currencies: HashMap<std::string::String, CurrencyProperties>,
}

#[derive(Debug, Serialize)]
pub struct ServerHealth {
    pub server_info: ServerInfo,
    pub connected_rpcs: Vec<RpcInfo>,
    pub status: Health,
}

#[derive(Debug, Serialize, Clone)]
pub struct RpcInfo {
    pub rpc_url: String,
    pub chain_name: String,
    pub status: Health,
}

#[derive(Debug, Serialize, Clone, PartialEq, Copy)]
#[serde(rename_all = "lowercase")]
pub enum Health {
    Ok,
    Degraded,
    Critical,
}

#[derive(Clone, Debug, Serialize, Decode, Encode)]
pub struct CurrencyInfo {
    pub currency: String,
    pub chain_name: String,
    pub kind: TokenKind,
    pub decimals: Decimals,
    pub rpc_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_id: Option<AssetId>,
    // #[serde(skip_serializing)]
    pub ss58: u16,
}

impl CurrencyInfo {
    #[expect(dead_code)]
    pub fn properties(&self) -> CurrencyProperties {
        CurrencyProperties {
            chain_name: self.chain_name.clone(),
            kind: self.kind,
            decimals: self.decimals,
            rpc_url: self.rpc_url.clone(),
            asset_id: self.asset_id,
            ss58: self.ss58,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CurrencyProperties {
    pub chain_name: String,
    pub kind: TokenKind,
    pub decimals: Decimals,
    pub rpc_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_id: Option<AssetId>,
    // #[serde(skip_serializing)]
    pub ss58: u16,
}

impl CurrencyProperties {
    pub fn info(
        &self,
        currency: String,
    ) -> CurrencyInfo {
        CurrencyInfo {
            currency,
            chain_name: self.chain_name.clone(),
            kind: self.kind,
            decimals: self.decimals,
            rpc_url: self.rpc_url.clone(),
            asset_id: self.asset_id,
            ss58: self.ss58,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Decode, Encode, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TokenKind {
    Asset,
    Native,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct ServerInfo {
    pub version: String,
    pub instance_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kalatori_remark: Option<String>,
}

#[derive(Clone, Debug, Serialize, Decode, Encode)]
pub struct TransactionInfo {
    #[serde(skip_serializing_if = "Option::is_none", flatten)]
    pub finalized_tx: Option<FinalizedTx>, // Clearly undefined in v2.1 - TODO
    pub transaction_bytes: String,
    pub sender: String,
    pub recipient: String,
    #[serde(serialize_with = "amount_serializer")]
    pub amount: Amount,
    pub currency: CurrencyInfo,
    pub status: TxStatus,
}

#[derive(Clone, Debug, Serialize, Decode, Encode)]
pub struct FinalizedTx {
    pub block_number: BlockNumber,
    pub position_in_block: ExtrinsicIndex,
    pub timestamp: String,
}

// TODO: `Encode` macro generates some code which cast usize to u8 and trigger
// clippy. It seems to be old issue happened again, https://github.com/paritytech/parity-scale-codec/issues/713
// Check for updates periodically and remove this module when problem is fixed
#[expect(clippy::cast_possible_truncation)]
mod amount {
    use super::{
        Decode,
        Encode,
    };

    #[derive(Clone, Debug, Decode, Encode)]
    pub enum Amount {
        All,
        Exact(f64),
    }
}

pub use amount::Amount;

fn amount_serializer<S: Serializer>(
    amount: &Amount,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match amount {
        Amount::All => serializer.serialize_str("all"),
        Amount::Exact(exact) => exact.serialize(serializer),
    }
}

#[derive(Clone, Debug, Serialize, Decode, Encode)]
#[serde(rename_all = "lowercase")]
pub enum TxStatus {
    Pending,
    Finalized,
    Failed,
}

impl From<TransactionStatus> for TxStatus {
    fn from(status: TransactionStatus) -> Self {
        match status {
            TransactionStatus::Completed => TxStatus::Finalized,
            TransactionStatus::Failed => TxStatus::Failed,
            TransactionStatus::Waiting | TransactionStatus::InProgress => TxStatus::Pending,
        }
    }
}

// ============================================================================
// Legacy Database Types (from database.rs)
// These types are used for sled database operations and migration
// ============================================================================

/// Transaction info as stored in sled database
#[derive(Encode, Decode)]
pub struct TransactionInfoDb {
    pub transaction_bytes: String,
    pub inner: TransactionInfoDbInner,
}

/// Inner transaction data for sled encoding
#[derive(Encode, Decode)]
pub struct TransactionInfoDbInner {
    pub finalized_tx: Option<FinalizedTxDb>,
    pub finalized_tx_timestamp: Option<String>,
    pub sender: String,
    pub recipient: String,
    pub amount: Amount,
    pub currency: CurrencyInfo,
    pub status: TxStatus,
    pub kind: TxKind,
}

/// Transaction kind (payment vs withdrawal)
#[derive(Encode, Decode, Debug, Clone, Copy)]
pub enum TxKind {
    Payment,
    Withdrawal,
}

/// Finalized transaction data in sled format
#[derive(Encode, Decode, Debug)]
pub struct FinalizedTxDb {
    pub block_number: BlockNumber,
    pub position_in_block: ExtrinsicIndex,
}

/// Convert from sled format to API format
impl From<TransactionInfoDb> for TransactionInfo {
    fn from(value: TransactionInfoDb) -> Self {
        let finalized_tx = value.inner.finalized_tx.and_then(|tx| {
            value
                .inner
                .finalized_tx_timestamp
                .map(|timestamp| FinalizedTx {
                    block_number: tx.block_number,
                    position_in_block: tx.position_in_block,
                    timestamp,
                })
        });

        Self {
            finalized_tx,
            transaction_bytes: value.transaction_bytes,
            sender: value.inner.sender,
            recipient: value.inner.recipient,
            amount: value.inner.amount,
            currency: value.inner.currency,
            status: value.inner.status,
        }
    }
}

fn decimal_to_amount(amount: rust_decimal::Decimal) -> Amount {
    // Convert Decimal to f64 for legacy API
    // Note: This may lose precision for very large or very precise numbers
    let amount_f64 = amount
        .to_string()
        .parse::<f64>()
        .unwrap_or(0.0);
    Amount::Exact(amount_f64)
}

fn asset_id_to_currency_name(asset_id: Option<u32>) -> Result<&'static str, OrderError> {
    match asset_id {
        Some(1337) => Ok("USDC"),
        Some(1984) => Ok("USDt"),
        None => Ok("DOT"),
        _ => Err(OrderError::UnknownCurrency),
    }
}

pub fn transaction_to_transaction_info(
    transaction: Transaction,
    currencies: &CurrenciesMap,
) -> Result<TransactionInfo, OrderError> {
    let asset_name = asset_id_to_currency_name(Some(transaction.asset_id))?;

    let currency = currencies
        .get(asset_name)
        .ok_or(OrderError::UnknownCurrency)?
        .info(asset_name.to_string());

    // Convert finalization data
    let finalized_tx = if let (Some(block_number), Some(position_in_block)) = (
        transaction.block_number,
        transaction.position_in_block,
    ) {
        Some(FinalizedTx {
            block_number,
            position_in_block,
            timestamp: transaction.created_at.to_rfc3339(),
        })
    } else {
        None
    };

    Ok(TransactionInfo {
        finalized_tx,
        transaction_bytes: transaction
            .transaction_bytes
            .unwrap_or_default(),
        sender: transaction.sender,
        recipient: transaction.recipient,
        amount: decimal_to_amount(transaction.amount),
        currency,
        status: transaction.status.into(),
    })
}

pub fn invoice_to_order_info(
    invoice: &Invoice,
    currencies: &CurrenciesMap,
) -> Result<OrderInfo, OrderError> {
    let asset_name = asset_id_to_currency_name(invoice.asset_id)?;

    let currency = currencies
        .get(asset_name)
        .ok_or(OrderError::UnknownCurrency)?
        .info(asset_name.to_string());

    Ok(OrderInfo {
        currency,
        amount: invoice
            .amount
            .to_string()
            .parse::<f64>()
            .unwrap_or(0.0),
        payment_account: invoice.payment_address.clone(),
        payment_status: PaymentStatus::from(invoice.status),
        withdrawal_status: invoice.withdrawal_status,
        death: Timestamp(
            #[expect(clippy::cast_sign_loss)]
            {
                invoice.valid_till.timestamp_millis() as u64
            },
        ),
        callback: invoice.callback.clone(),
        transactions: vec![], // Transactions would be loaded separately if needed
    })
}

/// Build a simplified currencies `HashMap` from `ChainConfig` for migration
/// purposes. Note: This uses placeholder values for decimals since we don't
/// have blockchain connection yet. The actual decimals are fetched
/// asynchronously by `ChainTracker` during normal operation.
pub fn build_currencies_from_config(
    chain_config: &ChainConfig
) -> std::collections::HashMap<String, CurrencyProperties> {
    let mut currencies = std::collections::HashMap::new();
    let rpc_url = chain_config
        .endpoints
        .first()
        .cloned()
        .unwrap_or_default();

    for asset in &chain_config.assets {
        let properties = CurrencyProperties {
            chain_name: chain_config.name.clone(),
            kind: TokenKind::Asset,
            decimals: 6, // Placeholder - not used during migration validation
            rpc_url: rpc_url.clone(),
            asset_id: Some(asset.id),
            ss58: 2, // Placeholder - not used during migration
        };

        currencies.insert(asset.name.clone(), properties);
    }

    currencies
}

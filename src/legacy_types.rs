//! Legacy V2 API Types
//!
//! This module contains all structures, enums, and type aliases used by the V2 HTTP API endpoints.
//! These types are preserved for backward compatibility with existing API clients.
//!
//! According to the architectural vision in CLAUDE.md, new code should use types from the `types`
//! module which work with the `SQLite` database schema. These legacy types will gradually be phased
//! out as the API evolves, but the V2 endpoints must remain unchanged for backward compatibility.

use std::collections::HashMap;

use codec::{Decode, Encode};
use serde::{Deserialize, Serialize, Serializer};

pub const AMOUNT: &str = "amount";
pub const CURRENCY: &str = "currency";
pub type AssetId = u32;
pub type Decimals = u8;
pub type BlockNumber = u32;
pub type ExtrinsicIndex = u32;

#[derive(Encode, Decode, Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Timestamp(pub u64);

#[derive(Debug, Serialize)]
pub struct InvalidParameter {
    pub parameter: String,
    pub message: String,
}

#[derive(Debug)]
pub struct OrderQuery {
    pub order: String,
    pub amount: f64,
    pub callback: String,
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
    pub fn info(&self, currency: String) -> CurrencyInfo {
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

// TODO: `Encode` macro generates some code which cast usize to u8 and trigger clippy.
// It seems to be old issue happened again, https://github.com/paritytech/parity-scale-codec/issues/713
// Check for updates periodically and remove this module when problem is fixed
#[expect(clippy::cast_possible_truncation)]
mod amount {
    use super::{Decode, Encode};

    #[derive(Clone, Debug, Decode, Encode)]
    pub enum Amount {
        All,
        Exact(f64),
    }
}

pub use amount::Amount;

fn amount_serializer<S: Serializer>(amount: &Amount, serializer: S) -> Result<S::Ok, S::Error> {
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

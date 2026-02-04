use alloy::primitives::{Address, B256, address};
use serde::{Serialize, Deserialize};
use rust_decimal::Decimal;
use chrono::{DateTime, Utc};
use uuid::Uuid;
use sqlx::{FromRow, Type};
use sqlx::types::{Json, Text};

use crate::swaps::{OrderSubmitRequest, UnsignedOrderData};

use super::ChainType;

// 1inch Aggregation Router (Limit Order Protocol), same for all supported chains
const LIMIT_ORDER_PROTOCOL: Address = address!("0x111111125421ca6dc452d289314280a0f8842a65");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
pub enum OneInchSupportedChain {
    Ethereum,
    Polygon,
    Bsc,
    Arbitrum,
    Avalanche,
    Optimism,
    Fantom,
    Gnosis,
    Base,
    Linea,
    Sonic,
    Unichain,
    // there is no zkSync Era (324) here, it has differentlimit order protocol v4 address
}

impl OneInchSupportedChain {
    pub fn chain_id(&self) -> u64 {
        use OneInchSupportedChain::*;

        match self {
            Ethereum => 1,
            Polygon => 137,
            Bsc => 56,
            Arbitrum => 42161,
            Avalanche => 43114,
            Optimism => 10,
            Fantom => 250,
            Gnosis => 100,
            Base => 8453,
            Linea => 59144,
            Sonic => 146,
            Unichain => 130,
        }
    }

    pub fn from_chain_id(chain_id: u64) -> Option<Self> {
        use OneInchSupportedChain::*;

        match chain_id {
            1 => Some(Ethereum),
            137 => Some(Polygon),
            56 => Some(Bsc),
            42161 => Some(Arbitrum),
            43114 => Some(Avalanche),
            10 => Some(Optimism),
            250 => Some(Fantom),
            100 => Some(Gnosis),
            8453 => Some(Base),
            59144 => Some(Linea),
            146 => Some(Sonic),
            130 => Some(Unichain),
            _ => None,
        }
    }

    pub fn verifying_protocol(&self) -> Address {
       LIMIT_ORDER_PROTOCOL
    }
}

impl TryFrom<ChainType> for OneInchSupportedChain {
    type Error = String;

    fn try_from(value: ChainType) -> Result<Self, Self::Error> {
        match value {
            ChainType::Polygon => Ok(Self::Polygon),
            ChainType::PolkadotAssetHub => Err("Polkadot Asset Hub is not supported chain for 1Inch swaps".to_string())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum OneInchSwapStatus {
    /// An order has been created but not submitted
    Created,
    /// An order has been submitted to 1Inch API
    Submitted,
    /// An order has been created and waiting for execution
    Pending,
    /// An order has been executed successfully
    Completed,
    /// An order has failed/canceled/refunded
    Failed,
}

impl std::fmt::Display for OneInchSwapStatus {
    fn fmt(
        &self,
        f: &mut std::fmt::Formatter<'_>,
    ) -> std::fmt::Result {
        match self {
            Self::Created => write!(f, "Created"),
            Self::Submitted => write!(f, "Submitted"),
            Self::Pending => write!(f, "Pending"),
            Self::Completed => write!(f, "Completed"),
            Self::Failed => write!(f, "Failed"),
        }
    }
}

impl std::str::FromStr for OneInchSwapStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Created" => Ok(Self::Created),
            "Submitted" => Ok(Self::Submitted),
            "Pending" => Ok(Self::Pending),
            "Completed" => Ok(Self::Completed),
            "Failed" => Ok(Self::Failed),
            _ => Err(format!("Unknown swap status: {s}"))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateOneInchSwapParams {
    pub invoice_id: Uuid,
    pub from_chain: u64,
    pub from_token_address: Address,
    pub from_amount_units: u128,
    pub from_address: Address,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateOneInchSwapData {
    pub invoice_id: Uuid,
    pub from_chain: OneInchSupportedChain,
    pub to_chain: OneInchSupportedChain,
    pub from_token_address: Address,
    pub to_token_address: Address,
    pub from_amount_units: u128,
    pub from_address: Address,
    pub to_address: Address,
}

impl CreateOneInchSwapData {
    pub fn is_cross_chain(&self) -> bool {
        self.from_chain != self.to_chain
    }
}

#[derive(FromRow)]
pub struct CreateOneInchSwapDataRow {
    pub invoice_id: Uuid,
    pub from_chain: OneInchSupportedChain,
    pub to_chain: OneInchSupportedChain,
    pub from_token_address: Text<Address>,
    pub to_token_address: Text<Address>,
    // Note: have to store u128 as Text because sqlite doesn't support it
    // as a native type. See details here: https://docs.rs/sqlx/latest/sqlx/sqlite/types/index.html#types
    pub from_amount_units: Text<u128>,
    pub from_address: Text<Address>,
    pub to_address: Text<Address>,
}

impl From<CreateOneInchSwapDataRow> for CreateOneInchSwapData {
    fn from(value: CreateOneInchSwapDataRow) -> Self {
        Self {
            invoice_id: value.invoice_id,
            from_chain: value.from_chain,
            to_chain: value.to_chain,
            from_token_address: value.from_token_address.0,
            to_token_address: value.to_token_address.0,
            from_amount_units: value.from_amount_units.0,
            from_address: value.from_address.0,
            to_address: value.to_address.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct OneInchPreparedSwap {
    pub id: Uuid,
    pub request: CreateOneInchSwapData,
    pub unsigned_order: UnsignedOrderData,
    pub to_amount: Decimal,
    pub created_at: DateTime<Utc>,
    pub valid_till: DateTime<Utc>,
}

impl OneInchPreparedSwap {
    pub fn to_signed(self, signature: String) -> OneInchSwap {
        let raw_order = OrderSubmitRequest {
            src_chain_id: self.request.from_chain.chain_id(),
            order: self.unsigned_order.order,
            signature,
            quote_id: self.unsigned_order.quote_id,
            extension: const_hex::encode_prefixed(self.unsigned_order.extension),
            secret_hashes: self.unsigned_order.secret_hashes,
        };

        OneInchSwap {
            id: self.id,
            request: self.request,
            status: OneInchSwapStatus::Created,
            to_amount: self.to_amount,
            order_hash: self.unsigned_order.order_hash,
            secrets: self.unsigned_order.secrets,
            raw_order,
            created_at: self.created_at,
            submitted_at: None,
            finished_at: None,
            valid_till: self.valid_till,
            error_message: None,
        }
    }

    pub fn into_public(self) -> PublicOneInchPreparedSwap {
        self.into()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicOneInchPreparedSwap {
    pub id: Uuid,
    pub invoice_id: Uuid,
    pub from_chain: u64,
    pub from_token_address: Address,
    pub from_amount_units: u128,
    pub from_address: Address,
    pub to_amount: Decimal,
    pub order_hash: B256,
    pub valid_till: DateTime<Utc>,
}

impl From<OneInchPreparedSwap> for PublicOneInchPreparedSwap {
    fn from(value: OneInchPreparedSwap) -> Self {
        Self {
            id: value.id,
            invoice_id: value.request.invoice_id,
            from_chain: value.request.from_chain.chain_id(),
            from_token_address: value.request.from_token_address,
            from_amount_units: value.request.from_amount_units,
            from_address: value.request.from_address,
            to_amount: value.to_amount,
            order_hash: value.unsigned_order.order_hash,
            valid_till: value.valid_till,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OneInchSwap {
    pub id: Uuid,
    pub request: CreateOneInchSwapData,
    pub status: OneInchSwapStatus,
    pub to_amount: Decimal, // approximate
    pub order_hash: B256,
    pub secrets: Vec<B256>,
    pub raw_order: OrderSubmitRequest,
    pub created_at: DateTime<Utc>,
    pub submitted_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub valid_till: DateTime<Utc>,
    pub error_message: Option<String>,
}

impl OneInchSwap {
    pub fn is_cross_chain(&self) -> bool {
        self.request.is_cross_chain()
    }

    pub fn into_public(self) -> PublicOneInchSwap {
        self.into()
    }
}

#[derive(FromRow)]
pub struct OneInchSwapRow {
    pub id: Uuid,
    #[sqlx(flatten)]
    pub request: CreateOneInchSwapDataRow,
    pub status: OneInchSwapStatus,
    pub to_amount: Text<Decimal>,
    pub order_hash: Text<B256>,
    pub secrets: Json<Vec<B256>>,
    pub raw_order: Json<OrderSubmitRequest>,
    pub created_at: DateTime<Utc>,
    pub submitted_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub valid_till: DateTime<Utc>,
    pub error_message: Option<String>,
}

impl From<OneInchSwapRow> for OneInchSwap {
    fn from(value: OneInchSwapRow) -> Self {
        Self {
            id: value.id,
            request: value.request.into(),
            status: value.status,
            to_amount: *value.to_amount,
            order_hash: *value.order_hash,
            secrets: value.secrets.0,
            raw_order: value.raw_order.0,
            created_at: value.created_at,
            submitted_at: value.submitted_at,
            finished_at: value.finished_at,
            valid_till: value.valid_till,
            error_message: value.error_message,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicOneInchSwap {
    pub id: Uuid,
    pub invoice_id: Uuid,
    pub from_chain: u64,
    pub from_token_address: Address,
    pub from_address: Address,
    pub from_amount_units: u128,
    pub status: OneInchSwapStatus,
    pub to_amount: Decimal,
    pub order_hash: B256,
    pub created_at: DateTime<Utc>,
    pub submitted_at: Option<DateTime<Utc>>,
    pub valid_till: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

impl From<OneInchSwap> for PublicOneInchSwap {
    fn from(value: OneInchSwap) -> Self {
        Self {
            id: value.id,
            invoice_id: value.request.invoice_id,
            from_chain: value.request.from_chain.chain_id(),
            from_token_address: value.request.from_token_address,
            from_address: value.request.from_address,
            from_amount_units: value.request.from_amount_units,
            status: value.status,
            to_amount: value.to_amount,
            order_hash: value.order_hash,
            created_at: value.created_at,
            submitted_at: value.submitted_at,
            valid_till: value.valid_till,
            error_message: value.error_message,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitOneInchSwapParams {
    pub swap_id: Uuid,
    pub invoice_id: Uuid,
    pub order_hash: B256,
    pub signature: String,
}

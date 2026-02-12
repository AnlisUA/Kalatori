use alloy::primitives::Address;
use serde::{
    Deserialize,
    Serialize,
};
use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateFrontEndSwapParams {
    pub invoice_id: Uuid,
    pub from_amount_units: u128,
    pub from_chain_id: u32,
    pub from_asset_id: Address,
    pub transaction_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrontEndSwap {
    pub id: Uuid,
    pub invoice_id: Uuid,
    pub from_amount_units: u128,
    pub from_chain_id: u32,
    pub from_asset_id: Address,
    pub transaction_hash: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

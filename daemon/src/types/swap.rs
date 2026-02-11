use alloy::primitives::Address;
use serde::{
    Deserialize,
    Serialize,
};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrontEndSwap {
    pub invoice_id: Uuid,
    pub from_amount_units: u128,
    pub from_chain_id: u32,
    pub from_asset_id: Address,
    pub transaction_hash: String,
}

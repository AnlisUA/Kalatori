mod hash_lock;
mod utils;

use std::collections::HashMap;
use std::time::Duration;

use alloy::sol;
use alloy::primitives::{U256, Address, keccak256, address, B256};
use alloy::sol_types::{eip712_domain, SolStruct};
use chrono::Utc;
use serde::{Serialize, Deserialize};
use serde::de::DeserializeOwned;
use rand::prelude::*;
use uuid::Uuid;
use secrecy::{SecretString, ExposeSecret};
use rust_decimal::Decimal;

use crate::types::CreateOneInchSwapData;

use utils::{generate_secrets, create_hashlock_from_secrets, build_extension, get_secret_hashes, MakerTraits};

const ONE_INCH_BASE_URL: &'static str = "https://api.1inch.dev";

const DEFAULT_PRICE_ESTIMATES_CURRENCY: &'static str = "USD";

const API_TIMEOUT_DURATION: Duration = Duration::from_secs(30);

// TRUE_ERC20 - placeholder token address used for cross-chain orders
// This represents "any ERC20" on the taker side for Fusion+ orders
const TRUE_ERC20: Address = address!("0xda0000d4000015a526378bb6fafc650cea5966f8");

// EIP-712 Domain constants
const LIMIT_ORDER_DOMAIN_NAME: &str = "1inch Aggregation Router";
const LIMIT_ORDER_DOMAIN_VERSION: &str = "6";


sol! {
    #[derive(Debug, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Order {
        uint256 salt;
        address maker;
        address receiver;
        address makerAsset;
        address takerAsset;
        uint256 makingAmount;
        uint256 takingAmount;
        uint256 makerTraits;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetQuoteParams {
    pub src_chain: u64,
    pub dst_chain: u64,
    pub src_token_address: Address,
    pub dst_token_address: Address,
    pub amount: String,
    // TODO: get rid of options, both params are always set, enable_estimate is always true
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wallet_address: Option<Address>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_estimate: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GasCost {
    pub gas_bump_estimate: u32,
    pub gas_price_estimate: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuctionPoint {
    pub delay: u16,
    pub coefficient: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotePreset {
    pub auction_duration: u32,
    pub start_auction_in: u64,
    pub initial_rate_bump: u32,
    pub auction_start_amount: String,
    pub start_amount: String,
    pub auction_end_amount: String,
    pub exclusive_resolver: Option<String>,
    pub cost_in_dst_token: String,
    pub points: Vec<AuctionPoint>,
    pub allow_partial_fills: bool,
    pub allow_multiple_fills: bool,
    pub gas_cost: GasCost,
    pub secrets_count: usize,
}

impl QuotePreset {
    pub fn encode_auction_details(&self, auction_start_time: u32) -> Vec<u8> {
        let mut data = Vec::new();

        // gasBumpEstimate (3 bytes, big endian uint24)
        let gas_bump_bytes = self.gas_cost.gas_bump_estimate.to_be_bytes();
        data.extend_from_slice(&gas_bump_bytes[1..4]); // Take last 3 bytes

        // gasPriceEstimate (4 bytes, big endian uint32)
        data.extend_from_slice(&self.gas_cost.gas_price_estimate.parse::<u32>().unwrap_or(0).to_be_bytes());

        // startTime (4 bytes, big endian uint32)
        data.extend_from_slice(&auction_start_time.to_be_bytes());

        // duration (3 bytes, big endian uint24)
        let duration_bytes = self.auction_duration.to_be_bytes();
        data.extend_from_slice(&duration_bytes[1..4]); // Take last 3 bytes

        // initialRateBump (3 bytes, big endian uint24)
        let bump_bytes = self.initial_rate_bump.to_be_bytes();
        data.extend_from_slice(&bump_bytes[1..4]); // Take last 3 bytes

        // Points: each point is coefficient(3 bytes) + delay(2 bytes) = 5 bytes
        // NO length byte - points are just concatenated directly
        for point in &self.points {
            // coefficient (3 bytes, uint24)
            let coef_bytes = point.coefficient.to_be_bytes();
            data.extend_from_slice(&coef_bytes[1..4]); // Take last 3 bytes

            // delay (2 bytes, uint16)
            data.extend_from_slice(&point.delay.to_be_bytes());
        }

        data
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeLocks {
    pub src_withdrawal: u32,
    pub src_public_withdrawal: u32,
    pub src_cancellation: u32,
    pub src_public_cancellation: u32,
    pub dst_withdrawal: u32,
    pub dst_public_withdrawal: u32,
    pub dst_cancellation: u32,
}

impl TimeLocks {
    pub fn encode(&self) -> U256 {
        // SDK order: [deployedAt, dstCancellation, dstPublicWithdrawal, dstWithdrawal,
        //             srcPublicCancellation, srcCancellation, srcPublicWithdrawal, srcWithdrawal]
        // Using reduce((acc, el) => (acc << 32n) | el) which builds from high to low bits
        let mut value = U256::ZERO;

        value |= U256::from(0) << 224; // deployedAt is always 0
        value |= U256::from(self.dst_cancellation) << 192;
        value |= U256::from(self.dst_public_withdrawal) << 160;
        value |= U256::from(self.dst_withdrawal) << 128;
        value |= U256::from(self.src_public_cancellation) << 96;
        value |= U256::from(self.src_cancellation) << 64;
        value |= U256::from(self.src_public_withdrawal) << 32;
        value |= U256::from(self.src_withdrawal);

        value
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Presets {
    pub fast: Option<QuotePreset>,
    pub medium: Option<QuotePreset>,
    pub slow: Option<QuotePreset>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecommendedPreset {
    Fast,
    Medium,
    Slow,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuoteResponse {
    pub quote_id: Uuid,
    pub src_token_amount: String,
    pub dst_token_amount: String,
    pub presets: Presets,
    pub time_locks: TimeLocks,
    pub src_escrow_factory: Address,
    pub dst_escrow_factory: Address,
    pub src_safety_deposit: String,
    pub dst_safety_deposit: String,
    pub whitelist: Vec<Address>,
    pub recommended_preset: RecommendedPreset,
}

impl QuoteResponse {
    pub fn get_recommended_preset(&self) -> &QuotePreset {
        match self.recommended_preset {
            RecommendedPreset::Fast => self.presets.fast.as_ref(),
            RecommendedPreset::Medium => self.presets.medium.as_ref(),
            RecommendedPreset::Slow => self.presets.slow.as_ref(),
        }.unwrap()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OneInchOrder {
    pub salt: String,
    pub maker: String,
    pub receiver: String,
    pub maker_asset: String,
    pub taker_asset: String,
    pub making_amount: String,
    pub taking_amount: String,
    pub maker_traits: String,
}

impl OneInchOrder {
    pub fn new(
        maker: Address,
        maker_asset: Address,
        making_amount: String,
        taking_amount: String,
        expiration: u64,
        nonce: u64,
        extension: &[u8],
    ) -> Self {
        let mut salt_bytes = [0u8; 32];
        rand::rng().fill_bytes(&mut salt_bytes);
        let mut salt_u256 = U256::from_be_bytes(salt_bytes);

        // Compute keccak256 hash of extension
        let extension_hash = keccak256(extension);

        // Get lower 160 bits (20 bytes) of the extension hash
        let lower_160_mask: U256 = (U256::from(1) << 160) - U256::from(1);
        let extension_hash_u256 = U256::from_be_bytes(extension_hash.0);
        let lower_160 = extension_hash_u256 & lower_160_mask;

        // Keep upper 96 bits of current salt (random), replace lower 160 with extension hash
        let upper_96_mask: U256 = !lower_160_mask;
        salt_u256 = (salt_u256 & upper_96_mask) | lower_160;
        let salt = salt_u256.to_string();

        let maker_traits = MakerTraits::new()
            .with_expiration(expiration)
            .with_nonce(nonce)
            .with_extension()
            .with_post_interaction()
            .build()
            .to_string();

        OneInchOrder {
            salt,
            // Use `.to_string()` because 1inch awaits checksummed address while serde serializes into other one
            maker: maker.to_string(),
            receiver: Address::ZERO.to_string(), // Will be set by extension for cross-chain
            maker_asset: maker_asset.to_string(),
            taker_asset: TRUE_ERC20.to_string(),
            making_amount,
            taking_amount,
            maker_traits,
        }
    }

    pub fn compute_hash(&self, chain_id: u64, verifying_contract: Address) -> B256 {
        let domain = eip712_domain! {
            name: LIMIT_ORDER_DOMAIN_NAME,
            version: LIMIT_ORDER_DOMAIN_VERSION,
            chain_id: chain_id,
            verifying_contract: verifying_contract,
        };

        let order_sol: Order = self.clone().into();
        order_sol.eip712_signing_hash(&domain)
    }
}

impl From<OneInchOrder> for Order {
    fn from(value: OneInchOrder) -> Self {
        Self {
            salt: U256::from_str_radix(&value.salt, 10).unwrap(),
            maker: value.maker.parse().unwrap(),
            receiver: value.receiver.parse().unwrap(),
            makerAsset: value.maker_asset.parse().unwrap(),
            takerAsset: value.taker_asset.parse().unwrap(),
            makingAmount: U256::from_str_radix(&value.making_amount, 10).unwrap(),
            takingAmount: U256::from_str_radix(&value.taking_amount, 10).unwrap(),
            makerTraits: U256::from_str_radix(&value.maker_traits, 10).unwrap(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct UnsignedOrderData {
    pub order: OneInchOrder,
    pub order_hash: B256,
    pub secrets: Vec<B256>,
    pub secret_hashes: Option<Vec<String>>,
    pub quote_id: Uuid,
    pub extension: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderSubmitRequest {
    pub src_chain_id: u64,
    pub order: OneInchOrder,
    pub signature: String,
    pub quote_id: Uuid,
    pub extension: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret_hashes: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetOrdersByHashesRequest<'a> {
    pub order_hashes: &'a [B256],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OrderStatus {
    Pending,
    Filled,
    Executed,
    Expired,
    Cancelled,
    Refunded,
    NotFound,
    #[serde(other)]
    Unknown,
}

impl std::fmt::Display for OrderStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrderStatus::Pending => write!(f, "pending"),
            OrderStatus::Filled => write!(f, "filled"),
            OrderStatus::Executed => write!(f, "executed"),
            OrderStatus::Expired => write!(f, "expired"),
            OrderStatus::Cancelled => write!(f, "cancelled"),
            OrderStatus::Refunded => write!(f, "refunded"),
            OrderStatus::NotFound => write!(f, "not-found"),
            OrderStatus::Unknown => write!(f, "unknown"),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EscrowEvent {
    #[serde(default)]
    pub transaction_hash: Option<String>,
    #[serde(default)]
    pub escrow: Option<String>,
    #[serde(default)]
    pub side: Option<String>,
    #[serde(default)]
    pub action: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FillStatusInfo {
    #[serde(default)]
    pub tx_hash: Option<String>,
    #[serde(default)]
    pub filled_maker_amount: Option<String>,
    #[serde(default)]
    pub filled_auction_taker_amount: Option<String>,
    #[serde(default)]
    pub escrow_events: Vec<EscrowEvent>,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderStatusResponse {
    pub status: OrderStatus,
    pub src_chain_id: u64,
    pub dst_chain_id: u64,
    #[serde(default)]
    pub fills: Vec<FillStatusInfo>,
    pub order_hash: B256,
    pub remaining_maker_amount: String,
    pub approximate_taking_amount: String,
}

impl OrderStatusResponse {
    pub fn has_dst_deployed_escrow(&self) -> bool {
        self.fills
            .iter()
            .any(|fill| fill.escrow_events
                .iter()
                // TODO: try to find real event names and define them as enum
                .any(|event| event.action
                    .as_ref()
                    .is_some_and(|name| name == "dst_escrow_created")
                )
            )
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadyFill {
    pub idx: u64,
    pub src_escrow_deploy_tx_hash: String,
    pub dst_escrow_deploy_tx_hash: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadyToAcceptSecretFills {
    pub fills: Vec<ReadyFill>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetReadyToAcceptSecretFillsRequest {
    order_hash: B256,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretSubmitRequest {
    order_hash: B256,
    secret: B256,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GetPricesRequest<'a> {
    tokens: &'a [Address],
    currency: &'static str,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetPricesResponse {
    #[serde(flatten)]
    pub usd_prices: HashMap<Address, Decimal>,
}

// Use separate type to keep API consistency. 1Inch has json in camelCase while our API use snake_case
impl From<GetPricesResponse> for crate::types::GetPricesResponse {
    fn from(value: GetPricesResponse) -> Self {
        Self {
            usd_prices: value.usd_prices,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OneInchApiError {
    pub status_code: u16,
    // 1inch uses either `description` and `message` depending on API endpoint
    #[serde(alias = "message")]
    pub description: String,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub meta: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OneInchApiResponse<T> {
    Ok(T),
    Err(OneInchApiError)
}

#[derive(Debug, thiserror::Error)]
pub enum OneInchError {
    #[error("Request error")]
    ReqwestError(#[from] reqwest::Error),
    #[error("JSON error")]
    JsonError(#[from] serde_json::Error),
    #[error("API Error")]
    ApiError(OneInchApiError),
}

#[derive(Debug, Clone)]
pub struct OneInchClient {
    client: reqwest::Client,
    api_key: SecretString,
}

impl OneInchClient {
    pub fn new(api_key: SecretString) -> Self {
        OneInchClient {
            client: reqwest::Client::new(),
            api_key,
        }
    }

    #[tracing::instrument(skip_all)]
    async fn send_request<T: Serialize + std::fmt::Debug, R: DeserializeOwned>(
        &self,
        path: impl AsRef<str>,
        method: reqwest::Method,
        params: T,
    ) -> Result<R, OneInchError> {
        let url = format!("{}{}", ONE_INCH_BASE_URL, path.as_ref());

        let request = self.client
            .request(method.clone(), &url)
            .timeout(API_TIMEOUT_DURATION)
            .bearer_auth(&self.api_key.expose_secret());

        let request = match method {
            reqwest::Method::GET => {
                request.query(&params)
            },
            reqwest::Method::POST => {
                request.json(&params)
            },
            _ => unreachable!(),
        };

        tracing::trace!(
            request.url = url,
            request.method = ?method,
            request.params = ?params,
            "Prepared request to 1Inch API"
        );

        let response = request
            .send()
            .await?;

        let status = response.status();

        let response_text =
            response
            .text()
            .await
            .map(|text| if text.is_empty() {
                // For some requests 1inch API returns just empty successful response. Not even `{}` or `null`.
                // So if we'll try to deserialize it, we'll have a failure while expect just `()`
                "null".to_string()
            } else {
                text
            })?;

        tracing::trace!(
            response.status = ?status,
            response.text = response_text,
            "Got response from 1Inch API"
        );

        let result = serde_json::from_str(&response_text)?;

        match result {
            OneInchApiResponse::Ok(resp) => Ok(resp),
            OneInchApiResponse::Err(e) => Err(OneInchError::ApiError(e)),
        }
    }

    #[tracing::instrument]
    pub async fn get_prices(&self, chain: u64, tokens: &[Address]) -> Result<GetPricesResponse, OneInchError> {
        self.send_request(
            format!("/price/v1.1/{}", chain),
            reqwest::Method::POST,
            GetPricesRequest {
                tokens,
                currency: DEFAULT_PRICE_ESTIMATES_CURRENCY,
            }
        ).await
    }

    pub async fn get_quote(&self, params: GetQuoteParams) -> Result<QuoteResponse, OneInchError> {
        self.send_request(
            "/fusion-plus/quoter/v1.1/quote/receive",
            reqwest::Method::GET,
            params,
        ).await
    }

    pub async fn submit_order(&self, params: OrderSubmitRequest) -> Result<(), OneInchError> {
        self.send_request(
            "/fusion-plus/relayer/v1.1/submit",
            reqwest::Method::POST,
            params,
        ).await
    }

    pub async fn get_orders_by_hashes(&self, order_hashes: &[B256]) -> Result<Vec<OrderStatusResponse>, OneInchError> {
        self.send_request(
            "/fusion-plus/orders/v1.1/order/status",
            reqwest::Method::POST,
            GetOrdersByHashesRequest {
                order_hashes,
            },
        ).await
    }

    pub async fn get_ready_to_accept_secret_fills(&self, order_hash: B256) -> Result<ReadyToAcceptSecretFills, OneInchError> {
        let path = format!("/fusion-plus/orders/v1.1/order/ready-to-accept-secret-fills/{}", order_hash);

        self.send_request(
            path,
            reqwest::Method::GET,
            GetReadyToAcceptSecretFillsRequest {
                order_hash,
            },
        ).await
    }

    pub async fn submit_secret(&self, order_hash: B256, secret: B256) -> Result<(), OneInchError> {
        self.send_request(
            "/fusion-plus/relayer/v1.1/submit/secret",
            reqwest::Method::POST,
            SecretSubmitRequest {
                order_hash,
                secret,
            },
        ).await
    }

    pub async fn build_order_from_request_data(
        &self,
        data: CreateOneInchSwapData,
        auction_time_buffer_secs: u64,
    ) -> Result<UnsignedOrderData, OneInchError> {
        let quote_params = GetQuoteParams {
            src_chain: data.from_chain.chain_id(),
            dst_chain: data.to_chain.chain_id(),
            src_token_address: data.from_token_address,
            dst_token_address: data.to_token_address,
            amount: data.from_amount_units.to_string(),
            // TODO: get rid of options, both params are always set, enable_estimate is always true
            wallet_address: Some(data.from_address),
            enable_estimate: Some(true),
        };

        let quote = self.get_quote(quote_params).await?;
        let quote_id = quote.quote_id;

        // build secrets and hashlock
        let preset = quote.get_recommended_preset();
        let secrets = generate_secrets(preset.secrets_count);
        let hashlock = create_hashlock_from_secrets(&secrets);
        let secret_hashes = get_secret_hashes(&secrets);

        // timing parameters
        let now = Utc::now().timestamp() as u64;
        let auction_start_time = now + preset.start_auction_in;
        let expiration = auction_start_time + preset.auction_duration as u64 + auction_time_buffer_secs;

        let extension = build_extension(
            hashlock.value(),
            data.to_chain.chain_id(),
            data.to_token_address,
            &quote,
            auction_start_time as u32,
        );

        let order = OneInchOrder::new(
            data.from_address,
            data.from_token_address,
            quote.src_token_amount.clone(),
            preset.auction_end_amount.clone(),
            expiration,
            now,
            &extension,
        );

        let order_hash = order.compute_hash(
            data.from_chain.chain_id(),
            data.from_chain.verifying_protocol(),
        );

        let data = UnsignedOrderData {
            order,
            order_hash,
            secrets,
            secret_hashes,
            quote_id,
            extension,
        };

        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use alloy::primitives::address;

    use super::*;

    #[tokio::test]
    async fn test_get_prices() {
        let client = OneInchClient::new(
            SecretString::from(std::env::var("ONE_INCH_API_KEY").unwrap()),
        );

        let resp = client
            .get_prices(
                137,
                &[address!("0x3c499c542cEF5E3811e1192ce70d8cC03d5c3359"), address!("0xc2132D05D31c914a87C6611C10748AEb04B58e8F")],
            )
            .await
            .unwrap();
        println!("Resp: {:#?}", resp);
    }
}

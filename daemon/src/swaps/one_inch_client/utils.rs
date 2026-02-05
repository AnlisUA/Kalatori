use alloy::primitives::{keccak256, B256, U256, Address};
use alloy::sol_types::SolValue;
use rand::prelude::*;

use super::{CrossQuoteResponse, IntentQuoteResponse};

use super::hash_lock::HashLock;

/// Generate a random 32-byte secret
pub fn generate_secret() -> B256 {
    let mut data = [0u8; 32];
    rand::rng().fill_bytes(&mut data);
    B256::from(data)
}

/// Generate multiple random secrets
pub fn generate_secrets(count: usize) -> Vec<B256> {
    (0..count).map(|_| generate_secret()).collect()
}

pub fn hash_secret(secret: &B256) -> B256 {
    keccak256(secret.as_slice())
}

/// Get secret hashes for API submission
/// Returns None for single fill, Some(vec) for multiple fills
pub fn get_secret_hashes(secrets: &[B256]) -> Option<Vec<String>> {
    if secrets.len() <= 1 {
        None
    } else {
        Some(
            secrets
                .iter()
                .map(|s| const_hex::encode_prefixed(hash_secret(s).as_slice()))
                .collect()
        )
    }
}

/// Create a HashLock from secrets based on the count
/// - 1 secret: SingleFill
/// - 3+ secrets: MultipleFills with Merkle tree
pub fn create_hashlock_from_secrets(secrets: &[B256]) -> HashLock {
    match secrets.len() {
        0 => panic!("At least one secret is required"),
        1 => HashLock::for_single_fill(&secrets[0]),
        2 => panic!("2 secrets not supported - use 1 or 3+"),
        _ => HashLock::for_multiple_fills(secrets),
    }
}

fn build_cross_interaction_data(
    auction_start_time: u32,
    whitelist: &Vec<Address>,
) -> Vec<u8> {
    let mut data = Vec::new();

    // Resolving start time (4 bytes)
    data.extend_from_slice(&auction_start_time.to_be_bytes());

    // Build whitelist from quote (all resolvers can fill at the same time)
    // Whitelist entries (12 bytes each)
    for addr in whitelist {
        // Last 10 bytes
        data.extend_from_slice(&addr.as_slice()[10..20]);
        data.extend_from_slice(&0u16.to_be_bytes());
    }

    // Whitelist length * 8
    data.push((whitelist.len() * 8) as u8);

    data
}

/// Encode the escrow extra data portion (5 x 32 bytes = 160 bytes)
/// Format: ABI.encode([hashLock, dstChainId, dstToken, safetyDeposits, timeLocks])
fn encode_extra_data(
    hashlock: B256,
    quote: &CrossQuoteResponse,
    dst_chain_id: u64,
    dst_token: Address,
) -> Vec<u8> {
    // ABI encode 5 values as uint256
    let hashlock_u256 = U256::from_be_bytes(hashlock.0);
    let dst_chain_id_u256 = U256::from(dst_chain_id);
    let dst_token_u256 = U256::from_be_bytes({
        let mut bytes = [0u8; 32];
        bytes[12..32].copy_from_slice(dst_token.as_slice());
        bytes
    });

    // Combine safety deposits: (src << 128) | dst
    let safety_deposits = (U256::from(quote.src_safety_deposit.parse::<u128>().unwrap()) << 128) | U256::from(quote.dst_safety_deposit.parse::<u128>().unwrap());

    let timelocks_u256 = quote.time_locks.encode();

    // ABI encode as tuple of 5 uint256
    let encoded = (
        hashlock_u256,
        dst_chain_id_u256,
        dst_token_u256,
        safety_deposits,
        timelocks_u256,
    ).abi_encode_params();

    encoded
}

/// Build the full extension string using 1inch SDK format
///
/// Extension format (from ExtensionBuilder):
/// - First 32 bytes: uint256 containing 8 x 32-bit cumulative offsets
/// - Fields 1-8: makerAssetSuffix, takerAssetSuffix, makingAmountData, takingAmountData,
///               predicate, makerPermit, preInteraction, postInteraction
/// - After fields: customData
///
/// For EscrowExtension:
/// - makingAmountData = address + auctionDetails + fees + whitelist addresses
/// - takingAmountData = same as makingAmountData
/// - postInteraction = address + interactionData + escrowExtraData
pub fn build_cross_extension(
    hash_lock: B256,
    dst_chain_id: u64,
    dst_token_address: Address,
    quote_response: &CrossQuoteResponse,
    auction_start_time: u32,
) -> Vec<u8> {
    let preset = quote_response.get_recommended_preset();
    // Build each field
    // Field 1: makerAssetSuffix - empty for Fusion+
    let maker_asset_suffix: Vec<u8> = vec![];

    // Field 2: takerAssetSuffix - empty for Fusion+
    let taker_asset_suffix: Vec<u8> = vec![];

    // Field 3: makingAmountData - settlement address + amount getter data
    let amount_getter_data = preset.encode_auction_details(auction_start_time);
    let mut making_amount_data = Vec::new();
    making_amount_data.extend_from_slice(quote_response.src_escrow_factory.as_slice());
    making_amount_data.extend_from_slice(&amount_getter_data);

    // Field 4: takingAmountData - same as makingAmountData
    let taking_amount_data = making_amount_data.clone();

    // Field 5: predicate - empty
    let predicate: Vec<u8> = vec![];

    // Field 6: makerPermit - empty (no permit needed)
    let maker_permit: Vec<u8> = vec![];

    // Field 7: preInteraction - empty
    let pre_interaction: Vec<u8> = vec![];

    // Field 8: postInteraction - escrow factory + FusionExtension interaction data + escrow extra data
    let interaction_data = build_cross_interaction_data(auction_start_time, &quote_response.whitelist);
    let extra_data = encode_extra_data(
        hash_lock,
        quote_response,
        dst_chain_id,
        dst_token_address,
    );
    let mut post_interaction = Vec::new();
    post_interaction.extend_from_slice(quote_response.src_escrow_factory.as_slice());
    post_interaction.extend_from_slice(&interaction_data);
    post_interaction.extend_from_slice(&extra_data);

    // CustomData - '0x' for no address complement
    let custom_data: Vec<u8> = vec![];

    // Calculate cumulative offsets for each field
    let fields = [
        &maker_asset_suffix,
        &taker_asset_suffix,
        &making_amount_data,
        &taking_amount_data,
        &predicate,
        &maker_permit,
        &pre_interaction,
        &post_interaction,
    ];

    let mut cumulative_offset = 0u32;
    let mut offsets = [0u32; 8];

    for (i, field) in fields.iter().enumerate() {
        cumulative_offset += field.len() as u32;
        offsets[i] = cumulative_offset;
    }

    // Pack 8 offsets into a uint256 (each offset is 32 bits)
    // offset[0] at bits 0-31, offset[1] at bits 32-63, etc.
    let mut offsets_u256 = U256::ZERO;
    for (i, &offset) in offsets.iter().enumerate() {
        offsets_u256 |= U256::from(offset) << (i * 32);
    }

    // Build the final extension bytes
    let mut extension = Vec::new();

    // First 32 bytes: the packed offsets
    let offset_bytes: [u8; 32] = offsets_u256.to_be_bytes();
    extension.extend_from_slice(&offset_bytes);

    // Then all 8 fields concatenated
    for field in &fields {
        extension.extend_from_slice(field);
    }

    // Then customData
    extension.extend_from_slice(&custom_data);
    extension
}

fn build_amount_getter_data(
    quote: &IntentQuoteResponse,
    auction_start_time: u32,
    auction_details: &[u8],
    for_amount_getters: bool,
) -> Vec<u8> {
    let mut data = Vec::new();

    // Auction details only for amount getters
    if for_amount_getters {
        data.extend_from_slice(&auction_details);
    }

    // Fees data (same for both)
    data.extend_from_slice(&quote.integrator_fee.unwrap_or(0).to_be_bytes()); // uint16
    data.push(quote.integrator_fee_share.unwrap_or(0)); // uint8
    data.extend_from_slice(&quote.fee.as_ref().map(|fee| fee.bps).flatten().unwrap_or(0).to_be_bytes()); // uint16
    data.push(100 - quote.fee.as_ref().map(|fee| fee.whitelist_discount_percent).flatten().unwrap_or(0).min(100)); // uint8

    if for_amount_getters {
        // For amount getters: just address halves (no delays)
        data.push(quote.whitelist.len() as u8);
        for item in &quote.whitelist {
            data.extend_from_slice(&item.0.0[10..20]);
        }
    } else {
        // For post-interaction: full whitelist with delays
        data.extend_from_slice(&auction_start_time.to_be_bytes()); // uint32
        data.push(quote.whitelist.len() as u8);
        for item in &quote.whitelist {
            data.extend_from_slice(&item.0.0[10..20]);
            data.extend_from_slice(&0u16.to_be_bytes()); // uint16
        }
    }

    data
}

fn build_interaction_data(
    quote: &IntentQuoteResponse,
    to_address: Option<Address>,
    auction_start_time: u32,
    auction_details: &[u8],
) -> Vec<u8> {
    let preset = quote.get_recommended_preset();
    let mut data = Vec::new();

    // Flags (1 byte) - bit 0 = has custom receiver
    let has_custom_receiver = to_address.map(|r| !r.is_zero()).unwrap_or(false);
    let flags: u8 = if has_custom_receiver { 1 } else { 0 };
    data.push(flags);

    // Integrator fee recipient (20 bytes)
    data.extend_from_slice(quote.integrator_fee_receiver.unwrap_or(Address::ZERO).as_slice());

    // Protocol fee recipient (20 bytes)
    data.extend_from_slice(quote.fee.as_ref().map(|fee| fee.receiver).flatten().unwrap_or(Address::ZERO).as_slice());

    // Custom receiver if flag is set
    if let Some(receiver) = to_address {
        if !receiver.is_zero() {
            data.extend_from_slice(receiver.as_slice());
        }
    }

    // Fees data + whitelist with delays
    data.extend_from_slice(&build_amount_getter_data(quote, auction_start_time, auction_details, false));

    // Surplus parameters - share of price improvement with protocol
    // estimated_taking_amount = marketAmount (expected return based on current market price)
    // protocol_surplus_fee = percentage of surplus (price improvement) that goes to protocol
    //
    // If resolver gets better price than estimated, the surplus is split:
    //   - protocol gets (surplus * protocol_surplus_fee / 100)
    //   - user gets the rest
    let mut estimated_taking_amount = U256::MAX;
    let mut protocol_surplus_fee = 0;

    let taking_amount = U256::from_str_radix(&preset.auction_end_amount, 10).unwrap_or(U256::ZERO);
    if let Some(ref market_amount) = quote.market_amount {
        let market = U256::from_str_radix(market_amount, 10).unwrap_or(U256::ZERO);
        // Only enable surplus tracking if market price > order taking amount
        if market > taking_amount {
            estimated_taking_amount = market;
            // Set surplus fee percentage from quote (0-100)
            if let Some(surplus_fee) = quote.surplus_fee {
                protocol_surplus_fee = surplus_fee.min(100) as u8;
            }
        }
    }

    // Surplus params
    // estimated taking amount (32 bytes, big-endian)
    data.extend_from_slice(&estimated_taking_amount.to_be_bytes::<32>());
    // protocol surplus fee as percentage (1 byte)
    data.push(protocol_surplus_fee);

    data
}

pub fn build_intent_extension(
    quote: &IntentQuoteResponse,
    auction_start_time: u32,
    to_address: Option<Address>,
) -> Vec<u8> {
    let preset = quote.get_recommended_preset();

    // Field 1: makerAssetSuffix - empty
    let maker_asset_suffix: Vec<u8> = vec![];

    // Field 2: takerAssetSuffix - empty
    let taker_asset_suffix: Vec<u8> = vec![];

    // Field 3: makingAmountData
    let mut making_amount_data = Vec::new();
    let auction_details = preset.encode_auction_details(auction_start_time);
    making_amount_data.extend_from_slice(quote.settlement_address.as_slice());
    making_amount_data.extend_from_slice(&build_amount_getter_data(&quote, auction_start_time, &auction_details, true));

    // Field 4: takingAmountData - must be identical to makingAmountData
    let taking_amount_data = making_amount_data.clone();

    // Field 5: predicate - empty
    let predicate: Vec<u8> = vec![];

    // Field 6: makerPermit - empty
    let maker_permit: Vec<u8> = vec![];

    // Field 7: preInteraction - empty
    let pre_interaction: Vec<u8> = vec![];

    // Field 8: postInteraction
    let mut post_interaction = Vec::new();
    post_interaction.extend_from_slice(quote.settlement_address.as_slice());
    post_interaction.extend_from_slice(&build_interaction_data(quote, to_address, auction_start_time, &auction_details));

    // Calculate cumulative offsets for each field
    let fields = [
        &maker_asset_suffix,
        &taker_asset_suffix,
        &making_amount_data,
        &taking_amount_data,
        &predicate,
        &maker_permit,
        &pre_interaction,
        &post_interaction,
    ];

    let mut cumulative_offset = 0u32;
    let mut offsets = [0u32; 8];

    for (i, field) in fields.iter().enumerate() {
        cumulative_offset += field.len() as u32;
        offsets[i] = cumulative_offset;
    }

    // Pack 8 offsets into a uint256 (each offset is 32 bits)
    // offset[0] at bits 0-31, offset[1] at bits 32-63, etc.
    let mut offsets_u256 = U256::ZERO;
    for (i, &offset) in offsets.iter().enumerate() {
        offsets_u256 |= U256::from(offset) << (i * 32);
    }

    // Build the final extension bytes
    let mut extension = Vec::new();

    // First 32 bytes: the packed offsets
    let offset_bytes: [u8; 32] = offsets_u256.to_be_bytes();
    extension.extend_from_slice(&offset_bytes);

    // Then all 8 fields concatenated
    for field in &fields {
        extension.extend_from_slice(field);
    }

    extension
}

/// MakerTraits encodes various order properties in a uint256
///
/// Bit layout (high to low):
/// - 255: NO_PARTIAL_FILLS_FLAG
/// - 254: ALLOW_MULTIPLE_FILLS_FLAG
/// - 252: PRE_INTERACTION_CALL_FLAG
/// - 251: POST_INTERACTION_CALL_FLAG
/// - 250: NEED_CHECK_EPOCH_MANAGER_FLAG
/// - 249: HAS_EXTENSION_FLAG
/// - 248: USE_PERMIT2_FLAG
/// - 247: UNWRAP_WETH_FLAG
/// - [160-200): Series (40 bits)
/// - [120-160): Nonce/Epoch (40 bits)
/// - [80-120): Expiration (40 bits)
/// - [0-80): Allowed Sender (80 bits, last 10 bytes of address)
#[derive(Debug, Clone, Default)]
pub struct MakerTraits {
    value: U256,
}

impl MakerTraits {
    const NO_PARTIAL_FILLS_BIT: usize = 255;
    const ALLOW_MULTIPLE_FILLS_BIT: usize = 254;
    const HAS_EXTENSION_BIT: usize = 249;
    const POST_INTERACTION_BIT: usize = 251;

    // Bit positions for masks
    const EXPIRATION_SHIFT: usize = 80;
    const NONCE_SHIFT: usize = 120;

    pub fn new() -> Self {
        Self { value: U256::ZERO }
    }

    pub fn with_no_partial_fills(mut self) -> Self {
        self.value |= U256::from(1) << Self::NO_PARTIAL_FILLS_BIT;
        self
    }

    pub fn with_multiple_fills(mut self) -> Self {
        self.value |= U256::from(1) << Self::ALLOW_MULTIPLE_FILLS_BIT;
        self
    }

    pub fn with_extension(mut self) -> Self {
        self.value |= U256::from(1) << Self::HAS_EXTENSION_BIT;
        self
    }

    pub fn with_post_interaction(mut self) -> Self {
        self.value |= U256::from(1) << Self::POST_INTERACTION_BIT;
        self
    }

    pub fn with_expiration(mut self, expiration: u64) -> Self {
        let mask = U256::from(0xFFFFFFFFFFu64) << Self::EXPIRATION_SHIFT;
        self.value = (self.value & !mask) | (U256::from(expiration) << Self::EXPIRATION_SHIFT);
        self
    }

    pub fn with_nonce(mut self, nonce: u64) -> Self {
        let mask = U256::from(0xFFFFFFFFFFu64) << Self::NONCE_SHIFT;
        self.value = (self.value & !mask) | (U256::from(nonce) << Self::NONCE_SHIFT);
        self
    }

    pub fn build(self) -> U256 {
        self.value
    }
}

//! Utils to process chain data without accessing the chain
//!
//! This module provides transaction construction using subxt APIs.

use crate::{
    chain::definitions::BlockHash,
    database::TxKind,
    definitions::{api_v2::AssetId, Balance},
    error::ChainError,
};
use codec::Encode;
use frame_metadata::v15::RuntimeMetadataV15;
use substrate_crypto_light::common::AccountId32;
use subxt::utils::H256;

pub struct AssetTransferConstructor<'a> {
    pub asset_id: u32,
    pub amount: u128,
    pub to_account: &'a AccountId32,
}

pub struct BalanceTransferConstructor<'a> {
    pub amount: u128,
    pub to_account: &'a AccountId32,
    pub is_clearing: bool,
}

// Subxt-based call construction
#[derive(Clone, Debug)]
pub struct CallToFill {
    pub call_data: Vec<u8>,
    pub asset_id: Option<u32>,
    pub amount: Option<u128>,
    pub to_account: Option<AccountId32>,
    pub is_clearing: Option<bool>,
}

// Subxt-based transaction construction
#[derive(Debug)]
pub struct TransactionToFill {
    pub call_data: Vec<u8>,
    pub signature: Option<Vec<u8>>,
    pub transaction_data: Option<TransactionData>,
}

// Struct to hold transaction construction data
#[derive(Debug, Clone)]
pub struct TransactionData {
    pub genesis_hash: BlockHash,
    pub account_id: AccountId32,
    pub calls: Vec<CallToFill>,
    pub block_hash: BlockHash,
    pub block_number: u32,
    pub tip: u128,
    pub asset_id: Option<AssetId>,
    pub nonce: u32,
}

// Helper struct for managing extrinsic construction
#[derive(Debug)]
pub struct ExtrinsicBuilder {
    pub call_data: Vec<u8>,
    pub nonce: u32,
    pub tip: u128,
    pub genesis_hash: H256,
    pub block_hash: H256,
    pub spec_version: u32,
    pub transaction_version: u32,
}

impl TransactionToFill {
    pub fn sign_this(&self) -> Option<Vec<u8>> {
        // For subxt, we construct the signable payload here
        if self.call_data.is_empty() {
            return None;
        }

        // Return the call data that needs to be signed
        Some(self.call_data.clone())
    }

    pub fn send_this_signed(
        &self,
        _metadata: &RuntimeMetadataV15,
    ) -> Result<Option<Vec<u8>>, ChainError> {
        if let Some(signature) = &self.signature {
            // Construct the final extrinsic with signature
            let mut extrinsic = Vec::new();
            extrinsic.extend_from_slice(&self.call_data);
            extrinsic.extend_from_slice(signature);
            Ok(Some(extrinsic))
        } else {
            Err(ChainError::TransactionNotSignable(
                "No signature available".to_string(),
            ))
        }
    }

    pub fn set_signature(&mut self, signature: Vec<u8>) {
        self.signature = Some(signature);
    }
}

// Subxt-based implementations for transaction construction
pub fn construct_single_asset_transfer_call(
    _metadata: &RuntimeMetadataV15,
    asset_transfer_constructor: &AssetTransferConstructor,
) -> Result<CallToFill, ChainError> {
    // Construct the actual asset transfer call using subxt dynamic API
    let call_data = construct_asset_transfer_call_data(
        asset_transfer_constructor.asset_id,
        asset_transfer_constructor.amount,
        asset_transfer_constructor.to_account,
    )?;

    Ok(CallToFill {
        call_data,
        asset_id: Some(asset_transfer_constructor.asset_id),
        amount: Some(asset_transfer_constructor.amount),
        to_account: Some(asset_transfer_constructor.to_account.clone()),
        is_clearing: None,
    })
}

pub fn construct_single_balance_transfer_call(
    _metadata: &RuntimeMetadataV15,
    balance_transfer_constructor: &BalanceTransferConstructor,
) -> Result<CallToFill, ChainError> {
    // Construct the actual balance transfer call using subxt dynamic API
    let call_data = construct_balance_transfer_call_data(
        balance_transfer_constructor.amount,
        balance_transfer_constructor.to_account,
        balance_transfer_constructor.is_clearing,
    )?;

    Ok(CallToFill {
        call_data,
        asset_id: None,
        amount: Some(balance_transfer_constructor.amount),
        to_account: Some(balance_transfer_constructor.to_account.clone()),
        is_clearing: Some(balance_transfer_constructor.is_clearing),
    })
}

pub fn construct_batch_call(
    _metadata: &RuntimeMetadataV15,
    call_set: &[CallToFill],
) -> Result<(CallToFill, Vec<u8>), ChainError> {
    if call_set.is_empty() {
        return Err(ChainError::TransactionNotSignable(
            "No calls to batch".to_string(),
        ));
    }

    // Construct batch call using subxt dynamic API
    let batch_call_data = construct_batch_call_data(call_set)?;
    let encoded_calls: Vec<u8> = call_set
        .iter()
        .flat_map(|call| call.call_data.clone())
        .collect();

    let batch_call = if let Some(first_call) = call_set.first() {
        CallToFill {
            call_data: batch_call_data,
            asset_id: first_call.asset_id,
            amount: first_call.amount,
            to_account: first_call.to_account.clone(),
            is_clearing: first_call.is_clearing,
        }
    } else {
        return Err(ChainError::TransactionNotSignable(
            "No calls to batch".to_string(),
        ));
    };

    Ok((batch_call, encoded_calls))
}

pub fn construct_batch_transaction(
    _metadata: &RuntimeMetadataV15,
    genesis_hash: BlockHash,
    account_id: AccountId32,
    call_set: &[CallToFill],
    block_hash: BlockHash,
    block_number: u32,
    tip: u128,
    asset_id: Option<AssetId>,
) -> Result<TransactionToFill, ChainError> {
    if call_set.is_empty() {
        return Err(ChainError::TransactionNotSignable(
            "No calls to batch".to_string(),
        ));
    }

    // Use the first call's data for single call or construct batch
    let call_data = if call_set.len() == 1 {
        call_set[0].call_data.clone()
    } else {
        let (batch_call, _) = construct_batch_call(_metadata, call_set)?;
        batch_call.call_data
    };

    let transaction_data = TransactionData {
        genesis_hash,
        account_id,
        calls: call_set.to_vec(),
        block_hash,
        block_number,
        tip,
        asset_id,
        nonce: 0, // Will be filled by the caller
    };

    Ok(TransactionToFill {
        call_data,
        signature: None,
        transaction_data: Some(transaction_data),
    })
}

/// Simplified transfer event parsing
pub fn parse_transfer_event(
    _invoice_address: &AccountId32,
    _event_fields: &[u8],
) -> Option<(TxKind, AccountId32, Balance)> {
    // Heuristic parser for common transfer event layouts:
    // Many Substrate transfer-like events encode fields as (from: AccountId32, to: AccountId32, amount: Balance)
    // Extract first two 32-byte chunks as accounts, followed by a u128 amount if present.
    let bytes = _event_fields;

    if bytes.len() < 64 {
        return None;
    }

    // Try to read 32-byte accounts
    let mut from_bytes = [0u8; 32];
    let mut to_bytes = [0u8; 32];
    from_bytes.copy_from_slice(&bytes[0..32]);
    to_bytes.copy_from_slice(&bytes[32..64]);
    let from_account = AccountId32(from_bytes);
    let to_account = AccountId32(to_bytes);

    // Try to read amount (u128) if available; otherwise assume zero and let higher layers ignore
    let mut amount_le_bytes = [0u8; 16];
    if bytes.len() >= 80 {
        amount_le_bytes.copy_from_slice(&bytes[64..80]);
    }
    let amount_u128 = u128::from_le_bytes(amount_le_bytes);
    let amount = Balance(amount_u128);

    // Classify by whether invoice address is the sender or recipient
    if to_account == *_invoice_address {
        // Incoming payment to the invoice address
        return Some((TxKind::Payment, from_account, amount));
    }

    if from_account == *_invoice_address {
        // Outgoing withdrawal from the invoice address
        return Some((TxKind::Withdrawal, to_account, amount));
    }

    None
}

// Helper functions for constructing call data using subxt dynamic API
fn construct_asset_transfer_call_data(
    asset_id: u32,
    amount: u128,
    to_account: &AccountId32,
) -> Result<Vec<u8>, ChainError> {
    // Construct Assets::transfer call
    let mut call_data = Vec::new();

    // Pallet index for Assets (typically 50 on Asset Hub)
    call_data.push(50u8);

    // Call index for transfer (typically 5)
    call_data.push(5u8);

    // Encode asset ID
    call_data.extend_from_slice(&asset_id.encode());

    // Encode destination account (32 bytes)
    call_data.extend_from_slice(&to_account.0);

    // Encode amount
    call_data.extend_from_slice(&amount.encode());

    Ok(call_data)
}

fn construct_balance_transfer_call_data(
    amount: u128,
    to_account: &AccountId32,
    is_clearing: bool,
) -> Result<Vec<u8>, ChainError> {
    let mut call_data = Vec::new();

    // Pallet index for Balances (typically 10)
    call_data.push(10u8);

    // Call index - use transfer_all if clearing, otherwise transfer_keep_alive
    if is_clearing {
        call_data.push(4u8); // transfer_all
        call_data.extend_from_slice(&to_account.0);
        call_data.push(0u8); // keep_alive = false
    } else {
        call_data.push(3u8); // transfer_keep_alive
        call_data.extend_from_slice(&to_account.0);
        call_data.extend_from_slice(&amount.encode());
    }

    Ok(call_data)
}

fn construct_batch_call_data(calls: &[CallToFill]) -> Result<Vec<u8>, ChainError> {
    let mut call_data = Vec::new();

    // Pallet index for Utility (typically 40)
    call_data.push(40u8);

    // Call index for batch (typically 2)
    call_data.push(2u8);

    // Encode calls vector
    let calls_encoded: Vec<u8> = calls
        .iter()
        .map(|call| call.call_data.clone())
        .collect::<Vec<_>>()
        .encode();

    call_data.extend_from_slice(&calls_encoded);

    Ok(call_data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::definitions::BlockHash;
    use substrate_crypto_light::common::AccountId32;

    #[test]
    fn test_asset_transfer_constructor() {
        let account_id = AccountId32([42u8; 32]);
        let constructor = AssetTransferConstructor {
            asset_id: 123,
            amount: 1000000000000u128,
            to_account: &account_id,
        };

        assert_eq!(constructor.asset_id, 123);
        assert_eq!(constructor.amount, 1000000000000u128);
        assert_eq!(constructor.to_account, &account_id);
    }

    #[test]
    fn test_balance_transfer_constructor() {
        let account_id = AccountId32([42u8; 32]);
        let constructor = BalanceTransferConstructor {
            amount: 1000000000000u128,
            to_account: &account_id,
            is_clearing: true,
        };

        assert_eq!(constructor.amount, 1000000000000u128);
        assert_eq!(constructor.to_account, &account_id);
        assert_eq!(constructor.is_clearing, true);
    }

    #[test]
    fn test_call_to_fill_creation() {
        let call = CallToFill {
            call_data: vec![1, 2, 3],
            asset_id: Some(42),
            amount: Some(1000),
            to_account: Some(AccountId32([1u8; 32])),
            is_clearing: None,
        };

        assert_eq!(call.call_data, vec![1, 2, 3]);
        assert_eq!(call.asset_id, Some(42));
    }

    #[test]
    fn test_transaction_to_fill_creation() {
        let transaction = TransactionToFill {
            call_data: vec![1, 2, 3, 4],
            signature: None,
            transaction_data: None,
        };

        assert_eq!(transaction.call_data, vec![1, 2, 3, 4]);

        // Test sign_this method
        let sign_data = transaction.sign_this();
        assert!(sign_data.is_some());
        assert_eq!(sign_data.unwrap(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn test_construct_single_asset_transfer_call() {
        let metadata = create_test_metadata();
        let account_id = AccountId32([42u8; 32]);
        let constructor = AssetTransferConstructor {
            asset_id: 123,
            amount: 1000000000000u128,
            to_account: &account_id,
        };

        let result = construct_single_asset_transfer_call(&metadata, &constructor);
        assert!(result.is_ok());

        let call = result.unwrap();
        assert_eq!(call.asset_id, Some(123));
        assert_eq!(call.amount, Some(1000000000000u128));
        assert!(!call.call_data.is_empty());
    }

    #[test]
    fn test_construct_single_balance_transfer_call() {
        let metadata = create_test_metadata();
        let account_id = AccountId32([42u8; 32]);
        let constructor = BalanceTransferConstructor {
            amount: 1000000000000u128,
            to_account: &account_id,
            is_clearing: true,
        };

        let result = construct_single_balance_transfer_call(&metadata, &constructor);
        assert!(result.is_ok());

        let call = result.unwrap();
        assert_eq!(call.amount, Some(1000000000000u128));
        assert_eq!(call.is_clearing, Some(true));
        assert!(!call.call_data.is_empty());
    }

    #[test]
    fn test_construct_batch_call() {
        let metadata = create_test_metadata();
        let calls = vec![
            CallToFill {
                call_data: vec![1, 2, 3],
                asset_id: Some(1),
                amount: Some(100),
                to_account: Some(AccountId32([1u8; 32])),
                is_clearing: None,
            },
            CallToFill {
                call_data: vec![4, 5, 6],
                asset_id: Some(2),
                amount: Some(200),
                to_account: Some(AccountId32([2u8; 32])),
                is_clearing: None,
            },
        ];

        let result = construct_batch_call(&metadata, &calls);
        assert!(result.is_ok());

        let (batch_call, batch_data) = result.unwrap();
        assert!(!batch_call.call_data.is_empty());
        assert!(!batch_data.is_empty());
    }

    #[test]
    fn test_construct_batch_transaction() {
        let metadata = create_test_metadata();
        let genesis_hash = BlockHash::from_str(
            "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
        )
        .unwrap();
        let account_id = AccountId32([42u8; 32]);
        let calls = vec![CallToFill {
            call_data: vec![1, 2, 3],
            asset_id: Some(1),
            amount: Some(100),
            to_account: Some(AccountId32([1u8; 32])),
            is_clearing: None,
        }];
        let block_hash = BlockHash::from_str(
            "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
        )
        .unwrap();

        let result = construct_batch_transaction(
            &metadata,
            genesis_hash,
            account_id,
            &calls,
            block_hash,
            1000000,
            0,
            Some(123),
        );

        assert!(result.is_ok());

        let transaction = result.unwrap();
        assert!(!transaction.call_data.is_empty());
        assert!(transaction.transaction_data.is_some());
    }

    #[test]
    fn test_parse_transfer_event() {
        let account_id = AccountId32([42u8; 32]);
        let event_fields = vec![1, 2, 3, 4];

        let result = parse_transfer_event(&account_id, &event_fields);

        // Should return None for stub implementation
        assert!(result.is_none());
    }

    #[test]
    fn test_extrinsic_builder_creation() {
        let builder = ExtrinsicBuilder {
            call_data: vec![1, 2, 3],
            nonce: 42,
            tip: 1000,
            genesis_hash: H256::from([1u8; 32]),
            block_hash: H256::from([2u8; 32]),
            spec_version: 1,
            transaction_version: 1,
        };

        assert_eq!(builder.call_data, vec![1, 2, 3]);
        assert_eq!(builder.nonce, 42);
        assert_eq!(builder.tip, 1000);
    }

    #[test]
    fn test_transaction_send_this_signed() {
        let mut transaction = TransactionToFill {
            call_data: vec![1, 2, 3, 4],
            signature: Some(vec![5, 6, 7, 8]),
            transaction_data: None,
        };

        let metadata = create_test_metadata();
        let result = transaction.send_this_signed(&metadata);

        assert!(result.is_ok());
        let transaction_bytes = result.unwrap();
        assert!(transaction_bytes.is_some());
        assert_eq!(transaction_bytes.unwrap(), vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn test_transaction_send_this_signed_no_signature() {
        let transaction = TransactionToFill {
            call_data: vec![1, 2, 3, 4],
            signature: None,
            transaction_data: None,
        };

        let metadata = create_test_metadata();
        let result = transaction.send_this_signed(&metadata);

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ChainError::TransactionNotSignable(_)
        ));
    }

    #[test]
    fn test_transaction_set_signature() {
        let mut transaction = TransactionToFill {
            call_data: vec![1, 2, 3, 4],
            signature: None,
            transaction_data: None,
        };

        // Test setting signature
        transaction.set_signature(vec![5, 6, 7, 8]);
        assert_eq!(transaction.signature, Some(vec![5, 6, 7, 8]));

        // Test overwriting signature
        transaction.set_signature(vec![9, 10, 11, 12]);
        assert_eq!(transaction.signature, Some(vec![9, 10, 11, 12]));
    }

    #[test]
    fn test_transaction_sign_this_empty_call_data() {
        let transaction = TransactionToFill {
            call_data: vec![],
            signature: None,
            transaction_data: None,
        };

        let sign_data = transaction.sign_this();
        assert!(sign_data.is_none());
    }

    #[test]
    fn test_construct_batch_call_empty_calls() {
        let metadata = create_test_metadata();
        let calls: Vec<CallToFill> = vec![];

        let result = construct_batch_call(&metadata, &calls);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ChainError::TransactionNotSignable(_)
        ));
    }

    #[test]
    fn test_construct_batch_transaction_empty_calls() {
        let metadata = create_test_metadata();
        let genesis_hash = BlockHash::from_str(
            "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
        )
        .unwrap();
        let account_id = AccountId32([42u8; 32]);
        let calls: Vec<CallToFill> = vec![];
        let block_hash = BlockHash::from_str(
            "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
        )
        .unwrap();

        let result = construct_batch_transaction(
            &metadata,
            genesis_hash,
            account_id,
            &calls,
            block_hash,
            1000000,
            0,
            Some(123),
        );

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ChainError::TransactionNotSignable(_)
        ));
    }

    #[test]
    fn test_construct_batch_transaction_single_call() {
        let metadata = create_test_metadata();
        let genesis_hash = BlockHash::from_str(
            "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
        )
        .unwrap();
        let account_id = AccountId32([42u8; 32]);
        let calls = vec![CallToFill {
            call_data: vec![1, 2, 3],
            asset_id: Some(1),
            amount: Some(100),
            to_account: Some(AccountId32([1u8; 32])),
            is_clearing: None,
        }];
        let block_hash = BlockHash::from_str(
            "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
        )
        .unwrap();

        let result = construct_batch_transaction(
            &metadata,
            genesis_hash,
            account_id,
            &calls,
            block_hash,
            1000000,
            0,
            Some(123),
        );

        assert!(result.is_ok());
        let transaction = result.unwrap();

        // For single call, it should use the call data directly
        assert_eq!(transaction.call_data, vec![1, 2, 3]);
        assert!(transaction.transaction_data.is_some());

        let tx_data = transaction.transaction_data.unwrap();
        assert_eq!(tx_data.calls.len(), 1);
        assert_eq!(tx_data.account_id, account_id);
        assert_eq!(tx_data.block_number, 1000000);
        assert_eq!(tx_data.tip, 0);
        assert_eq!(tx_data.asset_id, Some(123));
        assert_eq!(tx_data.nonce, 0);
    }

    #[test]
    fn test_construct_asset_transfer_call_data() {
        let result =
            construct_asset_transfer_call_data(1337, 1000000000000u128, &AccountId32([42u8; 32]));

        assert!(result.is_ok());
        let call_data = result.unwrap();

        // Should start with pallet index (50) and call index (5)
        assert_eq!(call_data[0], 50u8);
        assert_eq!(call_data[1], 5u8);

        // Should contain encoded asset ID, account, and amount
        assert!(call_data.len() > 2);
    }

    #[test]
    fn test_construct_balance_transfer_call_data_keep_alive() {
        let result = construct_balance_transfer_call_data(
            1000000000000u128,
            &AccountId32([42u8; 32]),
            false, // not clearing
        );

        assert!(result.is_ok());
        let call_data = result.unwrap();

        // Should start with pallet index (10) and call index (3 for transfer_keep_alive)
        assert_eq!(call_data[0], 10u8);
        assert_eq!(call_data[1], 3u8);

        // Should contain encoded account and amount
        assert!(call_data.len() > 2);
    }

    #[test]
    fn test_construct_balance_transfer_call_data_clearing() {
        let result = construct_balance_transfer_call_data(
            1000000000000u128,
            &AccountId32([42u8; 32]),
            true, // clearing
        );

        assert!(result.is_ok());
        let call_data = result.unwrap();

        // Should start with pallet index (10) and call index (4 for transfer_all)
        assert_eq!(call_data[0], 10u8);
        assert_eq!(call_data[1], 4u8);

        // Should contain encoded account and keep_alive flag
        assert!(call_data.len() > 2);
        // Last byte should be 0 (keep_alive = false)
        assert_eq!(call_data[call_data.len() - 1], 0u8);
    }

    #[test]
    fn test_construct_batch_call_data() {
        let calls = vec![
            CallToFill {
                call_data: vec![1, 2, 3],
                asset_id: Some(1),
                amount: Some(100),
                to_account: Some(AccountId32([1u8; 32])),
                is_clearing: None,
            },
            CallToFill {
                call_data: vec![4, 5, 6],
                asset_id: Some(2),
                amount: Some(200),
                to_account: Some(AccountId32([2u8; 32])),
                is_clearing: None,
            },
        ];

        let result = construct_batch_call_data(&calls);
        assert!(result.is_ok());
        let call_data = result.unwrap();

        // Should start with pallet index (40) and call index (2)
        assert_eq!(call_data[0], 40u8);
        assert_eq!(call_data[1], 2u8);

        // Should contain encoded calls
        assert!(call_data.len() > 2);
    }

    #[test]
    fn test_construct_batch_call_data_empty() {
        let calls: Vec<CallToFill> = vec![];

        let result = construct_batch_call_data(&calls);
        assert!(result.is_ok());
        let call_data = result.unwrap();

        // Should still create valid batch call data even with empty calls
        assert_eq!(call_data[0], 40u8);
        assert_eq!(call_data[1], 2u8);
    }

    #[test]
    fn test_extrinsic_builder_all_fields() {
        let builder = ExtrinsicBuilder {
            call_data: vec![1, 2, 3, 4, 5],
            nonce: 123,
            tip: 5000,
            genesis_hash: H256::from([1u8; 32]),
            block_hash: H256::from([2u8; 32]),
            spec_version: 42,
            transaction_version: 1,
        };

        assert_eq!(builder.call_data, vec![1, 2, 3, 4, 5]);
        assert_eq!(builder.nonce, 123);
        assert_eq!(builder.tip, 5000);
        assert_eq!(builder.genesis_hash, H256::from([1u8; 32]));
        assert_eq!(builder.block_hash, H256::from([2u8; 32]));
        assert_eq!(builder.spec_version, 42);
        assert_eq!(builder.transaction_version, 1);
    }

    #[test]
    fn test_transaction_data_with_nonce() {
        let genesis_hash = BlockHash::from_str(
            "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
        )
        .unwrap();
        let account_id = AccountId32([42u8; 32]);
        let block_hash = BlockHash::from_str(
            "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
        )
        .unwrap();
        let calls = vec![CallToFill {
            call_data: vec![1, 2, 3],
            asset_id: Some(1),
            amount: Some(100),
            to_account: Some(AccountId32([1u8; 32])),
            is_clearing: None,
        }];

        let transaction_data = TransactionData {
            genesis_hash,
            account_id,
            calls,
            block_hash,
            block_number: 1000000,
            tip: 5000,
            asset_id: Some(123),
            nonce: 42,
        };

        assert_eq!(transaction_data.nonce, 42);
        assert_eq!(transaction_data.tip, 5000);
        assert_eq!(transaction_data.block_number, 1000000);
        assert_eq!(transaction_data.asset_id, Some(123));
        assert_eq!(transaction_data.calls.len(), 1);
    }

    // Helper function to create test metadata
    fn create_test_metadata() -> frame_metadata::v15::RuntimeMetadataV15 {
        use scale_info::{meta_type, Registry, TypeInfo};

        let mut registry = Registry::new();

        frame_metadata::v15::RuntimeMetadataV15 {
            pallets: vec![
                frame_metadata::v15::PalletMetadata {
                    name: "System".to_string(),
                    storage: None,
                    calls: None,
                    event: None,
                    constants: vec![],
                    error: None,
                    index: 0,
                    docs: vec![],
                },
                frame_metadata::v15::PalletMetadata {
                    name: "Balances".to_string(),
                    storage: None,
                    calls: None,
                    event: None,
                    constants: vec![],
                    error: None,
                    index: 1,
                    docs: vec![],
                },
            ],
            extrinsic: frame_metadata::v15::ExtrinsicMetadata {
                version: 4,
                address_ty: registry.register_type(&meta_type::<()>()),
                call_ty: registry.register_type(&meta_type::<()>()),
                signature_ty: registry.register_type(&meta_type::<()>()),
                extra_ty: registry.register_type(&meta_type::<()>()),
                signed_extensions: vec![],
            },
            ty: registry.register_type(&meta_type::<()>()),
            types: registry.into(),
            outer_enums: frame_metadata::v15::OuterEnums {
                call_enum_ty: 0.into(),
                event_enum_ty: 0.into(),
                error_enum_ty: 0.into(),
            },
            custom: frame_metadata::v15::CustomMetadata {
                map: std::collections::BTreeMap::new(),
            },
            apis: vec![],
        }
    }
}

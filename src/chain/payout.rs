//! Separate engine for payout process.
//!
//! This is so unimportant for smooth SALES process, that it should be given the
//! lowest possible priority, optimized for lazy and very delayed process, and
//! in some cases might be disabeled altogether (TODO)

use chrono::{
    DateTime,
    Utc,
};
use tracing::info;
use uuid::Uuid;

use crate::chain::definitions::Invoice;
use crate::chain::tracker::ChainWatcher;
use crate::chain::utils::to_base58_string;
use crate::chain_client::{
    AssetHubClient,
    BlockChainClient,
    Encodeable,
    KeyringClient,
};
use crate::error::ChainError;
use crate::state::State;
use crate::types::{
    OutgoingTransactionMeta,
    Transaction,
    TransactionOrigin,
    TransactionStatus,
    TransactionType,
};

/// Single function that should completely handle payout attmept. Just do not
/// call anything else.
///
/// TODO: make this an additional runner independent from chain monitors
#[expect(clippy::too_many_lines)]
pub async fn payout(
    client: AssetHubClient,
    order: Invoice,
    state: State,
    chain: ChainWatcher,
    keyring_client: KeyringClient,
) -> Result<(), ChainError> {
    // TODO: make this retry and rotate RPCs maybe
    //
    // after some retries record a failure
    let currency = chain
        .assets
        .get(&order.currency.currency)
        .ok_or_else(|| ChainError::InvalidCurrency(order.currency.currency.clone()))?;

    let asset_id = currency
        .asset_id
        .ok_or(ChainError::AssetId)?;

    let derivation_params = vec![order.order_id];

    let sender = keyring_client
        .generate_asset_hub_address(derivation_params.clone().into())
        .await
        .unwrap();

    let amount = client
        .fetch_asset_balance(&asset_id, &sender)
        .await
        .unwrap();

    info!("Got balance for payout: {:?}", amount);

    let transaction = client
        .build_transfer_all(&sender, &order.recipient, &asset_id)
        .await
        .unwrap();

    let signed_transaction = client
        .sign_transaction(
            transaction,
            derivation_params,
            &keyring_client,
        )
        .await
        .unwrap();

    // TODO: why 42? perhaps store it into constant?
    let sender = to_base58_string(sender.0, 42);
    let recipient = to_base58_string(order.recipient.0, 42);

    let encoded_extrinsic = signed_transaction.to_hex_string();

    // Generate transaction ID for tracking
    let transaction_id = Uuid::new_v4();
    let now = Utc::now();

    // Build transaction record with Waiting status
    let mut tx_record = Transaction {
        id: transaction_id,
        invoice_id: order.id,
        asset_id,
        chain: order.currency.chain_name.clone(),
        amount,
        sender: sender.clone(),
        recipient: recipient.clone(),
        block_number: None,
        position_in_block: None,
        tx_hash: None,
        origin: TransactionOrigin::default(),
        status: TransactionStatus::Waiting,
        transaction_type: TransactionType::Outgoing,
        outgoing_meta: OutgoingTransactionMeta {
            extrinsic_bytes: Some(encoded_extrinsic.clone()),
            built_at: Some(now),
            sent_at: None,
            confirmed_at: None,
            failed_at: None,
            failure_message: None,
        },
        created_at: now,
        transaction_bytes: Some(encoded_extrinsic.clone()),
    };

    // Record transaction with Waiting status
    state
        .record_transaction_v2(order.id, tx_record.clone())
        .await
        .map_err(|_| ChainError::TransactionNotSaved)?;

    info!("First transaction record saved");

    // Attempt payout with error handling
    let payout_result = async {
        // Update to InProgress when submitting
        tx_record.status = TransactionStatus::InProgress;
        tx_record.outgoing_meta.sent_at = Some(Utc::now());

        state
            .update_transaction_v2(tx_record.clone())
            .await
            .map_err(|_| ChainError::TransactionNotSaved)?;

        let result = client
            .submit_and_watch_transaction(signed_transaction)
            .await
            // TODO: Errors will be reworked later
            // TODO: handle failed statuses
            .map_err(|_| ChainError::TransferEventNoExtrinsic)?;

        // Update to Completed
        tx_record.status = TransactionStatus::Completed;
        tx_record.block_number = Some(result.transaction_id.0);
        tx_record.position_in_block = Some(result.transaction_id.1);
        // TODO: probably it's gonna be easier to just use UTC::DateTime everywhere - including timestamps in asset hub client
        tx_record.outgoing_meta.confirmed_at = Some(
            #[expect(clippy::cast_possible_wrap)]
            DateTime::from_timestamp_millis(result.timestamp as i64).unwrap_or_else(Utc::now),
        );
        // TODO: add another field `executed_amount` which can be a bit different from
        // the requested amount
        tx_record.amount = result.amount;

        state
            .update_transaction_v2(tx_record.clone())
            .await
            .map_err(|_| ChainError::TransactionNotSaved)?;

        info!("Transaction record updated to Completed");

        Ok::<(), ChainError>(())
    }
    .await;

    // Handle payout result and mark as failed if error
    match payout_result {
        Ok(()) => {
            state.order_withdrawn(order.id).await;
            Ok(())
        },
        Err(e) => {
            // Mark transaction as failed
            tx_record.status = TransactionStatus::Failed;
            tx_record.outgoing_meta.failed_at = Some(Utc::now());
            tx_record.outgoing_meta.failure_message = Some(e.to_string());

            // Try to update, but don't fail if update fails
            drop(
                state
                    .update_transaction_v2(tx_record)
                    .await,
            );

            Err(e)
        },
    }
}

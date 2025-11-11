//! Separate engine for payout process.
//!
//! This is so unimportant for smooth SALES process, that it should be given the lowest possible
//! priority, optimized for lazy and very delayed process, and in some cases might be disabeled
//! altogether (TODO)

use crate::{
    chain::{
        AssetHubConfig, AssetHubOnlineClient,
        definitions::Invoice,
        runtime::runtime_types::{
            staging_xcm::v3::multilocation::MultiLocation,
            xcm::v3::{junction::Junction, junctions::Junctions},
        },
        tracker::ChainWatcher,
        utils::to_base58_string,
    },
    error::{ChainError, SignerError},
    state::State,
    types::{OutgoingTransactionMeta, Transaction, TransactionOrigin, TransactionStatus, TransactionType},
};
use chrono::Utc;
use rust_decimal::Decimal;
use subxt::config::DefaultExtrinsicParamsBuilder;
use subxt_signer::{DeriveJunction, ExposeSecret, SecretString, bip39::Mnemonic, sr25519::Keypair};
use tracing::info;
use uuid::Uuid;

/// Single function that should completely handle payout attmept. Just do not call anything else.
///
/// TODO: make this an additional runner independent from chain monitors
pub async fn payout(
    client: AssetHubOnlineClient,
    order: Invoice,
    state: State,
    chain: ChainWatcher,
    seed: SecretString,
) -> Result<(), ChainError> {
    // TODO: make this retry and rotate RPCs maybe
    //
    // after some retries record a failure
    let currency = chain
        .assets
        .get(&order.currency.currency)
        .ok_or_else(|| ChainError::InvalidCurrency(order.currency.currency.clone()))?;

    let asset_id = currency.asset_id.ok_or(ChainError::AssetId)?;
    info!("Currency checked");
    let dest = subxt::utils::AccountId32::from(order.recipient.0);

    let call = crate::chain::runtime::tx()
        .assets()
        .transfer_all(asset_id, dest.into(), false);

    // TODO: set pallet instance and parents to consts?
    let location = MultiLocation {
        parents: 0,
        interior: Junctions::X2(
            Junction::PalletInstance(50),
            Junction::GeneralIndex(u128::from(asset_id)),
        ),
    };

    let tx_config = DefaultExtrinsicParamsBuilder::<AssetHubConfig>::new()
        .tip_of(0, location)
        .mortal(32)
        .build();

    // TODO: need to validate phrase on start and somehow handle error cause it should be unexpected and probably can happen only if we zeroize seed too early
    let mnemonic = Mnemonic::parse(seed.expose_secret()).map_err(SignerError::from)?;
    // TODO: add support for password configuration and also validate keypair creation on start too
    let keypair = Keypair::from_phrase(&mnemonic, None).map_err(SignerError::from)?;

    info!("Keypair initiated");

    let order_keypair = keypair.derive([
        DeriveJunction::hard(to_base58_string(order.recipient.0, 2)),
        DeriveJunction::hard(order.id.to_string()),
    ]);

    // TODO: why 42? perhaps store it into constant?
    let sender = to_base58_string(order_keypair.public_key().0, 42);
    let recipient = to_base58_string(order.recipient.0, 42);

    let transaction = client
        .tx()
        .create_signed(&call, &order_keypair, tx_config)
        .await?;
    let encoded_extrinsic = const_hex::encode_prefixed(transaction.encoded());

    // Generate transaction ID for tracking
    let transaction_id = Uuid::new_v4();
    let now = Utc::now();

    // Build transaction record with Waiting status
    let mut tx_record = Transaction {
        id: transaction_id,
        invoice_id: order.id,
        asset_id,
        chain: order.currency.chain_name.clone(),
        amount: Decimal::ZERO,  // Will remain zero for transfer_all operations
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

        info!("Transaction record updated to InProgress");

        // Submit and watch for completion
        let result = transaction.submit_and_watch().await.inspect_err(|e| tracing::error!("Got error while submit transaction: {:?}", e))?;

        info!("Transaction watch result");

        // Wait for finalization
        let finalized = result.wait_for_finalized().await?;
        let events = finalized.fetch_events().await?;

        info!("Transaction finalized");

        // Get block number from the finalized block
        let block_hash = finalized.block_hash();
        let block = client.blocks().at(block_hash).await?;
        let block_number = block.number();
        let position_in_block = events.extrinsic_index();

        // Update to Completed
        tx_record.status = TransactionStatus::Completed;
        tx_record.block_number = Some(block_number);
        tx_record.position_in_block = Some(position_in_block);
        tx_record.outgoing_meta.confirmed_at = Some(Utc::now());

        // TODO: Extract actual transferred amount from events if needed
        // For now, keeping amount as Decimal::ZERO for transfer_all operations

        state
            .update_transaction_v2(tx_record.clone())
            .await
            .map_err(|_| ChainError::TransactionNotSaved)?;

        info!("Transaction record updated to Completed");

        Ok::<(), ChainError>(())
    }.await;

    // Handle payout result and mark as failed if error
    match payout_result {
        Ok(()) => {
            state.order_withdrawn(order.id).await;
            Ok(())
        }
        Err(e) => {
            // Mark transaction as failed
            tx_record.status = TransactionStatus::Failed;
            tx_record.outgoing_meta.failed_at = Some(Utc::now());
            tx_record.outgoing_meta.failure_message = Some(e.to_string());

            // Try to update, but don't fail if update fails
            drop(state.update_transaction_v2(tx_record).await);

            Err(e)
        }
    }
}

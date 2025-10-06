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
    database::{TransactionInfoDb, TransactionInfoDbInner, TxKind},
    definitions::api_v2::{Amount, TxStatus},
    error::ChainError,
    state::State,
};
use subxt::config::DefaultExtrinsicParamsBuilder;
use subxt_signer::{DeriveJunction, ExposeSecret, SecretString, bip39::Mnemonic, sr25519::Keypair};

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

    // TODO: need to validate phrase on start so unwrap here can be safe
    let mnemonic = Mnemonic::parse(seed.expose_secret()).unwrap();
    // TODO: add support for password configuration, ensure unwrap here is safe
    let keypair = Keypair::from_phrase(&mnemonic, None).unwrap();

    let order_keypair = keypair.derive([
        DeriveJunction::hard(to_base58_string(order.recipient.0, 2)),
        DeriveJunction::hard(&order.id),
    ]);

    // TODO: why 42? perhaps store it into constant?
    let sender = to_base58_string(order_keypair.public_key().0, 42);

    let transaction = client
        .tx()
        .create_signed(&call, &order_keypair, tx_config)
        .await?;
    let encoded_extrinsic = const_hex::encode_prefixed(transaction.encoded());

    state
        .record_transaction(
            TransactionInfoDb {
                transaction_bytes: encoded_extrinsic.clone(),
                inner: TransactionInfoDbInner {
                    finalized_tx_timestamp: None,
                    finalized_tx: None,
                    sender,
                    recipient: to_base58_string(order.recipient.0, 42),
                    amount: Amount::All,
                    currency: order.currency,
                    status: TxStatus::Pending,
                    kind: TxKind::Withdrawal,
                },
            },
            order.id.clone(),
        )
        .await
        .map_err(|_| ChainError::TransactionNotSaved)?;

    // send_stuff(&client, &encoded_extrinsic).await?;
    // TODO: handle result, check transfer details, find our transfer event, store it's details
    let _result = transaction.submit_and_watch().await?;

    state.order_withdrawn(order.id).await;
    // TODO obvious
    Ok(())
}

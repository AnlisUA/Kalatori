//! Separate engine for payout process.
//!
//! This is so unimportant for smooth SALES process, that it should be given the lowest possible
//! priority, optimized for lazy and very delayed process, and in some cases might be disabeled
//! altogether (TODO)

use crate::{
    chain::{
        definitions::Invoice, runtime::runtime_types::{staging_xcm::v3::multilocation::MultiLocation, xcm::v3::{junction::Junction, junctions::Junctions}}, tracker::ChainWatcher, AssetHubConfig, AssetHubOnlineClient
    },
    database::{TransactionInfoDb, TransactionInfoDbInner, TxKind},
    definitions::{
        api_v2::{Amount, TxStatus},
    },
    error::ChainError,
    state::State,
};
use base58::ToBase58;
use substrate_crypto_light::common::AsBase58;
use subxt::config::DefaultExtrinsicParamsBuilder;
use subxt_signer::{bip39::Mnemonic, sr25519::Keypair, DeriveJunction, ExposeSecret, SecretString};

// TODO: move it out to utils or use something similar from separate crate?
pub const HASH_512_LEN: usize = 64;
pub const BASE58_ID: &[u8] = b"SS58PRE";

fn ss58hash(data: &[u8]) -> [u8; HASH_512_LEN] {
    let mut blake2b_state = blake2b_simd::Params::new()
        .hash_length(HASH_512_LEN)
        .to_state();
    blake2b_state.update(BASE58_ID);
    blake2b_state.update(data);
    blake2b_state
        .finalize()
        .as_bytes()
        .try_into()
        .expect("static length, always fits")
}

// Same as `to_ss58check_with_version()` method for `Ss58Codec` from `sp_core`, comments from `sp_core`.
fn to_base58_string(bytes: [u8; 32], base58prefix: u16) -> String {
    // We mask out the upper two bits of the ident - SS58 Prefix currently only supports 14-bits
    let ident: u16 = base58prefix & 0b0011_1111_1111_1111;
    let mut v = match ident {
        0..=63 => vec![ident as u8],
        64..=16_383 => {
            // upper six bits of the lower byte(!)
            let first = ((ident & 0b0000_0000_1111_1100) as u8) >> 2;
            // lower two bits of the lower byte in the high pos,
            // lower bits of the upper byte in the low pos
            let second = ((ident >> 8) as u8) | ((ident & 0b0000_0000_0000_0011) as u8) << 6;
            vec![first | 0b0100_0000, second]
        }
        _ => unreachable!("masked out the upper two bits; qed"),
    };
    v.extend(bytes);
    let r = ss58hash(&v);
    v.extend(&r[0..2]);
    v.to_base58()
}

/// Single function that should completely handle payout attmept. Just do not call anything else.
///
/// TODO: make this an additional runner independent from chain monitors
#[expect(clippy::too_many_lines, clippy::arithmetic_side_effects)]
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

    let call = crate::chain::runtime::tx().assets().transfer_all(asset_id, dest.into(), false);

    // TODO: set pallet instance and parents to consts?
    let location = MultiLocation {
        parents: 0,
        interior: Junctions::X2(Junction::PalletInstance(50), Junction::GeneralIndex(asset_id as u128)),
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
        DeriveJunction::hard(order.recipient.to_base58_string(2)),
        DeriveJunction::hard(&order.id),
    ]);

    // TODO: why 42? perhaps store it into constant?
    let sender = to_base58_string(order_keypair.public_key().0, 42);

    let transaction = client.tx().create_signed(&call, &order_keypair, tx_config).await?;
    let encoded_extrinsic = const_hex::encode_prefixed(transaction.encoded());

    state
        .record_transaction(
            TransactionInfoDb {
                transaction_bytes: encoded_extrinsic.clone(),
                inner: TransactionInfoDbInner {
                    finalized_tx_timestamp: None,
                    finalized_tx: None,
                    sender,
                    recipient: order.recipient.to_base58_string(42),
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

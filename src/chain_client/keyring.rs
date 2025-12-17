use bip39::Mnemonic;
use subxt_signer::sr25519::Keypair;
use subxt_signer::{
    DeriveJunction,
    ExposeSecret,
    SecretString,
};
use thiserror::Error;
use tokio::sync::{
    mpsc,
    oneshot,
};
use tracing::instrument;
use zeroize::{
    Zeroize,
    ZeroizeOnDrop,
};

use super::asset_hub::{
    AssetHubAccountId,
    AssetHubSignedTransaction,
    AssetHubUnsignedTransaction,
};

const KEYRING_CHANNEL_CAPACITY: usize = 32;

pub type DerivationParams = Vec<String>;

pub struct SignTransactionRequestData<T> {
    pub transaction: T,
    pub derivation_params: DerivationParams,
}

pub struct GenerateAddressData {
    pub derivation_params: DerivationParams,
}

impl From<DerivationParams> for GenerateAddressData {
    fn from(derivation_params: DerivationParams) -> Self {
        Self {
            derivation_params,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, Error)]
pub enum KeyringError {
    #[error("Seed is invalid")]
    InvalidSeed,
    #[error("Unexpected error while send request to or receive response from Keyring")]
    MessageTransmissionFailed,
    #[expect(dead_code)]
    #[error("Timeout while send request to Keyring")]
    ResponseTimeout,
}

pub type KeyringResult<T> = Result<T, KeyringError>;

#[derive(Zeroize, ZeroizeOnDrop)]
pub struct Keyring {
    asset_hub_seed: SecretString,
}

impl Keyring {
    // TODO: receive secrets config
    pub fn new(seed: SecretString) -> Self {
        Self {
            asset_hub_seed: seed,
        }
    }

    pub fn ignite(
        self
    ) -> (
        tokio::task::JoinHandle<()>,
        KeyringClient,
    ) {
        let (tx, mut rx) = mpsc::channel(KEYRING_CHANNEL_CAPACITY);

        let handle = tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                self.process_message(msg);
            }

            tracing::info!("Keyring actor has been shut down");
        });

        let client = KeyringClient {
            tx,
        };

        (handle, client)
    }

    fn generate_asset_hub_derived_keypair(
        &self,
        params: DerivationParams,
    ) -> KeyringResult<Keypair> {
        let mut mnemonic = Mnemonic::parse(self.asset_hub_seed.expose_secret())
            .map_err(|_| KeyringError::InvalidSeed)?;

        let keypair =
            Keypair::from_phrase(&mnemonic, None).map_err(|_| KeyringError::InvalidSeed)?;

        mnemonic.zeroize();

        let derived_keypair = keypair.derive(
            params
                .into_iter()
                .map(DeriveJunction::hard),
        );

        Ok(derived_keypair)
    }

    fn process_sign_asset_hub_transaction(
        &self,
        data: SignTransactionRequestData<AssetHubUnsignedTransaction>,
    ) -> KeyringResult<AssetHubSignedTransaction> {
        let SignTransactionRequestData {
            mut transaction,
            derivation_params,
        } = data;
        let derived_keypair = self.generate_asset_hub_derived_keypair(derivation_params)?;
        Ok(transaction.sign(&derived_keypair))
    }

    fn process_generate_asset_hub_address(
        &self,
        data: GenerateAddressData,
    ) -> KeyringResult<AssetHubAccountId> {
        let derived_keypair = self.generate_asset_hub_derived_keypair(data.derivation_params)?;
        Ok(derived_keypair
            .public_key()
            .to_account_id())
    }

    fn process_message(
        &self,
        msg: KeyringMessage,
    ) {
        match msg {
            KeyringMessage::SignAssetHubTransaction(envelope) => {
                let (req, resp) = envelope.unpack();
                let result = self.process_sign_asset_hub_transaction(req);
                // TODO: add logs
                let _unused = resp.send(result);
            },
            KeyringMessage::GenerateAssetHubAddress(envelope) => {
                let (req, resp) = envelope.unpack();
                let result = self.process_generate_asset_hub_address(req);
                // TODO: add logs
                let _unused = resp.send(result);
            },
        }
    }
}

struct Envelope<T, R> {
    request: T,
    responder: oneshot::Sender<KeyringResult<R>>,
}

impl<T, R> Envelope<T, R> {
    pub fn new(request: T) -> (Self, ResponseReceiver<R>) {
        let (responder, receiver) = oneshot::channel();

        let envelope = Envelope {
            request,
            responder,
        };

        let response_receiver = ResponseReceiver {
            receiver,
        };

        (envelope, response_receiver)
    }

    pub fn unpack(self) -> (T, oneshot::Sender<KeyringResult<R>>) {
        let Envelope {
            request,
            responder,
        } = self;
        (request, responder)
    }
}

struct ResponseReceiver<R> {
    receiver: oneshot::Receiver<KeyringResult<R>>,
}

impl<R> ResponseReceiver<R> {
    async fn receive(self) -> KeyringResult<R> {
        // TODO: add timeouts?
        self.receiver
            .await
            .inspect_err(|e| {
                tracing::debug!(
                    error.category = crate::utils::logging::category::CHAIN_CLIENT,
                    error.operation = "keyring_response",
                    error.source = ?e,
                    "Failed to receive response from Keyring actor"
                );
            })
            .map_err(|_| KeyringError::MessageTransmissionFailed)?
    }
}

// adding new operations can be simplified using macros
enum KeyringMessage {
    SignAssetHubTransaction(
        Envelope<
            SignTransactionRequestData<AssetHubUnsignedTransaction>,
            AssetHubSignedTransaction,
        >,
    ),
    GenerateAssetHubAddress(Envelope<GenerateAddressData, AssetHubAccountId>),
}

impl KeyringMessage {
    fn new_sign_asset_hub_transaction(
        data: SignTransactionRequestData<AssetHubUnsignedTransaction>
    ) -> (
        Self,
        ResponseReceiver<AssetHubSignedTransaction>,
    ) {
        let (envelope, response_receiver) = Envelope::new(data);
        (
            Self::SignAssetHubTransaction(envelope),
            response_receiver,
        )
    }

    fn new_generate_asset_hub_address(
        data: GenerateAddressData
    ) -> (
        Self,
        ResponseReceiver<AssetHubAccountId>,
    ) {
        let (envelope, response_receiver) = Envelope::new(data);
        (
            Self::GenerateAssetHubAddress(envelope),
            response_receiver,
        )
    }
}

#[derive(Clone)]
pub struct KeyringClient {
    tx: mpsc::Sender<KeyringMessage>,
}

impl KeyringClient {
    async fn send_message_with_response<R>(
        &self,
        message: KeyringMessage,
        response_receiver: ResponseReceiver<R>,
    ) -> KeyringResult<R> {
        let () = self
            .tx
            .send(message)
            .await
            .inspect_err(|e| {
                tracing::debug!(
                    error.category = crate::utils::logging::category::CHAIN_CLIENT,
                    error.operation = "keyring_request",
                    error.source = ?e,
                    "Failed to send request to Keyring actor"
                );
            })
            .map_err(|_| KeyringError::MessageTransmissionFailed)?;

        response_receiver.receive().await
    }

    #[instrument(skip(self, data))]
    pub async fn sign_asset_hub_transaction(
        &self,
        data: SignTransactionRequestData<AssetHubUnsignedTransaction>,
    ) -> KeyringResult<AssetHubSignedTransaction> {
        let params = KeyringMessage::new_sign_asset_hub_transaction(data);
        self.send_message_with_response(params.0, params.1)
            .await
    }

    #[instrument(skip(self, data))]
    pub async fn generate_asset_hub_address(
        &self,
        data: GenerateAddressData,
    ) -> KeyringResult<AssetHubAccountId> {
        let params = KeyringMessage::new_generate_asset_hub_address(data);
        self.send_message_with_response(params.0, params.1)
            .await
    }
}

#[cfg(test)]
pub fn default_keyring_client() -> KeyringClient {
    let (tx, rx) = mpsc::channel(KEYRING_CHANNEL_CAPACITY);
    KeyringClient { tx }
}

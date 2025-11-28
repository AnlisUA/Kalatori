//! This is a tiny worker to hold secret key
//! We use it to avoid sending it back and forth through async pipes
//! so that we can be sure that zeroizing at least tries to do its thing
//!
//! Keep in mind, that leaking secrets in a system like Kalatori is a serious threat
//! with delayed attacks taken into account. Of course, some secret rotation scheme must
//! be implemented, but it seems likely that it would be neglected occasionally.
//! So we go to trouble of running this separate process.
//!
//! Also this abstraction could be used to implement off-system signer

use subxt::utils::AccountId32;
use subxt_signer::{DeriveJunction, ExposeSecret, SecretString, bip39::Mnemonic, sr25519::Keypair};
use tokio::sync::{mpsc, oneshot};
use zeroize::Zeroize;

use crate::chain::utils::to_base58_string;
use crate::{error::SignerError, utils::task_tracker::TaskTracker};

/// Signer handle
pub struct Signer {
    tx: mpsc::Sender<SignerRequest>,
}

impl Signer {
    /// Run once to initialize; this should do **all** secret management
    // Seems to be a false positive, we do want to take ownership of recipient cause it's used in async task
    #[expect(clippy::needless_pass_by_value)]
    pub fn init(
        recipient: AccountId32,
        task_tracker: &TaskTracker,
        mut seed: SecretString,
    ) -> Self {
        let (tx, mut rx) = mpsc::channel(16);
        task_tracker.spawn("Signer", async move {
            // TODO: shutdown on failure
            let mut mnemonic = Mnemonic::parse(seed.expose_secret()).map_err(SignerError::from)?;
            seed.zeroize();

            while let Some(req) = rx.recv().await {
                match req {
                    SignerRequest::PublicKey(request) => {
                        let new_public_key = {
                            // For some reason Keypair doesn't implement Zeroize trait but it's inner secret does
                            // so we just let it go out of scope as soon as possible
                            let keypair =
                                Keypair::from_phrase(&mnemonic, None).map_err(SignerError::from)?;
                            let new_pair = keypair.derive([
                                // api spec says use "2" for communication, let's use it here too
                                DeriveJunction::hard(to_base58_string(recipient.0, 2)),
                                DeriveJunction::hard(request.id.clone()),
                            ]);

                            Ok(to_base58_string(new_pair.public_key().0, request.ss58))
                        };

                        let _unused = request.res.send(new_public_key);
                    }
                    SignerRequest::Shutdown(res) => {
                        mnemonic.zeroize();
                        let _ = res.send(());
                        break;
                    }
                }
            }

            Ok("Signer module cleared and is shutting down!")
        });

        Self { tx }
    }

    pub async fn public(&self, id: String, ss58: u16) -> Result<String, SignerError> {
        let (res, rx) = oneshot::channel();
        self.tx
            .send(SignerRequest::PublicKey(PublicKeyRequest { id, ss58, res }))
            .await
            .map_err(|_| SignerError::SignerDown)?;
        rx.await.map_err(|_| SignerError::SignerDown)?
    }

    pub async fn shutdown(&self) {
        let (tx, _rx) = oneshot::channel();
        let _unused = self.tx.send(SignerRequest::Shutdown(tx)).await;
        // let _ = rx.await;
    }

    /// Clone wrapper in case we need to make it more complex later
    pub fn interface(&self) -> Self {
        Signer {
            tx: self.tx.clone(),
        }
    }
}

/// Messages sent to signer; signer never initiates anything on its own.
enum SignerRequest {
    /// Generate public key for order
    PublicKey(PublicKeyRequest),

    /// Safe termination
    Shutdown(oneshot::Sender<()>),
}

/// Information required to generate public invoice address, with callback
struct PublicKeyRequest {
    id: String,
    ss58: u16,
    res: oneshot::Sender<Result<String, SignerError>>,
}

use std::str::FromStr;

use chrono::Utc;
use futures::stream::{
    FuturesUnordered,
    StreamExt,
};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use tokio::time::{
    Duration,
    interval,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::chain_client::{
    AssetHubChainConfig,
    AssetHubClient,
    BlockChainClient,
    ChainConfig,
    GeneralChainTransfer,
    GeneralTransactionId,
    KeyringClient,
    SignedTransaction,
    SignedTransactionUtils,
    TransactionError,
};
use crate::dao::{
    DAO,
    DaoError,
};
use crate::types::{
    OutgoingTransaction,
    Payout,
    PayoutStatus,
    RetryMeta,
    Transaction,
    TransactionOrigin,
    TransactionOriginVariant,
    TransferInfo,
};

const MAX_CONCURRENT_TRANSFERS: u32 = 10;
const POLLING_INTERVAL_MILLIS: u64 = 100;

#[derive(Debug)]
pub struct ChainPayoutRequest<T: ChainConfig> {
    pub id: Uuid,
    pub invoice_id: Uuid,
    pub chain: String,
    pub asset_id: T::AssetId,
    pub source_address: T::AccountId,
    pub destination_address: T::AccountId,
    pub amount: Decimal,
    pub retry_meta: RetryMeta,
}

impl<T: ChainConfig> ChainPayoutRequest<T> {
    pub fn new(
        id: Uuid,
        invoice_id: Uuid,
        transfer_info: TransferInfo,
        retry_meta: RetryMeta,
    ) -> Result<Self, ()> {
        Ok(Self {
            id,
            invoice_id,
            chain: transfer_info.chain,
            asset_id: T::AssetId::from_str(&transfer_info.asset_id).map_err(|_| ())?,
            source_address: T::AccountId::from_str(&transfer_info.source_address)
                .map_err(|_| ())?,
            destination_address: T::AccountId::from_str(&transfer_info.destination_address)
                .map_err(|_| ())?,
            amount: transfer_info.amount,
            retry_meta,
        })
    }
}

#[derive(Debug)]
pub enum ChainPayoutRequestTyped {
    AssetHub(ChainPayoutRequest<AssetHubChainConfig>),
}

impl TryFrom<Payout> for ChainPayoutRequestTyped {
    type Error = ();

    fn try_from(value: Payout) -> Result<Self, Self::Error> {
        let request = match value.transfer_info.chain.as_ref() {
            "statemint" => ChainPayoutRequestTyped::AssetHub(ChainPayoutRequest::new(
                value.id,
                value.invoice_id,
                value.transfer_info,
                value.retry_meta,
            )?),
            _ => return Err(()),
        };

        Ok(request)
    }
}

#[derive(Debug)]
struct TransactionExecutionError {
    // Can be fully empty if transaction ID is not available
    transaction_id: GeneralTransactionId,
    retry_meta: RetryMeta,
    is_retriable: bool,
}

#[derive(Debug)]
struct TransactionExecutionData {
    transaction_id: Uuid,
    invoice_id: Uuid,
    origin: TransactionOrigin,
    result: Result<GeneralChainTransfer, TransactionExecutionError>,
}

pub struct TransfersExecutor {
    asset_hub_client: AssetHubClient,
    dao: DAO,
    keyring_client: KeyringClient,
}

type BoxedTransferFuture = std::pin::Pin<Box<dyn Future<Output = TransactionExecutionData> + Send>>;

async fn send_transfer_request<T: ChainConfig, C: BlockChainClient<T>>(
    client: C,
    signed_transaction: SignedTransaction<T>,
    request: ChainPayoutRequest<T>,
    transaction: Transaction,
) -> TransactionExecutionData {
    let response = client
        .submit_and_watch_transaction(signed_transaction)
        .await;

    let mut meta = request.retry_meta;

    let result = match response {
        Ok(transfer) => Ok(transfer.into()),
        Err(TransactionError::SubmissionStatusUnknown) => {
            // TODO: rework errors
            meta.increment_retry(String::new());

            Err(TransactionExecutionError {
                transaction_id: GeneralTransactionId::empty(),
                retry_meta: meta,
                is_retriable: true,
            })
        },
        Err(TransactionError::ExecutionFailed {
            transaction_id,
            error_code,
        }) => {
            meta.increment_retry(error_code);

            Err(TransactionExecutionError {
                transaction_id: transaction_id.into(),
                retry_meta: meta,
                is_retriable: false,
            })
        },
        Err(TransactionError::TransactionInfoFetchFailed {
            transaction_id,
        }) => {
            meta.increment_retry(String::new());

            Err(TransactionExecutionError {
                transaction_id: transaction_id.into(),
                retry_meta: meta,
                is_retriable: true,
            })
        },
        Err(TransactionError::InsufficientBalance {
            transaction_id,
        }) => {
            meta.increment_retry(String::new());

            Err(TransactionExecutionError {
                transaction_id: transaction_id.into(),
                retry_meta: meta,
                is_retriable: false,
            })
        },
        Err(TransactionError::UnknownAsset {
            transaction_id,
            asset_id,
        }) => {
            meta.increment_retry(asset_id.to_string());

            Err(TransactionExecutionError {
                transaction_id: transaction_id.into(),
                retry_meta: meta,
                is_retriable: false,
            })
        },
        Err(TransactionError::BuildFailed {
            ..
        }) => unreachable!(),
    };

    TransactionExecutionData {
        transaction_id: transaction.id,
        invoice_id: transaction.invoice_id,
        origin: transaction.origin,
        result,
    }
}

impl TransfersExecutor {
    async fn collect_pending_payout_requests(
        &self,
        limit: u32,
    ) -> Result<Vec<ChainPayoutRequestTyped>, DaoError> {
        let payout_requests = self
            .dao
            .get_pending_payouts(limit)
            .await?
            .into_iter()
            // TODO: add error handling and logging here
            .map(TryFrom::try_from)
            .filter_map(Result::ok)
            .collect::<Vec<ChainPayoutRequestTyped>>();

        Ok(payout_requests)
    }

    async fn build_and_sign_transfer<T: ChainConfig, C: BlockChainClient<T>>(
        &self,
        client: &C,
        request: &ChainPayoutRequest<T>,
    ) -> Result<SignedTransaction<T>, ()> {
        let transaction = client
            .build_transfer_all(
                &request.source_address,
                &request.destination_address,
                &request.asset_id,
            )
            .await
            .map_err(|_| ())?;

        let signed_transaction = client
            .sign_transaction(
                transaction,
                vec![request.invoice_id.to_string()],
                &self.keyring_client,
            )
            .await
            .map_err(|_| ())?;

        Ok(signed_transaction)
    }

    async fn store_built_transfer<T: ChainConfig>(
        &self,
        request: &ChainPayoutRequest<T>,
        signed_transaction: &SignedTransaction<T>,
    ) -> Result<Transaction, DaoError> {
        let data = OutgoingTransaction {
            id: Uuid::new_v4(),
            invoice_id: request.invoice_id,
            transfer_info: TransferInfo {
                asset_id: request.asset_id.to_string(),
                chain: request.chain.clone(),
                amount: request.amount,
                source_address: request.source_address.to_string(),
                destination_address: request.destination_address.to_string(),
            },
            tx_hash: signed_transaction.hash(),
            transaction_bytes: signed_transaction.to_hex_string(),
            origin: TransactionOrigin::payout(request.id),
        };

        self.dao
            .create_transaction(data.into())
            .await
    }

    async fn prepare_transfer<T: ChainConfig + 'static, C: BlockChainClient<T> + 'static>(
        &self,
        client: C,
        request: ChainPayoutRequest<T>,
    ) -> Result<BoxedTransferFuture, ()> {
        let signed_transaction = self
            .build_and_sign_transfer(&client, &request)
            .await?;
        let transaction = self
            .store_built_transfer(&request, &signed_transaction)
            .await
            .map_err(|_| ())?;

        let fut = Box::pin(send_transfer_request(
            client,
            signed_transaction,
            request,
            transaction,
        ));

        Ok(fut)
    }

    async fn schedule_transfers(
        &self,
        futures_set: &mut FuturesUnordered<BoxedTransferFuture>,
    ) -> Result<(), DaoError> {
        // Will be 0 if we reached the limit or overflowed (but it's not really expected)
        let limit = MAX_CONCURRENT_TRANSFERS
            .saturating_sub(
                futures_set
                    .len()
                    .to_u32()
                    .unwrap_or(u32::MAX)
            );

        if limit == 0 {
            return Ok(())
        }

        let payout_requests = self
            .collect_pending_payout_requests(limit)
            .await?;

        for request in payout_requests {
            tracing::info!(?request, "Prepare transfer for request");
            match request {
                ChainPayoutRequestTyped::AssetHub(request) => {
                    let client = self.asset_hub_client.clone();
                    let prepared_transfer = self
                        .prepare_transfer(client, request)
                        .await
                        .unwrap();
                    futures_set.push(prepared_transfer);
                },
            }
        }

        Ok(())
    }

    async fn handle_transfer_result(
        &self,
        data: TransactionExecutionData,
    ) -> Result<(), DaoError> {
        // Update the transaction and origin entity based on the result
        let mut dao_transaction = self.dao.begin_transaction().await?;

        match data.result {
            Ok(transfer) => {
                let chain_transaction_id = transfer.general_transaction_id();

                self.dao
                    .update_transaction_successful(
                        &mut dao_transaction,
                        data.transaction_id,
                        chain_transaction_id,
                        // TODO: use transfer.timestamp
                        Utc::now(),
                    )
                    .await?;

                #[expect(clippy::single_match)]
                match data.origin.variant() {
                    TransactionOriginVariant::Payout(payout_id) => {
                        self.dao
                            .update_payout_status(
                                &mut dao_transaction,
                                payout_id,
                                PayoutStatus::Completed,
                            )
                            .await?;
                    },
                    // TODO: should be implemented later, not necessary for now
                    _ => {},
                }
            },
            Err(error) => {
                self.dao
                    .update_transaction_failed(
                        &mut dao_transaction,
                        data.transaction_id,
                        error.transaction_id,
                        error
                            .retry_meta
                            .failure_message
                            .clone()
                            .unwrap_or_default(),
                        // TODO: use transfer.timestamp
                        Utc::now(),
                    )
                    .await?;

                #[expect(clippy::single_match)]
                match data.origin.variant() {
                    TransactionOriginVariant::Payout(payout_id) => {
                        self.dao
                            .update_payout_retry(
                                &mut dao_transaction,
                                payout_id,
                                error.retry_meta,
                                error.is_retriable,
                            )
                            .await?;
                    },
                    _ => {},
                }
            },
        }

        dao_transaction.commit().await?;

        // TODO: make it in transaction
        self.dao
            .update_invoice_withdrawal_status(
                data.invoice_id,
                crate::legacy_types::WithdrawalStatus::Completed,
            )
            .await?;

        Ok(())
    }

    async fn perform(
        self,
        token: CancellationToken,
    ) {
        let mut interval = interval(Duration::from_millis(
            POLLING_INTERVAL_MILLIS,
        ));

        let mut shutdown_expected = false;
        let mut futures_set = FuturesUnordered::new();

        loop {
            tokio::select! {
                _ = interval.tick(), if !shutdown_expected => {
                    self.schedule_transfers(&mut futures_set).await.unwrap();
                },
                future_result = futures_set.next(), if !futures_set.is_empty() => {
                    if let Some(data) = future_result {
                        tracing::info!(?data, "Processing.. transfer result from future.");
                        let result = self.handle_transfer_result(data).await;

                        if let Err(error) = result {
                            tracing::error!(
                                error = %error,
                                "Error while storing processing result to database",
                            );
                        }
                    } else {
                        // TODO: log unexpected empty future result
                    }
                },
                () = token.cancelled() => {
                    tracing::info!("Transfers executor received shutdown signal, finishing ongoing transfers...");

                    shutdown_expected = true;

                    if futures_set.is_empty() {
                        tracing::info!("No ongoing transfers, shutting down transfers executor.");

                        break;
                    }
                }
            }
        }
    }

    pub fn new(
        asset_hub_client: AssetHubClient,
        dao: DAO,
        keyring_client: KeyringClient,
    ) -> Self {
        Self {
            asset_hub_client,
            dao,
            keyring_client,
        }
    }

    pub fn ignite(
        self,
        token: CancellationToken,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            self.perform(token).await;
        })
    }
}

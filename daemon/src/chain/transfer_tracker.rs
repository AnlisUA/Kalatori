use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use futures::StreamExt;
use rust_decimal::Decimal;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::chain_client::{
    BlockChainClient,
    ChainConfig,
    ChainTransfer,
    GeneralChainTransfer,
    SubscriptionError,
    TransfersStream,
};
use crate::configs::PaymentsConfig;
use crate::dao::{
    DaoInterface,
    DaoTransactionInterface,
};
use crate::types::{
    ChainType,
    IncomingTransaction,
    Invoice,
    InvoiceStatus,
    InvoiceWithIncomingAmount,
    Payout,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvoiceRegistryRecord {
    // TODO: store something smaller? Most fields are not needed for matching
    pub invoice: Invoice,
    pub filled_amount: Decimal,
}

impl InvoiceRegistryRecord {
    pub fn new(
        invoice: Invoice,
        filled_amount: Decimal,
    ) -> Self {
        InvoiceRegistryRecord {
            invoice,
            filled_amount,
        }
    }
}

impl From<InvoiceWithIncomingAmount> for InvoiceRegistryRecord {
    fn from(value: InvoiceWithIncomingAmount) -> Self {
        InvoiceRegistryRecord {
            invoice: value.invoice,
            filled_amount: value.incoming_amount,
        }
    }
}

#[derive(Clone)]
pub struct InvoiceRegistry {
    invoices: Arc<RwLock<HashMap<Uuid, InvoiceRegistryRecord>>>,
}

impl InvoiceRegistry {
    pub fn new() -> Self {
        InvoiceRegistry {
            invoices: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn add_invoice(
        &self,
        record: InvoiceRegistryRecord,
    ) {
        let mut invoices = self.invoices.write().await;
        invoices.insert(record.invoice.id, record);
    }

    pub async fn add_invoices(
        &self,
        records: Vec<InvoiceRegistryRecord>,
    ) {
        let mut invoices_map = self.invoices.write().await;

        for record in records {
            invoices_map.insert(record.invoice.id, record);
        }
    }

    pub async fn remove_invoice(
        &self,
        invoice_id: &Uuid,
    ) {
        let mut invoices = self.invoices.write().await;
        invoices.remove(invoice_id);
    }

    pub async fn remove_invoices(
        &self,
        invoices_ids: &[Uuid],
    ) {
        let mut invoices = self.invoices.write().await;

        for invoice_id in invoices_ids {
            invoices.remove(invoice_id);
        }
    }

    pub async fn get_invoice(
        &self,
        invoice_id: &Uuid,
    ) -> Option<InvoiceRegistryRecord> {
        let invoices = self.invoices.read().await;
        invoices.get(invoice_id).cloned()
    }

    pub async fn find_invoice_by_address(
        &self,
        address: &str,
        chain: ChainType,
        asset_id: &str,
    ) -> Option<InvoiceRegistryRecord> {
        let invoices = self.invoices.read().await;

        invoices
            .values()
            .find(|inv| {
                inv.invoice.chain == chain
                    && inv.invoice.payment_address == address
                    && inv.invoice.asset_id == asset_id
            })
            .cloned()
    }

    pub async fn update_filled_amount(
        &self,
        invoice_id: &Uuid,
        new_filled_amount: Decimal,
    ) {
        let mut invoices = self.invoices.write().await;

        if let Some(record) = invoices.get_mut(invoice_id) {
            record.filled_amount = new_filled_amount;
        }
    }

    #[cfg_attr(not(test), expect(dead_code))]
    pub async fn invoices_count(&self) -> usize {
        let invoices = self.invoices.read().await;
        invoices.len()
    }
}

#[derive(Debug, thiserror::Error)]
enum ChainTransferTrackerError {
    #[error("DAO transaction error")]
    DaoTransactionError,
}

pub struct TransfersTracker<
    T: ChainConfig,
    C: BlockChainClient<T> + 'static,
    D: DaoInterface + 'static,
> {
    client: C,
    dao: D,
    registry: InvoiceRegistry,
    config: PaymentsConfig,
    phantom: std::marker::PhantomData<T>,
}

impl<T: ChainConfig, C: BlockChainClient<T> + 'static, D: DaoInterface + 'static>
    TransfersTracker<T, C, D>
{
    pub fn new(
        client: C,
        dao: D,
        registry: InvoiceRegistry,
        config: PaymentsConfig,
    ) -> Self {
        TransfersTracker {
            client,
            dao,
            registry,
            config,
            phantom: std::marker::PhantomData,
        }
    }

    async fn get_or_create_subscription(
        &self,
        subscription: Option<TransfersStream<T>>,
        asset_ids: &[T::AssetId],
    ) -> Option<TransfersStream<T>> {
        if subscription.is_some() {
            return subscription;
        }

        self.client
            .subscribe_transfers(asset_ids)
            .await
            .inspect_err(|e| {
                tracing::error!(
                    error.category = "transfer_tracker",
                    error.operation = "get_or_create_subscription",
                    error.source = ?e,
                    "Error subscribing to transfer events"
                );
            })
            .ok()
    }

    async fn store_transaction(
        &self,
        transaction: IncomingTransaction,
        invoice_status: InvoiceStatus,
    ) -> Result<(), ChainTransferTrackerError> {
        let dao_transaction = self
            .dao
            .begin_transaction()
            .await
            .map_err(|_e| ChainTransferTrackerError::DaoTransactionError)?;

        let invoice_id = transaction.invoice_id;

        dao_transaction
            .create_transaction(transaction.into())
            .await
            .map_err(|_e| ChainTransferTrackerError::DaoTransactionError)?;

        let invoice = dao_transaction
            .update_invoice_status(invoice_id, invoice_status)
            .await
            .map_err(|_e| ChainTransferTrackerError::DaoTransactionError)?;

        if invoice_status == InvoiceStatus::Paid {
            let payout = Payout::from_invoice(invoice, self.config.recipient.clone());

            dao_transaction
                .create_payout(payout)
                .await
                .map_err(|_e| ChainTransferTrackerError::DaoTransactionError)?;
        }

        // TODO: handle overpayments

        dao_transaction
            .commit()
            .await
            .map_err(|_e| ChainTransferTrackerError::DaoTransactionError)?;

        Ok(())
    }

    #[expect(clippy::arithmetic_side_effects)]
    async fn process_transfer(
        &self,
        transfer: GeneralChainTransfer,
    ) {
        // TODO: that's a temporary workaround, fix the types properly
        let recipient = super::utils::to_base58_string(
            subxt::utils::AccountId32::from_str(&transfer.recipient)
                .unwrap()
                .0,
            0,
        );

        if let Some(InvoiceRegistryRecord {
            invoice,
            mut filled_amount,
        }) = self
            .registry
            .find_invoice_by_address(
                &recipient,
                transfer.chain,
                &transfer.asset_id,
            )
            .await
        {
            tracing::info!(
                invoice_id = %invoice.id,
                chain = %transfer.chain,
                asset_id = %transfer.asset_id,
                sender = %transfer.sender,
                recipient = %recipient,
                amount = %transfer.amount,
                "Processing incoming transfer for invoice"
            );

            let transaction = IncomingTransaction::from_chain_transfer(invoice.id, transfer);
            filled_amount += transaction.transfer_info.amount;

            // TODO: handle overpayments
            let updated_status = if filled_amount >= invoice.amount {
                InvoiceStatus::Paid
            } else {
                InvoiceStatus::PartiallyPaid
            };

            match self
                .store_transaction(transaction, updated_status)
                .await
            {
                Ok(()) if updated_status == InvoiceStatus::Paid => {
                    tracing::info!(
                        invoice_id = %invoice.id,
                        "Invoice has been fully paid, removing from registry"
                    );

                    self.registry
                        .remove_invoice(&invoice.id)
                        .await;
                },
                Ok(()) if updated_status == InvoiceStatus::PartiallyPaid => {
                    tracing::info!(
                        invoice_id = %invoice.id,
                        filled_amount = %filled_amount,
                        "Invoice has been partially paid, updating filled amount in registry"
                    );

                    self.registry
                        .update_filled_amount(&invoice.id, filled_amount)
                        .await;
                },
                Ok(()) => {
                    // This should not happen
                    tracing::error!(
                        invoice_id = %invoice.id,
                        error.category = "transfer_tracker",
                        error.operation = "process_transfer",
                        "Unexpected invoice status after storing transaction"
                    );

                    self.registry
                        .update_filled_amount(&invoice.id, filled_amount)
                        .await;
                },
                // TODO: handle different errors separately. Behavior may differ based on the error
                Err(e) => {
                    tracing::error!(
                        invoice_id = %invoice.id,
                        error.category = "transfer_tracker",
                        error.operation = "process_transfer",
                        error.source = ?e,
                        "Error storing transaction for invoice"
                    );

                    self.registry
                        .update_filled_amount(&invoice.id, filled_amount)
                        .await;
                },
            }
        }
    }

    async fn handle_subscription_event(
        &self,
        event: Option<Result<Vec<ChainTransfer<T>>, SubscriptionError>>,
    ) -> Result<(), SubscriptionError> {
        match event {
            Some(Ok(transfers)) => {
                for transfer in transfers {
                    self.process_transfer(transfer.into())
                        .await;
                }

                Ok(())
            },
            Some(Err(e)) => {
                tracing::error!(
                    error.category = "transfer_tracker",
                    error.operation = "handle_subscription_event",
                    error.source = ?e,
                    "Error receiving transfer event"
                );
                Err(e)
            },
            None => {
                tracing::warn!("Transfer event subscription ended");
                Err(SubscriptionError::StreamClosed)
            },
        }
    }

    async fn perform(
        mut self,
        assets: Vec<T::AssetId>,
        token: CancellationToken,
    ) {
        tracing::info!(
            "Starting transfers tracker for chain for {}",
            self.client.chain_name()
        );

        let mut subscription = None;

        loop {
            subscription = self
                .get_or_create_subscription(subscription, &assets)
                .await;

            let Some(poll_subscription) = &mut subscription else {
                // If we couldn't create a subscription, try to recreate the client with another
                // RPC endpoint
                match self.client.recreate().await {
                    Ok(new_client) => {
                        self.client = new_client;

                        tracing::warn!(
                            "Recreated blockchain client for {} with new RPC endpoint",
                            self.client.chain_name()
                        );
                    },
                    Err(e) => {
                        tracing::error!(
                            error.category = "transfer_tracker",
                            error.operation = "perform",
                            error.source = ?e,
                            "Error recreating blockchain client"
                        );
                    },
                }

                continue;
            };

            tokio::select! {
                subscription_event = poll_subscription.next() => {
                    if self.handle_subscription_event(subscription_event).await.is_err() {
                        subscription = None;
                    }
                },
                () = token.cancelled() => {
                    tracing::info!(
                        "Transfers tracker received cancellation signal, shutting down"
                    );
                    break;
                },
            }
        }
    }

    pub fn ignite(
        self,
        assets: Vec<T::AssetId>,
        token: CancellationToken,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            self.perform(assets, token).await;
        })
    }
}

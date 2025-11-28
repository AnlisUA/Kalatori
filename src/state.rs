use crate::chain_client::KeyringClient;
use crate::error::ForceWithdrawalError;
use crate::{
    chain::{ChainManager, utils::to_base58_string},
    error::{DaoError, Error, OrderError},
    legacy_types::{
        Amount, ConfigWoChains, CurrencyProperties, FinalizedTx, Health, OrderInfo, OrderQuery,
        OrderResponse, OrderStatus, RpcInfo, ServerHealth, ServerInfo, ServerStatus,
        TransactionInfo, TxStatus,
    },
    signer::Signer,
    types::{
        Invoice, InvoiceCart, InvoiceStatus, Transaction, TransactionStatus, UpdateInvoiceData,
    },
    utils::task_tracker::TaskTracker,
};
use std::collections::HashMap;
use subxt::utils::AccountId32;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Struct to store state of daemon. If something requires cooperation of more than one component,
/// it should go through here.
#[derive(Clone, Debug)]
pub struct State {
    tx: tokio::sync::mpsc::Sender<StateAccessRequest>,
}

impl State {
    #[expect(clippy::too_many_lines)]
    pub fn initialise(
        signer: KeyringClient,
        ConfigWoChains {
            recipient,
            remark,
            account_lifetime,
        }: ConfigWoChains,
        dao: crate::dao::DAO,
        chain_manager_receiver: oneshot::Receiver<ChainManager>,
        instance_id: String,
        task_tracker: TaskTracker,
        shutdown_notification: CancellationToken,
    ) -> Self {
        /*
            currencies: HashMap<String, CurrencyProperties>,
            recipient: AccountId,
            pair: Pair,
            depth: Option<Timestamp>,
            account_lifetime: Timestamp,
            debug: bool,
            remark: String,
            invoices: RwLock<HashMap<String, Invoicee>>,
            rpc: String,
        */
        let (tx, mut rx) = tokio::sync::mpsc::channel(1024);

        let server_info = ServerInfo {
            // TODO
            version: env!("CARGO_PKG_VERSION").to_string(),
            kalatori_remark: remark,
            instance_id,
        };

        // Remember to always spawn async here or things might deadlock
        task_tracker.clone().spawn("State Handler", async move {
            let chain_manager = chain_manager_receiver.await.map_err(|_| Error::Fatal)?;

            // Clone chain_manager for restoration before moving into StateData
            let chain_manager_for_restoration = chain_manager.clone();

            let currencies = HashMap::new();
            let mut state = StateData {
                currencies,
                recipient: recipient.clone(),
                server_info,
                dao,
                chain_manager,
                signer,
                account_lifetime,
                invoices_restored: false,
            };

            loop {
                tokio::select! {
                    biased;
                    request_option = rx.recv() => {
                        let Some(state_request) = request_option else {
                            break;
                        };

                        match state_request {
                            StateAccessRequest::ConnectChain(assets) => {
                                // it MUST be asserted in chain tracker that assets are those and only
                                // those that user requested
                                state.update_currencies(assets);

                                // Restore active invoices now that currencies are populated
                                state.restore_active_invoices(
                                    chain_manager_for_restoration.clone(),
                                    &task_tracker
                                ).await;
                            }
                            StateAccessRequest::GetInvoiceStatus(request) => {
                                request
                                    .res
                                    .send(state.get_invoice_status(request.order).await)
                                    .map_err(|_| Error::Fatal)?;
                            }
                            StateAccessRequest::CreateInvoice(request) => {
                                request
                                    .res
                                    .send(state.create_invoice(request.order_query).await)
                                    .map_err(|_| Error::Fatal)?;
                            }
                            StateAccessRequest::IsCurrencySupported { currency, res } => {
                                let supported = state.currencies.contains_key(&currency);
                                res.send(supported).map_err(|_| Error::Fatal)?;
                            }
                            StateAccessRequest::ServerStatus(res) => {
                                let server_status = ServerStatus {
                                    server_info: state.server_info.clone(),
                                    supported_currencies: state.currencies.clone(),
                                };
                                res.send(server_status).map_err(|_| Error::Fatal)?;
                            }
                            StateAccessRequest::ServerHealth(res) => {
                                let connected_rpcs = state.chain_manager.get_connected_rpcs().await?;
                                let server_health = ServerHealth {
                                    server_info: state.server_info.clone(),
                                    connected_rpcs: connected_rpcs.clone(),
                                    status: Self::overall_health(&connected_rpcs),
                                };
                                res.send(server_health).map_err(|_| Error::Fatal)?;
                            }
                            StateAccessRequest::OrderPaid(invoice_id) => {
                                // Look up invoice to get order_id for legacy database
                                match state.dao.get_invoice_by_id(invoice_id).await {
                                    Ok(Some(invoice)) => {
                                        // Only perform actions if the record is saved in ledger
                                        let marked = state.dao.update_invoice_status(invoice_id, InvoiceStatus::Paid).await;
                                        // let marked = state.db.mark_paid(invoice.order_id.clone()).await;

                                        match marked {
                                            Ok(order) => {
                                                if !order.callback.is_empty() {
                                                    let callback = order.callback.clone();
                                                    tokio::spawn(async move {
                                                        tracing::info!("Sending callback to: {}", callback);

                                                        // fire and forget
                                                        if let Err(e) = reqwest::Client::new().get(&callback).send().await {
                                                            tracing::error!("Failed to send callback to {}: {:?}", callback, e);
                                                        }
                                                    });
                                                }

                                                let currency = state.get_currency_info(&invoice.chain, invoice.asset_id)?;
                                                let order_info = state.invoice_to_order_info(&order, &currency);
                                                drop(state.chain_manager.reap(invoice_id, order_info, state.recipient.clone()).await);
                                            }
                                            Err(e) => {
                                                tracing::error!(
                                                    "Order was paid but this could not be recorded! {e:?}"
                                                );
                                            }
                                        }
                                    }
                                    Ok(None) => {
                                        tracing::error!("Invoice {invoice_id} not found in database");
                                    }
                                    Err(e) => {
                                        tracing::error!("Failed to look up invoice {invoice_id}: {e:?}");
                                    }
                                }
                            }
                            StateAccessRequest::RecordTransactionV2 { invoice_id, transaction } => {
                                let record = state.dao.create_transaction(transaction).await;

                                if let Err(e) = record {
                                    tracing::error!(
                                        "Found a transaction related to invoice {invoice_id}, but this could not be recorded! {e:?}"
                                    );
                                }
                            }
                            StateAccessRequest::UpdateTransactionV2 { transaction } => {
                                let update = state.dao.update_transaction(transaction.clone()).await;

                                if let Err(e) = update {
                                    tracing::error!(
                                        "Failed to update transaction {}: {e:?}",
                                        transaction.id
                                    );
                                }
                            }
                            StateAccessRequest::OrderWithdrawn(id) => {
                                match state.dao.update_invoice_withdrawal_status(id, crate::legacy_types::WithdrawalStatus::Completed).await {
                                    Ok(_order) => {
                                        tracing::info!("Order {id} successfully marked as withdrawn");
                                    }
                                    Err(e) => {
                                        tracing::error!(
                                            "Order was withdrawn but this could not be recorded! {e:?}"
                                        );
                                    }
                                }
                            }
                            StateAccessRequest::ForceWithdrawal(order_id) => {
                                // Look up invoice_id from order_id
                                match state.dao.get_invoice_by_order_id(&order_id).await {
                                    Ok(Some(invoice)) => {
                                        let currency_info = state.get_currency_info(&invoice.chain, invoice.asset_id);
                                        let order = currency_info.map(|info| state.invoice_to_order_info(&invoice, &info));
                                        // let order = state.invoice_to_order_info(&invoice, &currency_info);

                                        match order {
                                            Ok(order_info) => {
                                                let result = state.chain_manager.reap(invoice.id, order_info, state.recipient.clone()).await;

                                                match result {
                                                    Ok(()) => {
                                                        // let marked = state.db.mark_forced(order_id.clone()).await;
                                                        let marked = state.dao.update_invoice_withdrawal_status(invoice.id, crate::legacy_types::WithdrawalStatus::Forced).await;

                                                        match marked {
                                                            Ok(_) => {
                                                                tracing::info!("Order {order_id} successfully marked as force withdrawn");
                                                            }
                                                            Err(e) => {
                                                                tracing::error!("Failed to mark order {order_id} as forced: {e:?}");
                                                            }
                                                        }
                                                    }
                                                    Err(e) => {
                                                        tracing::error!("Failed to initiate forced payout for order {order_id}: {e:?}");
                                                    }
                                                }
                                            },
                                            Err(e) => {
                                                tracing::error!("Error reading order {order_id} from database: {e:?}");
                                            }
                                        }
                                    }
                                    Ok(None) => {
                                        tracing::error!("Invoice for order_id {order_id} not found in new database");
                                    }
                                    Err(e) => {
                                        tracing::error!("Failed to look up invoice for order_id {order_id}: {e:?}");
                                    }
                                }
                            }
                            StateAccessRequest::IsOrderPaid(invoice_id, res) => {
                                // Look up invoice to get order_id for legacy database
                                match state.dao.get_invoice_by_id(invoice_id).await {
                                    Ok(Some(invoice)) => {
                                        let is_marked_paid = invoice.status == InvoiceStatus::Paid;

                                        res.send(is_marked_paid).map_err(|_| Error::Fatal)?;
                                    }
                                    Ok(None) => {
                                        tracing::error!("Invoice {invoice_id} not found in database");
                                        // Send false as invoice not found means not paid
                                        res.send(false).map_err(|_| Error::Fatal)?;
                                    }
                                    Err(e) => {
                                        tracing::error!("Failed to look up invoice {invoice_id}: {e:?}");
                                    }
                                }
                            }
                        }
                    }
                    // Orchestrate shutdown from here
                    () = shutdown_notification.cancelled() => {
                        // Web server shuts down on its own; it does not matter what it sends now.

                        // First shut down active actions for external world.
                        state.chain_manager.shutdown().await;

                        // And shut down finally
                        break;
                    }
                }
            }

            Ok("State handler is shutting down")
        });

        Self { tx }
    }
    fn overall_health(connected_rpcs: &[RpcInfo]) -> Health {
        if connected_rpcs.iter().all(|rpc| rpc.status == Health::Ok) {
            Health::Ok
        } else if connected_rpcs.iter().any(|rpc| rpc.status == Health::Ok) {
            Health::Degraded
        } else {
            Health::Critical
        }
    }

    pub async fn connect_chain(&self, assets: HashMap<String, CurrencyProperties>) {
        self.tx
            .send(StateAccessRequest::ConnectChain(assets))
            .await
            .unwrap_or_else(|e| {
                tracing::error!("Failed to send ConnectChain request: {}", e);
            });
    }

    pub async fn order_status(&self, order: &str) -> Result<OrderResponse, Error> {
        let (res, rx) = oneshot::channel();
        self.tx
            .send(StateAccessRequest::GetInvoiceStatus(GetInvoiceStatus {
                order: order.to_string(),
                res,
            }))
            .await
            .map_err(|_| Error::Fatal)?;
        rx.await.map_err(|_| Error::Fatal)?
    }

    pub async fn server_status(&self) -> Result<ServerStatus, Error> {
        let (res, rx) = oneshot::channel();
        self.tx
            .send(StateAccessRequest::ServerStatus(res))
            .await
            .map_err(|_| Error::Fatal)?;
        rx.await.map_err(|_| Error::Fatal)
    }

    pub async fn server_health(&self) -> Result<ServerHealth, Error> {
        let (res, rx) = oneshot::channel();
        self.tx
            .send(StateAccessRequest::ServerHealth(res))
            .await
            .map_err(|_| Error::Fatal)?;
        rx.await.map_err(|_| Error::Fatal)
    }

    pub async fn create_order(&self, order_query: OrderQuery) -> Result<OrderResponse, Error> {
        let (res, rx) = oneshot::channel();
        /*
                Invoicee {
                        callback: callback.clone(),
                        amount: Balance::parse(amount, 6),
                        paid: false,
                        paym_acc: pay_acc.clone(),
                    },
        */
        self.tx
            .send(StateAccessRequest::CreateInvoice(CreateInvoice {
                order_query,
                res,
            }))
            .await
            .map_err(|_| Error::Fatal)?;
        rx.await.map_err(|_| Error::Fatal)?
    }

    pub async fn is_currency_supported(&self, currency: &str) -> Result<bool, Error> {
        let (res, rx) = oneshot::channel();
        self.tx
            .send(StateAccessRequest::IsCurrencySupported {
                currency: currency.to_string(),
                res,
            })
            .await
            .map_err(|_| Error::Fatal)?;
        rx.await.map_err(|_| Error::Fatal)
    }

    pub async fn is_order_paid(&self, invoice_id: Uuid) -> Result<bool, Error> {
        let (res, rx) = oneshot::channel();
        self.tx
            .send(StateAccessRequest::IsOrderPaid(invoice_id, res))
            .await
            .map_err(|_| Error::Fatal)?;
        rx.await.map_err(|_| Error::Fatal)
    }

    pub async fn order_paid(&self, invoice_id: Uuid) {
        if self
            .tx
            .send(StateAccessRequest::OrderPaid(invoice_id))
            .await
            .is_err()
        {
            tracing::warn!("Data race on shutdown; please restart the daemon for cleaning up");
        }
    }

    pub async fn order_withdrawn(&self, order: Uuid) {
        if self
            .tx
            .send(StateAccessRequest::OrderWithdrawn(order))
            .await
            .is_err()
        {
            tracing::warn!("Data race on shutdown; please restart the daemon for cleaning up");
        }
    }

    pub async fn force_withdrawal(
        &self,
        order: String,
    ) -> Result<OrderResponse, ForceWithdrawalError> {
        self.tx
            .send(StateAccessRequest::ForceWithdrawal(order.clone()))
            .await
            .map_err(|_| ForceWithdrawalError::InvalidParameter(order.clone()))?;

        match self.order_status(&order).await {
            Ok(order_status) => Ok(order_status),
            Err(_) => Ok(OrderResponse::NotFound),
        }
    }
    pub fn interface(&self) -> Self {
        State {
            tx: self.tx.clone(),
        }
    }

    pub async fn record_transaction_v2(
        &self,
        invoice_id: Uuid,
        transaction: Transaction,
    ) -> Result<(), Error> {
        self.tx
            .send(StateAccessRequest::RecordTransactionV2 {
                invoice_id,
                transaction,
            })
            .await
            .map_err(|_| Error::Fatal)
    }

    pub async fn update_transaction_v2(&self, transaction: Transaction) -> Result<(), Error> {
        self.tx
            .send(StateAccessRequest::UpdateTransactionV2 { transaction })
            .await
            .map_err(|_| Error::Fatal)
    }
}

enum StateAccessRequest {
    ConnectChain(HashMap<String, CurrencyProperties>),
    GetInvoiceStatus(GetInvoiceStatus),
    CreateInvoice(CreateInvoice),
    IsCurrencySupported {
        currency: String,
        res: oneshot::Sender<bool>,
    },
    ServerStatus(oneshot::Sender<ServerStatus>),
    ServerHealth(oneshot::Sender<ServerHealth>),
    OrderPaid(Uuid),
    IsOrderPaid(Uuid, oneshot::Sender<bool>),
    RecordTransactionV2 {
        invoice_id: Uuid,
        transaction: Transaction,
    },
    UpdateTransactionV2 {
        transaction: Transaction,
    },
    OrderWithdrawn(Uuid),
    ForceWithdrawal(String),
}

struct GetInvoiceStatus {
    pub order: String,
    pub res: oneshot::Sender<Result<OrderResponse, Error>>,
}

struct CreateInvoice {
    pub order_query: OrderQuery,
    pub res: oneshot::Sender<Result<OrderResponse, Error>>,
}

struct StateData {
    currencies: HashMap<String, CurrencyProperties>,
    recipient: AccountId32,
    server_info: ServerInfo,
    dao: crate::dao::DAO,
    chain_manager: ChainManager,
    signer: KeyringClient,
    account_lifetime: crate::legacy_types::Timestamp,
    invoices_restored: bool,
}

impl StateData {
    fn update_currencies(&mut self, currencies: HashMap<String, CurrencyProperties>) {
        self.currencies.extend(currencies);
    }

    async fn restore_active_invoices(
        &mut self,
        chain_manager_wakeup: ChainManager,
        task_tracker: &TaskTracker,
    ) {
        // Only restore once
        if self.invoices_restored {
            tracing::debug!("Invoices already restored, skipping");
            return;
        }

        tracing::info!("Starting invoice restoration from database");

        // Fetch active invoices from the new SQLite database
        let active_invoices = match self.dao.get_active_invoices().await {
            Ok(invoices) => invoices,
            Err(e) => {
                tracing::error!("Failed to fetch active invoices from database: {e:?}");
                return;
            }
        };

        tracing::info!("Found {} active invoices to restore", active_invoices.len());

        // Pre-process invoices: convert to OrderInfo using state methods
        let mut invoices_to_restore = Vec::new();
        for invoice in active_invoices {
            match self.get_currency_info(&invoice.chain, invoice.asset_id) {
                Ok(currency) => {
                    let order_info = self.invoice_to_order_info(&invoice, &currency);
                    invoices_to_restore.push((invoice.id, invoice.order_id.clone(), order_info));
                }
                Err(e) => {
                    tracing::error!(
                        "Failed to get currency info for invoice {} (order: {}): {e:?}",
                        invoice.id,
                        invoice.order_id
                    );
                }
            }
        }

        let recipient_cloned = self.recipient.clone();
        task_tracker.spawn("Restore saved orders", async move {
            let mut restored_count: u32 = 0;
            let mut failed_count: u32 = 0;

            for (invoice_id, order_id, order_info) in invoices_to_restore {
                match chain_manager_wakeup
                    .add_invoice(invoice_id, order_info, recipient_cloned.clone())
                    .await
                {
                    Ok(()) => {
                        tracing::info!("Restored invoice {} (order: {})", invoice_id, order_id);
                        restored_count = restored_count.saturating_add(1);
                    }
                    Err(e) => {
                        tracing::error!(
                            "Failed to restore invoice {} (order: {}): {e:?}",
                            invoice_id,
                            order_id
                        );
                        failed_count = failed_count.saturating_add(1);
                    }
                }
            }

            tracing::info!(
                "Invoice restoration complete: {} restored, {} failed",
                restored_count,
                failed_count
            );

            Ok("All saved orders restored")
        });

        self.invoices_restored = true;
    }

    async fn get_invoice_status(&self, order: String) -> Result<OrderResponse, Error> {
        // Fetch invoice from DAO
        let Some(invoice) = self
            .dao
            .get_invoice_by_order_id(&order)
            .await
            .map_err(DaoError::Sqlx)?
        else {
            return Ok(OrderResponse::NotFound);
        };

        // Fetch transactions for this invoice
        let transactions = self
            .dao
            .get_invoice_transactions(invoice.id)
            .await
            .map_err(DaoError::Sqlx)?;

        // Convert transactions to legacy format
        let legacy_transactions: Vec<TransactionInfo> = transactions
            .iter()
            .map(|tx| self.transaction_to_transaction_info(tx))
            .collect::<Result<Vec<_>, _>>()?;

        // Get currency info for the invoice
        let currency = self.get_currency_info(&invoice.chain, invoice.asset_id)?;

        // Convert invoice to OrderInfo
        let mut order_info = self.invoice_to_order_info(&invoice, &currency);
        order_info.transactions = legacy_transactions;

        // Build response
        let message = String::new(); //TODO
        Ok(OrderResponse::FoundOrder(OrderStatus {
            order,
            message,
            recipient: to_base58_string(self.recipient.0, 2), // TODO maybe but spec says use "2"
            server_info: self.server_info.clone(),
            order_info,
            payment_page: String::new(),
            redirect_url: String::new(),
        }))
    }

    async fn create_invoice(&self, order_query: OrderQuery) -> Result<OrderResponse, Error> {
        const MAX_RETRIES: u8 = 3;

        let invoice_id = Uuid::new_v4();
        let order = order_query.order.clone();
        let currency_properties = self
            .currencies
            .get(&order_query.currency)
            .ok_or(OrderError::UnknownCurrency)?;
        let currency = currency_properties.info(order_query.currency.clone());

        let derivation_params = vec![
            to_base58_string(self.recipient.0, 2),
            order.clone(),
        ];

        let payment_account_id = self
            .signer
            .generate_asset_hub_address(derivation_params.into())
            .await?;

        let payment_account = to_base58_string(payment_account_id.0, currency.ss58);

        // Retry loop for optimistic locking conflicts
        for attempt in 0..MAX_RETRIES {
            // PHASE 1: Check if invoice exists
            match self
                .dao
                .get_invoice_by_order_id(&order)
                .await
                .map_err(DaoError::Sqlx)?
            {
                None => {
                    // PHASE 2a: Create new invoice
                    let mut invoice = Invoice::from_order_query(
                        order_query.clone(),
                        currency.clone(),
                        payment_account.clone(),
                        self.account_lifetime,
                    )
                    .map_err(DaoError::AmountConversion)?;

                    invoice.id = invoice_id;

                    self.dao
                        .create_invoice(invoice.clone())
                        .await
                        .map_err(DaoError::Sqlx)?;

                    // Convert Invoice back to OrderInfo for backward compatibility
                    let order_info = self.invoice_to_order_info(&invoice, &currency);

                    // Register with chain manager
                    self.chain_manager
                        .add_invoice(invoice.id, order_info.clone(), self.recipient.clone())
                        .await?;

                    return Ok(OrderResponse::NewOrder(self.order_status(
                        order,
                        order_info,
                        String::new(),
                    )));
                }
                Some(existing_invoice) => {
                    // PHASE 2b: Update or collision based on status
                    if existing_invoice.status == InvoiceStatus::Waiting {
                        // Try to update existing pending invoice
                        let update_data = UpdateInvoiceData {
                            id: existing_invoice.id,
                            amount: rust_decimal::Decimal::try_from(order_query.amount)
                                .map_err(|e| DaoError::AmountConversion(format!("{e}")))?,
                            cart: InvoiceCart::empty(),
                            valid_till: crate::types::calculate_valid_till(self.account_lifetime),
                            version: existing_invoice.version,
                        };

                        let updated_invoice = match self
                            .dao
                            .update_invoice_data(update_data)
                            .await
                        {
                            Ok(invoice) => invoice,
                            Err(sqlx::Error::RowNotFound) => {
                                // Version conflict - retry
                                if attempt < MAX_RETRIES.saturating_sub(1) {
                                    tracing::warn!(
                                        "Version conflict updating invoice {}, retrying... (attempt {}/{})",
                                        order,
                                        attempt.saturating_add(1),
                                        MAX_RETRIES
                                    );
                                    continue;
                                }
                                return Err(DaoError::MaxRetriesReached.into());
                            }
                            Err(e) => return Err(DaoError::Sqlx(e).into()),
                        };

                        let order_info = self.invoice_to_order_info(&updated_invoice, &currency);

                        return Ok(OrderResponse::ModifiedOrder(self.order_status(
                            order,
                            order_info,
                            String::new(),
                        )));
                    }
                    // Paid/Expired/Canceled - collision
                    let order_info = self.invoice_to_order_info(&existing_invoice, &currency);
                    return Ok(OrderResponse::CollidedOrder(self.order_status(
                        order,
                        order_info,
                        String::from("Order with this ID was already processed"),
                    )));
                }
            }
        }

        // Should never reach here due to return statements in loop
        Err(DaoError::MaxRetriesReached.into())
    }

    /// Get `CurrencyInfo` from chain and `asset_id` by looking up in currencies `HashMap`
    ///
    /// # Errors
    /// Returns error if currency not found in current configuration
    fn get_currency_info(
        &self,
        chain: &str,
        asset_id: Option<u32>,
    ) -> Result<crate::legacy_types::CurrencyInfo, Error> {
        // Search for matching currency in the currencies HashMap
        for (currency_name, properties) in &self.currencies {
            if properties.chain_name == chain && properties.asset_id == asset_id {
                return Ok(properties.info(currency_name.clone()));
            }
        }

        // Currency not found in current configuration
        Err(OrderError::UnknownCurrency.into())
    }

    /// Convert `TransactionStatus` (new) to `TxStatus` (legacy)
    fn transaction_status_to_tx_status(status: TransactionStatus) -> TxStatus {
        match status {
            TransactionStatus::Completed => TxStatus::Finalized,
            TransactionStatus::Failed => TxStatus::Failed,
            TransactionStatus::Waiting | TransactionStatus::InProgress => TxStatus::Pending,
        }
    }

    /// Convert `Decimal` amount to legacy `Amount` enum
    fn decimal_to_amount(amount: rust_decimal::Decimal) -> Amount {
        // Convert Decimal to f64 for legacy API
        // Note: This may lose precision for very large or very precise numbers
        let amount_f64 = amount.to_string().parse::<f64>().unwrap_or(0.0);
        Amount::Exact(amount_f64)
    }

    /// Convert `Transaction` (new) to `TransactionInfo` (legacy) for backward compatibility
    ///
    /// # Errors
    /// Returns error if currency lookup fails
    fn transaction_to_transaction_info(
        &self,
        transaction: &Transaction,
    ) -> Result<TransactionInfo, Error> {
        // Reconstruct CurrencyInfo from stored asset_id and chain
        let currency = self.get_currency_info(&transaction.chain, Some(transaction.asset_id))?;

        // Convert finalization data
        let finalized_tx = if let (Some(block_number), Some(position_in_block)) =
            (transaction.block_number, transaction.position_in_block)
        {
            Some(FinalizedTx {
                block_number,
                position_in_block,
                timestamp: transaction.created_at.to_rfc3339(),
            })
        } else {
            None
        };

        Ok(TransactionInfo {
            finalized_tx,
            transaction_bytes: transaction.transaction_bytes.clone().unwrap_or_default(),
            sender: transaction.sender.clone(),
            recipient: transaction.recipient.clone(),
            amount: Self::decimal_to_amount(transaction.amount),
            currency,
            status: Self::transaction_status_to_tx_status(transaction.status),
        })
    }

    /// Convert Invoice to `OrderInfo` for backward compatibility with V2 API
    #[expect(clippy::unused_self)]
    fn invoice_to_order_info(
        &self,
        invoice: &Invoice,
        currency: &crate::legacy_types::CurrencyInfo,
    ) -> OrderInfo {
        use crate::legacy_types::PaymentStatus;

        OrderInfo {
            order_id: invoice.order_id.clone(),
            currency: currency.clone(),
            amount: invoice.amount.to_string().parse::<f64>().unwrap_or(0.0),
            payment_account: invoice.payment_address.clone(),
            payment_status: PaymentStatus::from(invoice.status),
            withdrawal_status: invoice.withdrawal_status,
            death: crate::legacy_types::Timestamp(
                #[expect(clippy::cast_sign_loss)]
                {
                    invoice.valid_till.timestamp_millis() as u64
                },
            ),
            callback: invoice.callback.clone(),
            transactions: vec![], // Transactions would be loaded separately if needed
        }
    }

    fn order_status(&self, order: String, order_info: OrderInfo, message: String) -> OrderStatus {
        OrderStatus {
            order,
            message,
            recipient: to_base58_string(self.recipient.0, 2), // TODO maybe but spec says use "2"
            server_info: self.server_info.clone(),
            order_info,
            payment_page: String::new(),
            redirect_url: String::new(),
        }
    }
}

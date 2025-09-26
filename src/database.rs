//! Database server module
//!
//! We do not need concurrency here, as this is our actual source of truth for legally binging
//! commercial offers and contracts, hence causality is a must. Care must be taken that no threads
//! are spawned here other than main database server thread that does everything in series.

use crate::{
    definitions::api_v2::{
        Amount, BlockNumber, CurrencyInfo, ExtrinsicIndex, FinalizedTx, OrderCreateResponse,
        OrderInfo, OrderQuery, PaymentStatus, ServerInfo, Timestamp, TransactionInfo, TxStatus,
        WithdrawalStatus,
    },
    error::DbError,
    utils::task_tracker::TaskTracker,
};
use codec::{Decode, Encode};
use names::Generator;
use sled::Tree;
use std::time::SystemTime;
use substrate_crypto_light::common::AccountId32;
use tokio::sync::{mpsc, oneshot};

pub const MODULE: &str = module_path!();

// Tables
const PENDING_TRANSACTIONS: &str = "pending_transactions";
const TRANSACTIONS: &str = "transactions";

const SERVER_INFO_ID: &str = "instance_id";

const ORDERS_TABLE: &[u8] = b"orders";
const SERVER_INFO_TABLE: &[u8] = b"server_info";

pub struct ConfigWoChains {
    pub recipient: AccountId32,
    pub debug: Option<bool>,
    pub remark: Option<String>,
    //pub depth: Option<Duration>,
}

/// Database server handle
#[derive(Clone, Debug)]
pub struct Database {
    tx: mpsc::Sender<DbRequest>,
}

impl Database {
    #[expect(clippy::too_many_lines)]
    // TODO: check if it's DEFINITELY won't break something. Check `ZeroizeOnDrop` marco implementation
    #[expect(tail_expr_drop_order)]
    pub fn init(
        path_option: Option<String>,
        task_tracker: &TaskTracker,
        account_lifetime: Timestamp,
    ) -> Result<Self, DbError> {
        let (tx, mut rx) = tokio::sync::mpsc::channel(1024);
        let database = if let Some(path) = path_option {
            tracing::info!("Creating/Opening the database at {path:?}.");

            sled::open(path).map_err(DbError::DbStartError)?
        } else {
            // TODO
            /*
            tracing::warn!(
                "The in-memory backend for the database is selected. All saved data will be deleted after the shutdown!"
            );*/
            sled::open("temp.db").map_err(DbError::DbStartError)?
        };
        let orders = database
            .open_tree(ORDERS_TABLE)
            .map_err(DbError::DbStartError)?;
        let transactions = database
            .open_tree(TRANSACTIONS)
            .map_err(DbError::DbStartError)?;
        let pending_transactions = database
            .open_tree(PENDING_TRANSACTIONS)
            .map_err(DbError::DbStartError)?;

        task_tracker.spawn("Database server", async move {
            // No process forking beyond this point!
            while let Some(req) = rx.recv().await {
                match req {
                    DbRequest::ActiveOrderList(res) => {
                        // TODO: require optimization? If yes, can create another table or just in-memory map
                        // like HashMap<PaymentStatus, OrderId> to efficiently search orders by status.
                        let _unused = res.send(Ok(orders
                            .iter()
                            .filter_map(Result::ok)
                            .filter_map(|(encoded_id, encoded_order)| {
                                match (
                                    String::decode(&mut &encoded_id[..]),
                                    OrderInfo::decode(&mut &encoded_order[..]),
                                ) {
                                    (Ok(a), Ok(b)) => Some((a, b)),
                                    _ => None,
                                }
                            })
                            .filter(|(_, b)| b.payment_status == PaymentStatus::Pending)
                            .collect()));
                    }
                    DbRequest::CreateOrder(request) => {
                        let _unused = request.res.send(create_order(
                            &request.order,
                            request.query,
                            request.currency,
                            request.payment_account,
                            &orders,
                            account_lifetime,
                        ));
                    }
                    DbRequest::ReadOrder(request) => {
                        let _unused = request.res.send(read_order(
                            &request.order,
                            &orders,
                            &transactions,
                            &pending_transactions,
                        ));
                    }
                    DbRequest::MarkPaid(request) => {
                        let _unused = request.res.send(mark_paid(request.order, &orders));
                    }
                    DbRequest::IsMarkedPaid(order, res) => {
                        let _unused = res.send(is_marked_paid(&orders, &order));
                    }
                    DbRequest::MarkWithdrawn(request) => {
                        let _unused = request.res.send(mark_withdrawn(request.order, &orders));
                    }
                    DbRequest::MarkForced(request) => {
                        let _unused = request.res.send(mark_forced(request.order, &orders));
                    }
                    DbRequest::MarkStuck(request) => {
                        let _unused = request.res.send(mark_stuck(request.order, &orders));
                    }
                    DbRequest::RecordTransaction {
                        order,
                        tx: transaction_info_db,
                        res,
                    } => {
                        let _unused = res.send(record_transaction(
                            &transactions,
                            &pending_transactions,
                            order,
                            transaction_info_db,
                        ));
                    }
                    DbRequest::InitializeServerInfo(res) => {
                        let server_info_tree = database
                            .open_tree(SERVER_INFO_TABLE)
                            .map_err(DbError::DbStartError);
                        let result = server_info_tree.and_then(|tree| {
                            let data =
                                tree.get(SERVER_INFO_ID).map_err(DbError::DbInternalError)?;

                            if let Some(server_info_data) = data {
                                let server_info: ServerInfo =
                                    serde_json::from_slice(&server_info_data).map_err(|e| {
                                        DbError::DeserializationError(e.to_string())
                                    })?;
                                Ok(server_info.instance_id)
                            } else {
                                let mut generator = Generator::default();
                                let new_instance_id = generator
                                    .next()
                                    .unwrap_or_else(|| "unknown-instance".to_string());
                                let server_info_data = ServerInfo {
                                    version: env!("CARGO_PKG_VERSION").to_string(),
                                    instance_id: new_instance_id.clone(),
                                    debug: false,
                                    kalatori_remark: None,
                                };
                                tree.insert(
                                    SERVER_INFO_ID,
                                    serde_json::to_vec(&server_info_data)
                                        .map_err(|e| DbError::SerializationError(e.to_string()))?,
                                )?;
                                Ok(new_instance_id)
                            }
                        });
                        let _unused = res.send(result);
                    }
                    DbRequest::Shutdown(res) => {
                        let _ = res.send(());
                        break;
                    }
                }
            }

            drop(database.flush());

            Ok("Database server is shutting down")
        });

        Ok(Self { tx })
    }

    pub async fn initialize_server_info(&self) -> Result<String, DbError> {
        let (res, rx) = oneshot::channel();

        self.tx
            .send(DbRequest::InitializeServerInfo(res))
            .await
            .map_err(|_| DbError::DbEngineDown)?;

        rx.await.map_err(|_| DbError::DbEngineDown)?
    }

    pub async fn order_list(&self) -> Result<Vec<(String, OrderInfo)>, DbError> {
        let (res, rx) = oneshot::channel();

        self.tx
            .send(DbRequest::ActiveOrderList(res))
            .await
            .map_err(|_| DbError::DbEngineDown)?;

        rx.await.map_err(|_| DbError::DbEngineDown)?
    }

    pub async fn create_order(
        &self,
        order: String,
        query: OrderQuery,
        currency: CurrencyInfo,
        payment_account: String,
    ) -> Result<OrderCreateResponse, DbError> {
        let (res, rx) = oneshot::channel();

        self.tx
            .send(DbRequest::CreateOrder(CreateOrder {
                order,
                query,
                currency,
                payment_account,
                res,
            }))
            .await
            .map_err(|_| DbError::DbEngineDown)?;

        rx.await.map_err(|_| DbError::DbEngineDown)?
    }

    pub async fn read_order(&self, order: String) -> Result<Option<OrderInfo>, DbError> {
        let (res, rx) = oneshot::channel();

        self.tx
            .send(DbRequest::ReadOrder(ReadOrder { order, res }))
            .await
            .map_err(|_| DbError::DbEngineDown)?;

        rx.await.map_err(|_| DbError::DbEngineDown)?
    }

    pub async fn record_transaction(
        &self,
        order: String,
        tx: TransactionInfoDb,
    ) -> Result<(), DbError> {
        let (res, rx) = oneshot::channel();

        self.tx
            .send(DbRequest::RecordTransaction { order, tx, res })
            .await
            .map_err(|_| DbError::DbEngineDown)?;

        rx.await.map_err(|_| DbError::DbEngineDown)?
    }

    pub async fn mark_paid(&self, order: String) -> Result<OrderInfo, DbError> {
        let (res, rx) = oneshot::channel();

        self.tx
            .send(DbRequest::MarkPaid(MarkPaid { order, res }))
            .await
            .map_err(|_| DbError::DbEngineDown)?;

        rx.await.map_err(|_| DbError::DbEngineDown)?
    }

    pub async fn is_marked_paid(&self, order: String) -> Result<bool, DbError> {
        let (res, rx) = oneshot::channel();
        self.tx
            .send(DbRequest::IsMarkedPaid(order, res))
            .await
            .map_err(|_| DbError::DbEngineDown)?;
        rx.await.map_err(|_| DbError::DbEngineDown)?
    }

    pub async fn mark_withdrawn(&self, order: String) -> Result<(), DbError> {
        let (res, rx) = oneshot::channel();
        self.tx
            .send(DbRequest::MarkWithdrawn(ModifyOrder { order, res }))
            .await
            .map_err(|_| DbError::DbEngineDown)?;
        rx.await.map_err(|_| DbError::DbEngineDown)?
    }
    pub async fn mark_forced(&self, order: String) -> Result<(), DbError> {
        let (res, rx) = oneshot::channel();
        self.tx
            .send(DbRequest::MarkForced(ModifyOrder { order, res }))
            .await
            .map_err(|_| DbError::DbEngineDown)?;
        rx.await.map_err(|_| DbError::DbEngineDown)?
    }

    #[expect(dead_code)]
    pub async fn mark_stuck(&self, order: String) -> Result<(), DbError> {
        let (res, rx) = oneshot::channel();
        self.tx
            .send(DbRequest::MarkStuck(ModifyOrder { order, res }))
            .await
            .map_err(|_| DbError::DbEngineDown)?;
        rx.await.map_err(|_| DbError::DbEngineDown)?
    }

    pub async fn shutdown(&self) {
        let (tx, rx) = oneshot::channel();
        let _unused = self.tx.send(DbRequest::Shutdown(tx)).await;
        let _ = rx.await;
    }
}

enum DbRequest {
    CreateOrder(CreateOrder),
    ActiveOrderList(oneshot::Sender<Result<Vec<(String, OrderInfo)>, DbError>>),
    ReadOrder(ReadOrder),
    MarkPaid(MarkPaid),
    MarkWithdrawn(ModifyOrder),
    MarkForced(ModifyOrder),
    IsMarkedPaid(String, oneshot::Sender<Result<bool, DbError>>),
    MarkStuck(ModifyOrder),
    InitializeServerInfo(oneshot::Sender<Result<String, DbError>>),
    Shutdown(oneshot::Sender<()>),
    RecordTransaction {
        order: String,
        tx: TransactionInfoDb,
        res: oneshot::Sender<Result<(), DbError>>,
    },
}

pub struct CreateOrder {
    pub order: String,
    pub query: OrderQuery,
    pub currency: CurrencyInfo,
    pub payment_account: String,
    pub res: oneshot::Sender<Result<OrderCreateResponse, DbError>>,
}

pub struct ReadOrder {
    pub order: String,
    pub res: oneshot::Sender<Result<Option<OrderInfo>, DbError>>,
}

pub struct ModifyOrder {
    pub order: String,
    pub res: oneshot::Sender<Result<(), DbError>>,
}

pub struct MarkPaid {
    pub order: String,
    pub res: oneshot::Sender<Result<OrderInfo, DbError>>,
}

fn calculate_death_ts(account_lifetime: Timestamp) -> Timestamp {
    #[expect(clippy::cast_possible_truncation)]
    let start = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;

    #[expect(clippy::arithmetic_side_effects)]
    Timestamp(start + account_lifetime.0)
}

fn create_order(
    order: &str,
    query: OrderQuery,
    currency: CurrencyInfo,
    payment_account: String,
    orders: &Tree,
    account_lifetime: Timestamp,
) -> Result<OrderCreateResponse, DbError> {
    let order_key = order.encode();

    let resp = match get_order(orders, order) {
        // If order already exists, update it
        Ok(mut old_order_info) => match old_order_info.payment_status {
            PaymentStatus::Pending => {
                let death = calculate_death_ts(account_lifetime);

                old_order_info.death = death;
                old_order_info.currency = currency;
                old_order_info.amount = query.amount;

                orders.insert(&order_key, old_order_info.encode())?;
                OrderCreateResponse::Modified(old_order_info)
            }
            PaymentStatus::Paid => OrderCreateResponse::Collision(old_order_info),
        },
        // If order not exists, create it
        Err(DbError::OrderNotFound(_)) => {
            let death = calculate_death_ts(account_lifetime);
            let order_info_new = OrderInfo::new(query, currency, payment_account, death);

            orders.insert(&order_key, order_info_new.encode())?;
            OrderCreateResponse::New(order_info_new)
        }
        // Return any else database errors
        Err(e) => return Err(e),
    };

    Ok(resp)
}

fn read_order(
    key: &str,
    orders: &Tree,
    tx_table: &Tree,
    pending_tx_table: &Tree,
) -> Result<Option<OrderInfo>, DbError> {
    let order_key = key.encode();
    let Some(order_encoded) = orders.get(&order_key)? else {
        return Ok(None);
    };

    let mut order = OrderInfo::decode(&mut &order_encoded[..])?;
    let transactions = tx_table
        .scan_prefix(&order_key)
        .map(|result| {
            result.map_err(DbError::from).and_then(|(k, v)| {
                let (_order_key, block_number, position_in_block) =
                    <(String, BlockNumber, ExtrinsicIndex)>::decode(&mut k.as_ref())?;

                TransactionInfoDb::decode(&mut v.as_ref())
                    .map(|mut tx| {
                        tx.inner.finalized_tx = Some(FinalizedTxDb {
                            block_number,
                            position_in_block,
                        });

                        TransactionInfo::from(tx)
                    })
                    .map_err(Into::into)
            })
        })
        .chain(pending_tx_table.scan_prefix(order_key).map(|result| {
            result.map_err(DbError::from).and_then(|(k, v)| {
                let (_order_key, transaction_bytes) = <(String, String)>::decode(&mut k.as_ref())?;

                TransactionInfoDbInner::decode(&mut v.as_ref())
                    .map(|tx| TransactionInfo {
                        finalized_tx: None,
                        transaction_bytes,
                        sender: tx.sender,
                        recipient: tx.recipient,
                        amount: tx.amount,
                        currency: tx.currency,
                        status: tx.status,
                    })
                    .map_err(Into::into)
            })
        }))
        .collect::<Result<Vec<_>, _>>()?;

    order.transactions = transactions;

    Ok(order.into())
}

fn record_transaction(
    tx_table: &Tree,
    pending_tx_table: &Tree,
    order: String,
    mut tx: TransactionInfoDb,
) -> Result<(), DbError> {
    let pending_tx_key = (order.clone(), tx.transaction_bytes.clone()).encode();
    let finalized_info = tx
        .inner
        .finalized_tx
        .take()
        .zip(tx.inner.finalized_tx_timestamp.as_ref());

    // Search the given transaction among pending ones and update it or move it to finalized
    // transactions.
    let encoded_tx_inner = pending_tx_table.get(&pending_tx_key)?;

    if let Some(_encoded_tx_inner) = encoded_tx_inner {
        if let Some((finalized_tx, _finalized_tx_timestamp)) = finalized_info {
            tracing::debug!("moving pending tx to finalized");

            pending_tx_table.remove(pending_tx_key)?;

            tx_table.insert(
                (
                    order,
                    finalized_tx.block_number,
                    finalized_tx.position_in_block,
                )
                    .encode(),
                tx.encode(),
            )?;
        } else {
            tracing::debug!("updating pending tx");

            pending_tx_table.insert(pending_tx_key, tx.inner.encode())?;
        }
    // Save the given finalized transaction.
    } else if let Some((finalized_tx, _finalized_tx_timestamp)) = finalized_info {
        tracing::debug!("save finalized tx");

        tx_table.insert(
            (
                order,
                finalized_tx.block_number,
                finalized_tx.position_in_block,
            )
                .encode(),
            tx.encode(),
        )?;

    // Save the pending transaction.
    } else {
        tracing::debug!("adding pending tx");

        pending_tx_table.insert(pending_tx_key, tx.inner.encode())?;
    }

    Ok(())
}

fn get_order(orders_tree: &Tree, order: &str) -> Result<OrderInfo, DbError> {
    let order_key = order.encode();

    match orders_tree.get(order_key)? {
        Some(value) => Ok(OrderInfo::decode(&mut &value[..])?),
        None => Err(DbError::OrderNotFound(order.to_string())),
    }
}

fn mark_paid(order: String, orders: &Tree) -> Result<OrderInfo, DbError> {
    let mut order_info = get_order(orders, &order)?;

    if order_info.payment_status == PaymentStatus::Pending {
        order_info.payment_status = PaymentStatus::Paid;
        orders.insert(order.encode(), order_info.encode())?;
        Ok(order_info)
    } else {
        Err(DbError::AlreadyPaid(order))
    }
}

fn is_marked_paid(orders: &Tree, order: &str) -> Result<bool, DbError> {
    let order_info = get_order(orders, order)?;
    Ok(order_info.payment_status == PaymentStatus::Paid)
}

fn mark_withdrawn(order: String, orders: &Tree) -> Result<(), DbError> {
    let mut order_info = get_order(orders, &order)?;

    if order_info.payment_status == PaymentStatus::Paid {
        if order_info.withdrawal_status == WithdrawalStatus::Waiting {
            order_info.withdrawal_status = WithdrawalStatus::Completed;
            orders.insert(order.encode(), order_info.encode())?;
            Ok(())
        } else {
            Err(DbError::WithdrawalWasAttempted(order))
        }
    } else {
        Err(DbError::NotPaid(order))
    }
}

fn mark_forced(order: String, orders: &Tree) -> Result<(), DbError> {
    let mut order_info = get_order(orders, &order)?;

    if order_info.payment_status == PaymentStatus::Pending
        || order_info.payment_status == PaymentStatus::Paid
    {
        if order_info.withdrawal_status == WithdrawalStatus::Waiting {
            order_info.withdrawal_status = WithdrawalStatus::Forced;
            orders.insert(order.encode(), order_info.encode())?;
            Ok(())
        } else {
            Err(DbError::WithdrawalWasAttempted(order))
        }
    } else {
        Err(DbError::NotPaid(order))
    }
}

fn mark_stuck(order: String, orders: &Tree) -> Result<(), DbError> {
    let mut order_info = get_order(orders, &order)?;

    if order_info.payment_status == PaymentStatus::Paid {
        if order_info.withdrawal_status == WithdrawalStatus::Waiting {
            order_info.withdrawal_status = WithdrawalStatus::Failed;
            orders.insert(order.encode(), order_info.encode())?;
            Ok(())
        } else {
            Err(DbError::WithdrawalWasAttempted(order))
        }
    } else {
        Err(DbError::NotPaid(order))
    }
}

#[derive(Encode, Decode)]
pub struct TransactionInfoDbInner {
    pub finalized_tx: Option<FinalizedTxDb>,
    pub finalized_tx_timestamp: Option<String>,
    pub sender: String,
    pub recipient: String,
    pub amount: Amount,
    pub currency: CurrencyInfo,
    pub status: TxStatus,
    pub kind: TxKind,
}

#[derive(Encode, Decode)]
pub struct TransactionInfoDb {
    pub transaction_bytes: String,
    pub inner: TransactionInfoDbInner,
}

#[derive(Encode, Decode, Debug)]
pub enum TxKind {
    Payment,
    Withdrawal,
}

#[derive(Encode, Decode)]
pub struct FinalizedTxDb {
    pub block_number: BlockNumber,
    pub position_in_block: ExtrinsicIndex,
}

impl From<TransactionInfoDb> for TransactionInfo {
    fn from(value: TransactionInfoDb) -> Self {
        let finalized_tx = value.inner.finalized_tx.and_then(|tx| {
            value
                .inner
                .finalized_tx_timestamp
                .map(|timestamp| FinalizedTx {
                    block_number: tx.block_number,
                    position_in_block: tx.position_in_block,
                    timestamp,
                })
        });

        Self {
            finalized_tx,
            transaction_bytes: value.transaction_bytes,
            sender: value.inner.sender,
            recipient: value.inner.recipient,
            amount: value.inner.amount,
            currency: value.inner.currency,
            status: value.inner.status,
        }
    }
}

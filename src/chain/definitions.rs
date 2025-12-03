//! Common objects for chain interaction system

use std::str::FromStr;

use crate::chain::tracker::ChainWatcher;
use crate::chain_client::{
    AssetHubClient,
    BlockChainClient,
};
use crate::definitions::Balance;
use crate::error::ChainError;
use crate::legacy_types::{
    CurrencyInfo,
    OrderInfo,
    RpcInfo,
    Timestamp,
};
use rust_decimal::prelude::{
    Decimal,
    ToPrimitive,
};
use subxt::utils::AccountId32;
use tokio::sync::oneshot;
use uuid::Uuid;

pub enum ChainRequest {
    WatchAccount(WatchAccount),
    Reap(WatchAccount),
    Shutdown(oneshot::Sender<()>),
    GetConnectedRpcs(oneshot::Sender<Vec<RpcInfo>>),
}

#[derive(Debug)]
pub struct WatchAccount {
    pub id: Uuid,
    pub order_id: String,
    pub address: AccountId32,
    pub currency: CurrencyInfo,
    pub amount: f64,
    pub recipient: AccountId32,
    pub res: oneshot::Sender<Result<(), ChainError>>,
    pub death: Timestamp,
}

impl WatchAccount {
    pub fn new(
        id: Uuid,
        order: OrderInfo,
        recipient: AccountId32,
        res: oneshot::Sender<Result<(), ChainError>>,
    ) -> Result<WatchAccount, ChainError> {
        Ok(WatchAccount {
            id,
            order_id: order.order_id,
            address: AccountId32::from_str(&order.payment_account)
                .map_err(|e| ChainError::InvoiceAccount(e.to_string()))?,
            currency: order.currency,
            amount: order.amount,
            recipient,
            res,
            death: order.death,
        })
    }
}

pub enum ChainTrackerRequest {
    WatchAccount(WatchAccount),
    Transfers(Vec<crate::chain_client::ChainTransfer<crate::chain_client::AssetHubChainConfig>>),
    Reap(WatchAccount),
    #[expect(dead_code)]
    ForceReap(WatchAccount),
    Shutdown(oneshot::Sender<()>),
}

#[derive(Clone, Debug)]
pub struct Invoice {
    pub id: Uuid,
    pub order_id: String,
    pub address: AccountId32,
    pub currency: CurrencyInfo,
    pub amount: f64,
    pub recipient: AccountId32,
    pub death: Timestamp,
}

impl Invoice {
    pub fn from_request(watch_account: WatchAccount) -> Self {
        drop(watch_account.res.send(Ok(())));
        Invoice {
            id: watch_account.id,
            order_id: watch_account.order_id,
            address: watch_account.address,
            currency: watch_account.currency,
            amount: watch_account.amount,
            recipient: watch_account.recipient,
            death: watch_account.death,
        }
    }

    pub async fn balance(
        &self,
        client: &AssetHubClient,
        chain_watcher: &ChainWatcher,
    ) -> Result<Balance, ChainError> {
        let currency = chain_watcher
            .assets
            .get(&self.currency.currency)
            .ok_or_else(|| ChainError::InvalidCurrency(self.currency.currency.clone()))?;

        // TODO: asset_id shouldn't be optional, will change in future
        let Some(asset_id) = currency.asset_id else {
            return Err(ChainError::InvalidCurrency(
                self.currency.currency.clone(),
            ));
        };

        let amount = client
            .fetch_asset_balance(&asset_id, &self.address)
            .await
            .map_err(|_| ChainError::StorageQuery)?;

        #[expect(clippy::arithmetic_side_effects)]
        let balance = (amount / Decimal::new(1, 6))
            .to_u128()
            .unwrap();

        Ok(Balance(balance))
    }

    pub async fn check(
        &self,
        client: &AssetHubClient,
        chain_watcher: &ChainWatcher,
    ) -> Result<bool, ChainError> {
        // TODO: what if we receive significantly more money then expect? Perhaps need
        // to check in some range?
        Ok(self
            .balance(client, chain_watcher)
            .await?
            >= Balance::parse(self.amount, self.currency.decimals))
    }
}

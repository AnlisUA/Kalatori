//! Common objects for chain interaction system

use std::str::FromStr;

use crate::{
    chain::AssetHubOnlineClient,
    chain::tracker::ChainWatcher,
    definitions::{
        Balance,
        api_v2::{CurrencyInfo, OrderInfo, RpcInfo, Timestamp},
    },
    error::ChainError,
};
use subxt::utils::AccountId32;
use tokio::sync::oneshot;

pub enum ChainRequest {
    WatchAccount(WatchAccount),
    Reap(WatchAccount),
    Shutdown(oneshot::Sender<()>),
    GetConnectedRpcs(oneshot::Sender<Vec<RpcInfo>>),
}

#[derive(Debug)]
pub struct WatchAccount {
    pub id: String,
    pub address: AccountId32,
    pub currency: CurrencyInfo,
    pub amount: f64,
    pub recipient: AccountId32,
    pub res: oneshot::Sender<Result<(), ChainError>>,
    pub death: Timestamp,
}

impl WatchAccount {
    pub fn new(
        id: String,
        order: OrderInfo,
        recipient: AccountId32,
        res: oneshot::Sender<Result<(), ChainError>>,
    ) -> Result<WatchAccount, ChainError> {
        Ok(WatchAccount {
            id,
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
    NewBlock(
        subxt::blocks::Block<crate::chain::AssetHubConfig, crate::chain::AssetHubOnlineClient>,
    ),
    Reap(WatchAccount),
    #[expect(dead_code)]
    ForceReap(WatchAccount),
    Shutdown(oneshot::Sender<()>),
}

#[derive(Clone, Debug)]
pub struct Invoice {
    pub id: String,
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
            address: watch_account.address,
            currency: watch_account.currency,
            amount: watch_account.amount,
            recipient: watch_account.recipient,
            death: watch_account.death,
        }
    }

    pub async fn balance(
        &self,
        client: &AssetHubOnlineClient,
        chain_watcher: &ChainWatcher,
    ) -> Result<Balance, ChainError> {
        let currency = chain_watcher
            .assets
            .get(&self.currency.currency)
            .ok_or_else(|| ChainError::InvalidCurrency(self.currency.currency.clone()))?;

        // TODO: asset_id shouldn't be optional, will change in future
        let Some(asset_id) = currency.asset_id else {
            return Err(ChainError::InvalidCurrency(self.currency.currency.clone()));
        };

        let request_data = crate::chain::runtime::storage()
            .assets()
            // TODO: change stored type to subxt's account id
            .account(asset_id, subxt::utils::AccountId32(self.address.0));

        let balance = client
            .storage()
            .at_latest()
            .await?
            .fetch(&request_data)
            .await?
            .map_or(0, |result| result.balance);

        Ok(Balance(balance))
    }

    pub async fn check(
        &self,
        client: &AssetHubOnlineClient,
        chain_watcher: &ChainWatcher,
    ) -> Result<bool, ChainError> {
        // TODO: what if we receive significantly more money then expect? Perhaps need to check in some range?
        Ok(self.balance(client, chain_watcher).await?
            >= Balance::parse(self.amount, self.currency.decimals))
    }
}

use chrono::{Duration, Utc};
use uuid::Uuid;
use rust_decimal::Decimal;

use crate::chain::{InvoiceRegistry, InvoiceRegistryRecord};
use crate::chain::utils::to_base58_string;
use crate::chain_client::KeyringClient;
use crate::configs::PaymentsConfig;
use crate::dao::{
    DaoInterface,
    DaoTransactionInterface,
    DaoTransactionError,
};
use crate::dao::DaoInvoiceError;
use crate::types::{
    CreateInvoiceData,
    Invoice,
    Payout,
    Transaction,
    UpdateInvoiceData,
};
use crate::handlers::types::{CreateInvoiceParams, UpdateInvoiceParams};

#[derive(Clone)]
pub struct AppState<D: DaoInterface + Clone + 'static> {
    keyring: KeyringClient,
    dao: D,
    registry: InvoiceRegistry,
    payments_config: PaymentsConfig,
}

impl<D: DaoInterface + Clone + 'static> AppState<D> {
    pub fn new(
        keyring: KeyringClient,
        dao: D,
        registry: InvoiceRegistry,
        payments_config: PaymentsConfig,
    ) -> Self {
        Self {
            keyring,
            dao,
            registry,
            payments_config,
        }
    }

    pub async fn get_invoice(
        &self,
        invoice_id: Uuid,
    ) -> Result<Option<Invoice>, DaoInvoiceError> {
        self.dao.get_invoice_by_id(invoice_id).await
    }

    pub async fn get_invoice_by_order_id(
        &self,
        order_id: &str,
    ) -> Result<Option<Invoice>, DaoInvoiceError> {
        self.dao.get_invoice_by_order_id(order_id).await
    }

    pub async fn create_invoice(
        &self,
        params: CreateInvoiceParams,
    ) -> Result<Invoice, DaoInvoiceError> {
        let id = Uuid::new_v4();
        // Later we can extend CreateInvoiceParams to include optional chain and asset_id
        let chain = self.payments_config.default_chain.clone();
        let asset_id = params.asset_id.unwrap_or_else(|| self.payments_config.default_asset_id.clone());
        let valid_till = Utc::now() + Duration::milliseconds(self.payments_config.account_lifetime_millis as i64);

        let payment_address = match chain.as_str() {
            "statemint" => {
                let derivation_params = vec![id.to_string()];

                let account_id = self
                    .keyring
                    .generate_asset_hub_address(derivation_params.into())
                    .await
                    // TODO: handle error
                    .unwrap();

                to_base58_string(account_id.0, 0)
            },
            _ => unreachable!()
        };

        let data = CreateInvoiceData {
            order_id: params.order_id,
            callback_url: params.callback_url,
            amount: params.amount,
            cart: params.cart,
            redirect_url: params.redirect_url,
            id,
            asset_id: asset_id.parse().unwrap(),
            chain,
            payment_address,
            valid_till,
        };

        // TODO: refactor create_invoice to take CreateInvoiceData directly
        let invoice = self.dao.create_invoice(data.into()).await?;

        tracing::info!(
            invoice_id = %invoice.id,
            payment_address = %invoice.payment_address,
            "Created new invoice",
        );

        self.registry.add_invoice(InvoiceRegistryRecord::new(
            invoice.clone(),
            Decimal::ZERO,
        )).await;

        Ok(invoice)
    }

    pub async fn update_invoice(&self, params: UpdateInvoiceParams) -> Result<Invoice, DaoInvoiceError> {
        // TODO: current implementation of the whole method is not optimal. We fetch the invoice first to get it's current data,
        // then we update it with new data. It would be better to have a method in DAO that updates only the fields that are provided,
        // For that we would need to use query builder instead of raw SQL queries.
        let invoice = self.dao
            .get_invoice_by_id(params.invoice_id)
            .await?
            .ok_or_else(|| DaoInvoiceError::NotFound {
                identifier: params.invoice_id.to_string()
            })?;

        let data = UpdateInvoiceData {
            id: params.invoice_id,
            amount: params.amount.unwrap_or(invoice.amount),
            cart: params.cart.unwrap_or(invoice.cart),
            valid_till: Utc::now() + Duration::milliseconds(self.payments_config.account_lifetime_millis as i64),
            version: invoice.version,
        };

        self.dao.update_invoice_data(data).await
    }

    pub async fn get_invoice_transactions(&self, invoice_id: Uuid) -> Result<Vec<Transaction>, DaoTransactionError> {
        self.dao.get_invoice_transactions(invoice_id).await
    }

    // TODO: also mark invoice as paid? Probably, need to change main status as well. In that case, also remove from registry
    pub async fn force_withdrawal(&self, order_id: String) -> Result<Invoice, DaoInvoiceError> {
        match self.dao.get_invoice_by_order_id(&order_id).await? {
            Some(invoice) => {
                let dao_transaction = self.dao
                    .begin_transaction()
                    .await
                    .map_err(|_| DaoInvoiceError::DatabaseError)?;

                let payout = Payout {
                    id: Uuid::new_v4(),
                    invoice_id: invoice.id,
                    initiator_type: crate::types::InitiatorType::System,
                    initiator_id: None,
                    status: crate::types::PayoutStatus::Waiting,
                    transfer_info: crate::types::TransferInfo {
                        chain: invoice.chain.clone(),
                        asset_id: invoice.asset_id.unwrap_or(0).to_string(),
                        amount: invoice.amount,
                        source_address: invoice.payment_address,
                        destination_address: self.payments_config.recipient.clone(),
                    },
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                    retry_meta: crate::types::RetryMeta::default(),
                };

                let _result = dao_transaction.create_payout(payout)
                    .await
                    .map_err(|_| DaoInvoiceError::DatabaseError)?;

                let marked = dao_transaction
                    .update_invoice_withdrawal_status(invoice.id, crate::legacy_types::WithdrawalStatus::Forced)
                    .await?;

                dao_transaction
                    .commit()
                    .await
                    .map_err(|_| DaoInvoiceError::DatabaseError)?;

                Ok(marked)
            },
            None => {
                tracing::error!("Invoice for order_id {order_id} not found in new database");
                Err(DaoInvoiceError::NotFound { identifier: order_id  })
            },
        }
    }
}

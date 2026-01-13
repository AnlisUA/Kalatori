use std::collections::HashMap;

use chrono::{
    Duration,
    Utc,
};
use rust_decimal::Decimal;
use uuid::Uuid;

use kalatori_client::types::{
    CreateInvoiceParams, Invoice as PublicInvoice, InvoiceStatus, UpdateInvoiceParams
};

use crate::chain::utils::to_base58_string;
use crate::chain::{
    InvoiceRegistry,
    InvoiceRegistryRecord,
};
use crate::chain_client::KeyringClient;
use crate::configs::PaymentsConfig;
use crate::dao::{
    DaoInterface,
    DaoInvoiceError,
    DaoTransactionError,
};
use crate::types::{
    ChainType,
    CreateInvoiceData,
    Invoice,
    InvoiceWithIncomingAmount,
    Transaction,
    UpdateInvoiceData,
};

pub struct AppState<D: DaoInterface> {
    keyring: KeyringClient,
    dao: D,
    registry: InvoiceRegistry,
    asset_names_map: HashMap<String, String>,
    payments_config: PaymentsConfig,
}

impl<D: DaoInterface> AppState<D> {
    pub fn new(
        keyring: KeyringClient,
        dao: D,
        registry: InvoiceRegistry,
        asset_names_map: HashMap<String, String>,
        payments_config: PaymentsConfig,
    ) -> Self {
        Self {
            keyring,
            dao,
            registry,
            asset_names_map,
            payments_config,
        }
    }

    fn build_payment_url(&self, invoice_id: Uuid) -> String {
        format!(
            "{}/public?{}",
            // self.payments_config.payment_base_url,
            "localhost:16726",
            invoice_id,
        )
    }

    pub fn build_public_invoice(&self, invoice: InvoiceWithIncomingAmount) -> PublicInvoice {
        let InvoiceWithIncomingAmount { invoice, total_received_amount: incoming_amount } = invoice;

        PublicInvoice {
            id: invoice.id,
            order_id: invoice.order_id,
            amount: invoice.amount,
            asset_id: invoice.asset_id,
            asset_name: invoice.asset_name,
            chain: invoice.chain,
            payment_address: invoice.payment_address,
            payment_url: self.build_payment_url(invoice.id),
            status: invoice.status,
            cart: invoice.cart,
            total_received_amount: incoming_amount,
            redirect_url: invoice.redirect_url,
            valid_till: invoice.valid_till,
            created_at: invoice.created_at,
            updated_at: invoice.updated_at,
        }
    }

    pub async fn get_invoice(
        &self,
        invoice_id: Uuid,
    ) -> Result<Option<Invoice>, DaoInvoiceError> {
        self.dao
            .get_invoice_by_id(invoice_id)
            .await
    }

    #[expect(clippy::arithmetic_side_effects, clippy::cast_possible_wrap)]
    #[tracing::instrument(skip_all)]
    pub async fn create_invoice(
        &self,
        params: CreateInvoiceParams,
    ) -> Result<Invoice, DaoInvoiceError> {
        let id = Uuid::new_v4();
        // Later we can extend CreateInvoiceParams to include optional chain and
        // asset_id
        let chain = self
            .payments_config
            .default_chain;

        let asset_id = self.payments_config
            .default_asset_id
            .clone();

        let asset_name = self.asset_names_map.get(&asset_id)
            .cloned()
            // This should never happen, but just in case
            .unwrap_or_else(|| "UNKNOWN".to_string());

        let valid_till = Utc::now()
            + Duration::milliseconds(
                self.payments_config
                    .account_lifetime_millis as i64,
            );

        let payment_address = match chain {
            ChainType::PolkadotAssetHub => {
                let derivation_params = vec![id.to_string()];

                let account_id = self
                    .keyring
                    .generate_asset_hub_address(derivation_params.into())
                    .await
                    .map_err(|e| {
                        tracing::error!(
                            error.category = "create_invoice",
                            error.operation = "generate_asset_hub_address",
                            error.source = ?e,
                            "Failed to generate payment address for new invoice",
                        );
                        // TODO: replace error
                        DaoInvoiceError::DatabaseError
                    })?;

                to_base58_string(account_id.0, 0)
            },
        };

        let data = CreateInvoiceData {
            order_id: params.order_id,
            amount: params.amount,
            cart: params.cart,
            redirect_url: params.redirect_url,
            id,
            asset_id,
            asset_name,
            chain,
            payment_address,
            valid_till,
        };

        let invoice = self
            .dao
            .create_invoice(data)
            .await?;

        tracing::info!(
            invoice_id = %invoice.id,
            payment_address = %invoice.payment_address,
            "Created new invoice",
        );

        self.registry
            .add_invoice(InvoiceRegistryRecord::new(
                invoice.clone(),
                Decimal::ZERO,
            ))
            .await;

        Ok(invoice)
    }

    #[expect(clippy::arithmetic_side_effects, clippy::cast_possible_wrap)]
    pub async fn update_invoice(
        &self,
        params: UpdateInvoiceParams,
    ) -> Result<Invoice, DaoInvoiceError> {
        let invoice = self
            .dao
            .get_invoice_by_id(params.invoice_id)
            .await?
            .ok_or_else(|| DaoInvoiceError::NotFound {
                invoice_id: params.invoice_id,
            })?;

        let data = UpdateInvoiceData {
            invoice_id: params.invoice_id,
            amount: params.amount,
            cart: params.cart,
            valid_till: Utc::now()
                + Duration::milliseconds(
                    self.payments_config
                        .account_lifetime_millis as i64,
                ),
        };

        self.dao.update_invoice_data(data).await
    }

    pub async fn cancel_invoice_admin(
        &self,
        invoice_id: Uuid,
    ) -> Result<Invoice, DaoInvoiceError> {
        self.dao
            .update_invoice_status(invoice_id, InvoiceStatus::AdminCanceled)
            .await
    }

    pub async fn get_invoice_transactions(
        &self,
        invoice_id: Uuid,
    ) -> Result<Vec<Transaction>, DaoTransactionError> {
        self.dao
            .get_invoice_transactions(invoice_id)
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use mockall::predicate::eq;

    use crate::chain_client::KeyringError;
    use crate::dao::MockDaoInterface;
    use crate::types::{InvoiceCart, default_invoice};

    use super::*;

    fn setup_app_state() -> AppState<MockDaoInterface> {
        let asset_names_map = HashMap::from([
            (1337.to_string(), "USDC".to_string()),
            (1984.to_string(), "USDt".to_string()),
        ]);

        let config = PaymentsConfig {
            default_chain: ChainType::PolkadotAssetHub,
            default_asset_id: "1337".to_string(),
            account_lifetime_millis: 600_000,
            recipient: "5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty".to_string(),
        };

        let keyring = KeyringClient::default();
        let dao = MockDaoInterface::default();
        let registry = InvoiceRegistry::new();

        AppState::new(
            keyring,
            dao,
            registry,
            asset_names_map,
            config,
        )
    }

    fn compare_create_invoice_data(
        expected: &CreateInvoiceData,
        actual: &CreateInvoiceData,
    ) -> bool {
        // We don't compare IDs here, as they are generated randomly
        expected.order_id == actual.order_id
            && expected.amount == actual.amount
            && expected.cart == actual.cart
            && expected.redirect_url == actual.redirect_url
            && expected.asset_id == actual.asset_id
            && expected.chain == actual.chain
            && expected.payment_address == actual.payment_address
            // It might be off by a few milliseconds, so we compare timestamps.
            // It still might fail if the test runs too slow, but it's unlikely.
            && expected.valid_till.timestamp() == actual.valid_till.timestamp()
    }

    fn compare_created_invoice(
        expected: &Invoice,
        actual: &Invoice,
    ) -> bool {
        // We don't compare IDs here, as they are generated randomly
        expected.order_id == actual.order_id
            && expected.asset_id == actual.asset_id
            && expected.asset_name == actual.asset_name
            && expected.chain == actual.chain
            && expected.amount == actual.amount
            && expected.payment_address == actual.payment_address
            && expected.status == actual.status
            && expected.cart == actual.cart
            && expected.redirect_url == actual.redirect_url
            // It might be off by a few milliseconds, so we compare timestamps.
            // It still might fail if the test runs too slow, but it's unlikely.
            && expected.valid_till.timestamp() == actual.valid_till.timestamp()
            && expected.created_at.timestamp() == actual.created_at.timestamp()
            && expected.updated_at.timestamp() == actual.updated_at.timestamp()
    }

    #[tokio::test]
    async fn test_get_invoice() {
        let mut app_state = setup_app_state();
        let invoice_id = Uuid::new_v4();

        // Test case 1: Invoice found
        let invoice = Invoice {
            id: invoice_id,
            ..default_invoice()
        };

        let returning_invoice = invoice.clone();

        app_state.dao
            .expect_get_invoice_by_id()
            .once()
            .with(eq(invoice_id))
            .returning(move |_| Ok(Some(returning_invoice.clone())));

        let result = app_state
            .get_invoice(invoice_id)
            .await
            .unwrap();

        assert_eq!(result, Some(invoice));

        // Test case 2: Invoice not found
        let invoice_id = Uuid::new_v4();

        app_state.dao
            .expect_get_invoice_by_id()
            .once()
            .with(eq(invoice_id))
            .returning(|_| Ok(None));

        let result = app_state
            .get_invoice(invoice_id)
            .await
            .unwrap();

        assert_eq!(result, None);

        // Test case 3: Database error
        let invoice_id = Uuid::new_v4();

        app_state.dao
            .expect_get_invoice_by_id()
            .once()
            .with(eq(invoice_id))
            .returning(|_| Err(DaoInvoiceError::DatabaseError));

        let result = app_state
            .get_invoice(invoice_id)
            .await;

        assert!(matches!(result, Err(DaoInvoiceError::DatabaseError)));
    }

    #[tokio::test]
    async fn test_create_invoice() {
        let mut app_state = setup_app_state();

        let uri = subxt_signer::SecretUri::from_str("//Bob").unwrap();
        let keypair = subxt_signer::sr25519::Keypair::from_uri(&uri).unwrap();
        let account_id = keypair.public_key().to_account_id();
        // Multiple clones to move into closures
        let bob_account_id_1 = account_id.clone();
        let bob_account_id_2 = account_id.clone();

        // Test case 1: Successful invoice creation
        // Expected:
        // - KeyringClient called to generate address
        // - Asset ID replaced with default value (not provided in params)
        // - DAO called to create invoice
        // - Registry updated with new invoice
        let params = CreateInvoiceParams {
            order_id: "order123".to_string(),
            amount: Decimal::new(1000, 2), // 10.00
            cart: InvoiceCart::empty(),
            redirect_url: "https://redirect.url".to_string(),
        };

        app_state.keyring
            .expect_generate_asset_hub_address()
            .once()
            .withf(|data|
                data.derivation_params.len() == 1
                && Uuid::from_str(&data.derivation_params[0]).is_ok()
            )
            .returning(move |_| Ok(bob_account_id_1.clone()));

        let expected_create_invoice_data = {
            CreateInvoiceData {
                id: Uuid::new_v4(), // We can't predict this, so we'll match fields except ID
                order_id: params.order_id.clone(),
                amount: params.amount,
                cart: params.cart.clone(),
                redirect_url: params.redirect_url.clone(),
                asset_id: 1337.to_string(),
                asset_name: "USDC".to_string(),
                chain: ChainType::PolkadotAssetHub,
                payment_address: to_base58_string(account_id.0, 0),
                valid_till: Utc::now() + Duration::milliseconds(app_state.payments_config.account_lifetime_millis as i64),
            }
        };

        let mut expected_invoice: Invoice = expected_create_invoice_data.clone().into();

        app_state.dao
            .expect_create_invoice()
            .once()
            .withf(move |data| compare_create_invoice_data(&expected_create_invoice_data, data))
            .returning(|data| Ok(data.into()));

        let result = app_state
            .create_invoice(params.clone())
            .await
            .unwrap();

        expected_invoice.id = result.id; // Set the ID to match for comparison
        assert!(compare_created_invoice(&expected_invoice, &result));

        let registry_record = app_state.registry.get_invoice(&result.id).await.unwrap();
        assert_eq!(registry_record.invoice, result);
        assert!(registry_record.filled_amount.is_zero());

        // Test case 2: Keyring error
        // Expected:
        // - KeyringClient called to generate address
        // - Error propagated
        // - DAO not called
        // - Registry not updated
        let params = CreateInvoiceParams {
            order_id: "order456".to_string(),
            amount: Decimal::new(5000, 2), // 50.00
            cart: InvoiceCart::empty(),
            redirect_url: "https://redirect.url".to_string(),
        };

        app_state.keyring
            .expect_generate_asset_hub_address()
            .once()
            .withf(|data|
                data.derivation_params.len() == 1
                && Uuid::from_str(&data.derivation_params[0]).is_ok()
            )
            .returning(move |_| Err(KeyringError::InvalidSeed));

        let result = app_state
            .create_invoice(params.clone())
            .await;

        assert!(matches!(result, Err(DaoInvoiceError::DatabaseError)));
        let registry_records_count = app_state.registry.invoices_count().await;
        assert_eq!(registry_records_count, 1); // Only the previous successful invoice is present

        // Test case 3: DAO error
        // Expected:
        // - KeyringClient called to generate address
        // - DAO called to create invoice
        // - Error propagated
        // - Registry not updated
        // - Previous registry entries remain
        let params = CreateInvoiceParams {
            order_id: "order789".to_string(),
            amount: Decimal::new(7500, 2), // 75.00
            cart: InvoiceCart::empty(),
            redirect_url: "https://redirect.url".to_string(),
        };

        let expected_create_invoice_data = {
            CreateInvoiceData {
                id: Uuid::new_v4(), // We can't predict this, so we'll match fields except ID
                order_id: params.order_id.clone(),
                amount: params.amount,
                cart: params.cart.clone(),
                redirect_url: params.redirect_url.clone(),
                asset_id: 1337.to_string(),
                asset_name: "USDC".to_string(),
                chain: ChainType::PolkadotAssetHub,
                payment_address: to_base58_string(account_id.0, 0),
                valid_till: Utc::now() + Duration::milliseconds(app_state.payments_config.account_lifetime_millis as i64),
            }
        };

        app_state.keyring
            .expect_generate_asset_hub_address()
            .once()
            .withf(|data|
                data.derivation_params.len() == 1
                && Uuid::from_str(&data.derivation_params[0]).is_ok()
            )
            .returning(move |_| Ok(bob_account_id_2.clone()));

        app_state.dao
            .expect_create_invoice()
            .once()
            .withf(move |data| compare_create_invoice_data(&expected_create_invoice_data, data))
            .returning(|_| Err(DaoInvoiceError::DatabaseError));

        let result = app_state
            .create_invoice(params)
            .await;

        assert!(matches!(result, Err(DaoInvoiceError::DatabaseError)));
        let registry_records_count = app_state.registry.invoices_count().await;
        assert_eq!(registry_records_count, 1); // Only the first successful invoice is present
    }
}

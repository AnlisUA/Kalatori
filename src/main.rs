mod chain;
mod chain_client;
mod configs;
mod dao;
mod error;
mod expiration_detector;
mod handlers;
mod legacy_types;
mod server;
mod sled_to_sqlite_migration;
mod state;
mod types;
mod utils;

use std::process::ExitCode;
use std::str::FromStr;

use subxt::utils::AccountId32;
use tokio::runtime::Runtime;
use tokio_util::sync::CancellationToken;
use tracing::Level;

use chain_client::Keyring;
use configs::{
    ChainConfig,
    DatabaseConfig,
    chain_config_with_prefix,
    database_config_with_prefix,
    payments_config_with_prefix,
    seed_config_with_prefix,
    web_server_config_with_prefix,
};
use dao::DAO;
use error::{
    Error,
    PrettyCause,
};
use expiration_detector::ExpirationDetector;
use legacy_types::{
    ConfigWoChains,
    CurrencyProperties,
    Timestamp,
    TokenKind,
};
use state::State;
use utils::logger;
use utils::shutdown::{
    self,
    ShutdownNotification,
    ShutdownOutcome,
};
use utils::task_tracker::TaskTracker;

use crate::chain::{TransfersExecutor, TransfersTracker, InvoiceRegistry};
use crate::chain_client::{
    AssetHubClient,
    BlockChainClient,
};

const DEFAULT_ENV_PREFIX: &str = "KALATORI";

fn main() -> ExitCode {
    let shutdown_notification = ShutdownNotification::new();

    // Sets the panic hook to print directly to the standard error because the
    // logger isn't initialized yet.
    shutdown::set_panic_hook(
        |panic| eprintln!("{panic}"),
        shutdown_notification.clone(),
    );

    let result = try_main(shutdown_notification.clone());

    if let Err(error) = result {
        // TODO: https://github.com/rust-lang/rust/issues/92698
        // An equilibristic to conditionally print an error message without storing it
        // as `String` on the heap.
        let print = |message| {
            if tracing::event_enabled!(Level::ERROR) {
                tracing::error!("{message}");
            } else {
                eprintln!("{message}");
            }
        };

        print(format_args!(
            "Badbye! The daemon's got an error during the initialization:{}",
            error.pretty_cause()
        ));

        ExitCode::FAILURE
    } else {
        match *shutdown_notification
            .outcome
            .read_blocking()
        {
            ShutdownOutcome::UserRequested => {
                tracing::info!("Goodbye!");

                ExitCode::SUCCESS
            },
            ShutdownOutcome::UnrecoverableError {
                panic,
            } => {
                tracing::error!(
                    "Badbye! The daemon's shut down with errors{}.",
                    if panic { " due to internal bugs" } else { "" }
                );

                ExitCode::FAILURE
            },
        }
    }
}

fn try_main(shutdown_notification: ShutdownNotification) -> Result<(), Error> {
    logger::initialize("")?;
    shutdown::set_panic_hook(
        |panic| tracing::error!("{panic}"),
        shutdown_notification.clone(),
    );

    tracing::info!(
        "Kalatori {} is starting...",
        env!("CARGO_PKG_VERSION")
    );

    Runtime::new()
        .map_err(Error::Runtime)?
        .block_on(async_try_main(shutdown_notification))
}

/// Build a simplified currencies `HashMap` from `ChainConfig` for migration
/// purposes. Note: This uses placeholder values for decimals since we don't
/// have blockchain connection yet. The actual decimals are fetched
/// asynchronously by `ChainTracker` during normal operation.
fn build_currencies_from_config(
    chain_config: &ChainConfig
) -> std::collections::HashMap<String, CurrencyProperties> {
    let mut currencies = std::collections::HashMap::new();
    let rpc_url = chain_config
        .endpoints
        .first()
        .cloned()
        .unwrap_or_default();

    for asset in &chain_config.assets {
        let properties = CurrencyProperties {
            chain_name: chain_config.name.clone(),
            kind: TokenKind::Asset,
            decimals: 6, // Placeholder - not used during migration validation
            rpc_url: rpc_url.clone(),
            asset_id: Some(asset.id),
            ss58: 2, // Placeholder - not used during migration
        };

        currencies.insert(asset.name.clone(), properties);
    }

    currencies
}

async fn perform_sled_to_sqlite_migration(
    database_config: &DatabaseConfig,
    chain_config: &ChainConfig,
    dao: &DAO,
) -> Result<(), Error> {
    // Run sled to SQLite migration if sled database exists
    if !database_config.temporary {
        let sled_path = std::path::PathBuf::from(&database_config.path);
        if sled_path.exists() {
            tracing::info!(
                "Found sled database at {:?}, running migration to SQLite...",
                sled_path
            );

            let currencies = build_currencies_from_config(chain_config);

            match sled_to_sqlite_migration::migrate_sled_to_sqlite(sled_path, dao, &currencies)
                .await
            {
                Ok(stats) => {
                    tracing::info!(
                        "Migration completed successfully: {} invoices ({} skipped as existing), \
                            {} finalized transactions, {} pending transactions, {} duplicate transactions skipped, \
                            server_info migrated: {}",
                        stats.invoices_migrated,
                        stats.invoices_skipped_existing,
                        stats.finalized_transactions_migrated,
                        stats.pending_transactions_migrated,
                        stats.transactions_skipped_duplicates,
                        stats.server_info_migrated
                    );

                    if !stats.warnings.is_empty() {
                        tracing::warn!(
                            "Migration completed with {} warnings:",
                            stats.warnings.len()
                        );
                        for warning in &stats.warnings {
                            tracing::warn!("  - {}", warning);
                        }
                    }
                },
                Err(e) => {
                    return Err(Error::MigrationFailed(e));
                },
            }
        } else {
            tracing::debug!(
                "No sled database found at {:?}, skipping migration",
                sled_path
            );
        }
    }

    Ok(())
}

async fn async_try_main(shutdown_notification: ShutdownNotification) -> Result<(), Error> {
    // Planned start order
    // 1. Load configs
    // 2. Init database
    // 3. Load data from old database to the new one
    // 4. Get info about required chains and assets from database and configs
    // 5. Start keyring (background task)
    // 6. Start chain monitoring (incoming transactions, background task)
    // 7. Fetch balances of "pending" payments, ensure balance equals to expected
    //    (can be made in background)
    //  7.1 If balance > sum(related transactions amount), fetch related
    // transactions using API and update Invoice statuses respectively
    // 8. Start payments executor (background task)
    // 9. Start API (background task)
    let env_prefix =
        std::env::var("KALATORI_APP_ENV_PREFIX").unwrap_or_else(|_| DEFAULT_ENV_PREFIX.to_string());

    let configs_path = std::env::var(format!("{env_prefix}_CONFIG_DIR_PATH")).unwrap_or_default();

    let seed_config = seed_config_with_prefix(&configs_path, &env_prefix);
    let chain_config = chain_config_with_prefix(&configs_path, &env_prefix);
    let payments_config = payments_config_with_prefix(&configs_path, &env_prefix);
    let web_server_config = web_server_config_with_prefix(&configs_path, &env_prefix);
    let database_config = database_config_with_prefix(&configs_path, &env_prefix);

    // Start services
    let (task_tracker, error_rx) = TaskTracker::new();

    // TODO: replace with expect?
    let recipient = AccountId32::from_str(&payments_config.recipient).unwrap();

    // Initialize DAO for SQLite database operations
    let dao = DAO::new(database_config.clone())
        .await
        .map_err(error::DaoError::Sqlx)?;

    perform_sled_to_sqlite_migration(&database_config, &chain_config, &dao)
        .await
        .unwrap();

    let instance_id = dao
        .initialize_server_info()
        .await
        .map_err(error::DaoError::Sqlx)?;

    let assets: Vec<_> = chain_config
        .assets
        .iter()
        .map(|config| config.id)
        .collect();

    let asset_hub_client = AssetHubClient::new(&chain_config)
        .await
        .map_err(|_| Error::Fatal)?;

    asset_hub_client
        .init_asset_info(&assets)
        .await
        .map_err(|_| Error::Fatal)?;

    let keyring = Keyring::new(seed_config.seed);
    // Please don't keep keyring_client in this scope, it must be moved in order to keep
    // graceful shutdown working.
    let (keyring_handle, keyring_client) = keyring.ignite();

    let invoice_registry = InvoiceRegistry::new();

    let expiration_detector = ExpirationDetector::new(
        dao.clone(),
        invoice_registry.clone(),
    );

    let expiration_detector_handle = expiration_detector.ignite(shutdown_notification.token.clone());

    let transfers_tracker = TransfersTracker::new(
        asset_hub_client.clone(),
        dao.clone(),
        // TODO: fill registry with invoices from database
        invoice_registry.clone(),
        payments_config.clone(),
    );

    let transfers_tracker_handle = transfers_tracker.ignite(assets, shutdown_notification.token.clone());

    let transfer_executor = TransfersExecutor::new(
        asset_hub_client,
        dao.clone(),
        keyring_client.clone(),
    );

    let transfer_executor_handle = transfer_executor.ignite(shutdown_notification.token.clone());

    let currencies = build_currencies_from_config(&chain_config);

    let state = State::initialise(
        keyring_client,
        ConfigWoChains {
            recipient,
            remark: payments_config.remark,
            account_lifetime: Timestamp(payments_config.account_lifetime_millis),
        },
        dao,
        invoice_registry,
        currencies,
        instance_id,
        task_tracker.clone(),
        shutdown_notification.token.clone(),
    );

    let server = server::new(
        shutdown_notification.token.clone(),
        web_server_config,
        state.interface(),
    )
    .await?;

    task_tracker.spawn("the server module", server);

    let shutdown_completed = CancellationToken::new();
    let mut shutdown_listener = tokio::spawn(shutdown::listener(
        shutdown_notification.token.clone(),
        shutdown_completed.clone(),
    ));

    tracing::info!("The initialization has been completed.");

    // Start the main loop and wait for it to gracefully end or the early
    // termination signal.
    tokio::select! {
        biased;
        () = task_tracker.wait_and_shutdown(error_rx, shutdown_notification) => {
            shutdown_completed.cancel();

            let (
                shutdown_result,
                _keyring_result,
                _transfer_executor_result,
                _expiration_detector_result,
                _transfers_tracker_result,
            ) = tokio::join!(
                shutdown_listener,
                keyring_handle,
                transfer_executor_handle,
                expiration_detector_handle,
                transfers_tracker_handle,
            );

            shutdown_result
        }
        shutdown_listener_result = &mut shutdown_listener => shutdown_listener_result
    }
        .expect("shutdown listener shouldn't panic")
}

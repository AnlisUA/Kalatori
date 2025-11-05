use std::{process::ExitCode, str::FromStr};
use subxt::utils::AccountId32;
use tokio::{runtime::Runtime, sync::oneshot};
use tokio_util::sync::CancellationToken;
use tracing::Level;
use utils::{
    logger,
    shutdown::{self, ShutdownNotification, ShutdownOutcome},
    task_tracker::TaskTracker,
};

mod chain;
mod configs;
mod database;
mod definitions;
mod error;
mod handlers;
mod server;
mod signer;
mod state;
mod types;
mod utils;

use chain::ChainManager;
use database::ConfigWoChains;
use error::{Error, PrettyCause};
use signer::Signer;
use state::State;

use crate::{
    configs::{
        chain_config_with_prefix, database_config_with_prefix, payments_config_with_prefix,
        seed_config_with_prefix, web_server_config_with_prefix,
    },
    definitions::api_v2::Timestamp,
};

const DEFAULT_ENV_PREFIX: &str = "KALATORI";

fn main() -> ExitCode {
    let shutdown_notification = ShutdownNotification::new();

    // Sets the panic hook to print directly to the standard error because the logger isn't
    // initialized yet.
    shutdown::set_panic_hook(|panic| eprintln!("{panic}"), shutdown_notification.clone());

    let result = try_main(shutdown_notification.clone());

    if let Err(error) = result {
        // TODO: https://github.com/rust-lang/rust/issues/92698
        // An equilibristic to conditionally print an error message without storing it as `String`
        // on the heap.
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
        match *shutdown_notification.outcome.read_blocking() {
            ShutdownOutcome::UserRequested => {
                tracing::info!("Goodbye!");

                ExitCode::SUCCESS
            }
            ShutdownOutcome::UnrecoverableError { panic } => {
                tracing::error!(
                    "Badbye! The daemon's shut down with errors{}.",
                    if panic { " due to internal bugs" } else { "" }
                );

                ExitCode::FAILURE
            }
        }
    }
}

fn try_main(shutdown_notification: ShutdownNotification) -> Result<(), Error> {
    logger::initialize(logger::default_filter())?;
    shutdown::set_panic_hook(
        |panic| tracing::error!("{panic}"),
        shutdown_notification.clone(),
    );

    tracing::info!("Kalatori {} is starting...", env!("CARGO_PKG_VERSION"));

    Runtime::new()
        .map_err(Error::Runtime)?
        .block_on(async_try_main(shutdown_notification))
}

async fn async_try_main(shutdown_notification: ShutdownNotification) -> Result<(), Error> {
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

    // TODO: quite dirty hack to make it work right now. Should be refactored ASAP.
    // Spawn separate task for handling payouts. This task should replace Signer and store seed phrase
    let signer = Signer::init(recipient.clone(), &task_tracker, seed_config.seed.clone());
    let seed_secret = seed_config.seed;

    let db = database::Database::init(
        database_config,
        &task_tracker,
        Timestamp(payments_config.account_lifetime_millis),
    )?;

    let instance_id = db.initialize_server_info().await?;

    let (cm_tx, cm_rx) = oneshot::channel();

    let state = State::initialise(
        signer.interface(),
        ConfigWoChains {
            recipient,
            remark: payments_config.remark,
        },
        db,
        cm_rx,
        instance_id,
        task_tracker.clone(),
        shutdown_notification.token.clone(),
    );

    cm_tx
        .send(ChainManager::ignite(
            seed_secret,
            chain_config,
            &state,
            &task_tracker,
            &shutdown_notification.token,
        )?)
        .map_err(|_| Error::Fatal)?;

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

    // Start the main loop and wait for it to gracefully end or the early termination signal.
    tokio::select! {
        biased;
        () = task_tracker.wait_and_shutdown(error_rx, shutdown_notification) => {
            shutdown_completed.cancel();

            shutdown_listener.await
        }
        shutdown_listener_result = &mut shutdown_listener => shutdown_listener_result
    }
    .expect("shutdown listener shouldn't panic")
}

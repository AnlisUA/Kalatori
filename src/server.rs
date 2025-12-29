use std::borrow::Cow;
use std::future::Future;
use std::net::SocketAddr;

use axum::{
    Router,
    routing,
};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::configs::WebServerConfig;
use crate::dao::DaoInterface;
use crate::error::{
    Error,
    ServerError,
};
use crate::handlers::health::{
    health,
    status,
};
use crate::handlers::order::{
    force_withdrawal,
    order,
};
use crate::state::AppState;
use crate::handlers::public::public_routes;
use crate::legacy_types::LegacyApiData;

#[derive(Clone)]
pub struct ApiState<D: DaoInterface + Clone + 'static> {
    pub state: AppState<D>,
    pub legacy_api_data: LegacyApiData,
}

pub async fn new<D: DaoInterface + Clone + 'static>(
    shutdown_notification: CancellationToken,
    config: WebServerConfig,
    app_state: AppState<D>,
    legacy_api_data: LegacyApiData,
) -> Result<impl Future<Output = Result<Cow<'static, str>, Error>>, ServerError> {
    let api_state = ApiState {
        state: app_state,
        legacy_api_data,
    };

    let host = SocketAddr::new(config.host, config.port);

    let v2 = Router::new()
        .route("/order/:order_id", routing::post(order))
        .route(
            "/order/:order_id/forceWithdrawal",
            routing::post(force_withdrawal),
        )
        .route("/status", routing::get(status))
        .route("/health", routing::get(health));

    let app = Router::new()
        .nest("/public", public_routes())
        .nest("/v2", v2)
        .with_state(api_state);

    let listener = TcpListener::bind(host)
        .await
        .map_err(|_| ServerError::TcpListenerBind(host))?;

    Ok(async move {
        tracing::info!("The server is listening on {host}.");

        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_notification.cancelled_owned())
            .await
            .map_err(|_| ServerError::ThreadError)?;

        Ok("The server module is shut down.".into())
    })
}

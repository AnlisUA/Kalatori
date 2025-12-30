use std::borrow::Cow;
use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;

use axum::extract::rejection::RawPathParamsRejection;
use axum::extract::{
    self,
    MatchedPath,
    Query,
    RawPathParams,
};
use axum::response::Response;
use axum::{
    Router,
    routing,
};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::configs::WebServerConfig;
use crate::error::{
    Error,
    ServerError,
};
use crate::handlers::health::{
    audit,
    health,
    status,
};
use crate::handlers::order::{
    force_withdrawal,
    investigate,
    order,
};
use crate::state::State;
use crate::handlers::public::public_routes;

pub async fn new(
    shutdown_notification: CancellationToken,
    config: WebServerConfig,
    state: State,
) -> Result<impl Future<Output = Result<Cow<'static, str>, Error>>, ServerError> {
    let host = SocketAddr::new(config.host, config.port);

    let v2: Router<State> = Router::new()
        .route("/order/:order_id", routing::post(order))
        .route(
            "/order/:order_id/forceWithdrawal",
            routing::post(force_withdrawal),
        )
        .route("/status", routing::get(status))
        .route("/health", routing::get(health))
        .route("/audit", routing::get(audit))
        .route(
            "/order/:order_id/investigate",
            routing::post(investigate),
        );

    let app = Router::new()
        .route(
            "/public/v2/payment/:paymentAccount",
            routing::post(public_payment_account),
        )
        .nest("/public", public_routes())
        .nest("/v2", v2)
        .with_state(state);

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

// TODO: Clarify what this is doing
// #[debug_handler]
async fn public_payment_account(
    extract::State(_state): extract::State<State>,
    _matched_path: MatchedPath,
    _path_result: Result<RawPathParams, RawPathParamsRejection>,
    _query: Query<HashMap<String, String>>,
) -> Response {
    todo!()
}

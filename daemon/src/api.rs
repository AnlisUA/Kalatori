mod dev;
mod private;
mod public;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::http::{HeaderName, Method};
use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;
use tower_http::request_id::{SetRequestIdLayer, PropagateRequestIdLayer, MakeRequestUuid};
use tower_http::cors::{CorsLayer, Any};

use crate::configs::WebServerConfig;
use crate::dao::DaoInterface;
use crate::state::AppState;

use dev::dev_routes;
use private::private_routes;
use public::public_routes;

pub type ApiState<D> = Arc<AppState<D>>;

const REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");

pub async fn api_server<D: DaoInterface>(
    config: WebServerConfig,
    state: AppState<D>,
    cancellation_token: tokio_util::sync::CancellationToken,
) -> impl std::future::Future<Output = ()> {
    let api_state = Arc::new(state);

    let host = SocketAddr::new(config.host, config.port);

    let listener = TcpListener::bind(host)
        .await
        .expect("Failed to bind to address");

    let router = axum::Router::new()
        .nest("/dev", dev_routes())
        .nest("/private", private_routes())
        .nest("/public", public_routes())
        .layer(
            tower::ServiceBuilder::new()
                .layer(
                    SetRequestIdLayer::new(
                        REQUEST_ID_HEADER,
                        MakeRequestUuid,
                    )
                )
                .layer(
                    TraceLayer::new_for_http()
                        .make_span_with(|request: &axum::http::Request<_>| {
                            let request_id = request
                                .headers()
                                .get(REQUEST_ID_HEADER)
                                .and_then(|v| v.to_str().ok())
                                .unwrap_or("-");

                            tracing::info_span!(
                                "HTTP Request",
                                method = %request.method(),
                                path = %request.uri().path(),
                                request_id = %request_id,
                            )
                        })
                )
                .layer(
                    PropagateRequestIdLayer::new(REQUEST_ID_HEADER)
                )
                .layer(
                    CorsLayer::new()
                        .allow_methods([Method::GET, Method::POST])
                        .allow_origin(Any)
                )
        )
        .with_state(api_state);

    async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(cancellation_token.cancelled_owned())
            .await
            .unwrap();
    }
}

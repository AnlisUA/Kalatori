/// API server implementation
///
/// API namespaces:
/// - `/public`: Publicly accessible endpoints that do not require authentication. Should return only sanitized data
/// without sensitive information and details about the internal state.
/// - `/private`: Endpoints that require authentication and are intended for internal use. Should return only sanitized
/// data without sensitive information and details about the internal state.
/// - `/dev`: Development and testing endpoints. May include endpoints that are not intended for production use. Allowed
/// to return raw data including sensitive information and internal state details for debugging purposes. Should not be
/// exposed in production environments.
///
/// Error handling principles:
/// - For invalid or malformed JSON, query parameters, or request structure, return structured JSON error response.
/// - For authentication errors, return structured JSON error response.
/// - For application-level errors (e.g., entity not found, validation errors), return structured JSON error response.
/// - For unexpected server errors, return structured JSON error response with a generic message.
/// - For invalid routes or methods under `/private` and `/dev` namespaces, return structured JSON error response,
/// while `/public` namespace returns standard 404 HTML response.

#[cfg(feature = "dev_api")]
mod dev;
mod private;
mod public;
mod utils;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::http::{HeaderName, Method, StatusCode};
use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;
use tower_http::request_id::{SetRequestIdLayer, PropagateRequestIdLayer, MakeRequestUuid};
use tower_http::cors::{CorsLayer, Any};
use secrecy::{SecretString, ExposeSecret};
use zeroize::Zeroize;

use kalatori_client::types::ApiError;
use kalatori_client::utils::HmacConfig;

use crate::configs::WebServerConfig;
use crate::dao::DaoInterface;
use crate::state::AppState;

pub type ApiState<D> = Arc<AppState<D>>;

const REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");

pub trait ApiErrorExt: std::error::Error {
    fn category(&self) -> &str;
    fn code(&self) -> &str;
    fn message(&self) -> &str;
    fn http_status_code(&self) -> StatusCode;

    fn into_api_error(&self) -> ApiError {
        ApiError {
            category: self.category().to_string(),
            code: self.code().to_string(),
            message: self.message().to_string(),
        }
    }
}

#[cfg(not(feature = "dev_api"))]
mod dev {
    pub fn routes<D: super::DaoInterface>() -> axum::Router<super::ApiState<D>> {
        axum::Router::new()
    }
}

pub async fn api_server<D: DaoInterface>(
    config: WebServerConfig,
    mut api_secret_key: SecretString,
    state: AppState<D>,
    cancellation_token: tokio_util::sync::CancellationToken,
) -> impl std::future::Future<Output = ()> {
    let api_state = Arc::new(state);
    let hmac_config = HmacConfig::new(api_secret_key.expose_secret().as_bytes().to_vec(), 6000);
    api_secret_key.zeroize();

    let host = SocketAddr::new(config.host, config.port);

    let listener = TcpListener::bind(host)
        .await
        .expect("Failed to bind to address");

    let router = axum::Router::new()
        .nest("/dev", dev::routes())
        .nest("/private", private::routes(hmac_config))
        .nest("/public", public::routes())
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

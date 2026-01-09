use crate::dao::DaoInterface;

use axum::routing::{get, post};

use super::middlewares::{HmacConfig, hmac_validator};

use super::ApiState;

#[derive(serde::Deserialize)]
struct Params {
    a: String,
    b: String,
}

#[tracing::instrument(skip(_params))]
async fn test_get(
    axum::extract::Query(_params): axum::extract::Query<Params>,
) -> &'static str {
    tracing::info!("Inside private GET route");
    "Private route accessed"
}

#[tracing::instrument(skip(_params))]
async fn test_post(
    axum::extract::Json(_params): axum::extract::Json<Params>,
) -> &'static str {
    tracing::info!("Inside private POST route");
    "Private POST route accessed"
}

pub fn private_routes<D: DaoInterface>() -> axum::Router<ApiState<D>> {
    let config = HmacConfig::new("secret", 6000);

    axum::Router::new()
        .route("/test-get", get(test_get))
        .route("/test-post", post(test_post))
        .layer(axum::middleware::from_fn_with_state(config, hmac_validator))
}

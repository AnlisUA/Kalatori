use crate::dao::DaoInterface;

use axum::routing::{get, post};
use axum::extract::{State, Json};
use rust_decimal::Decimal;

use kalatori_client::utils::HmacConfig;
use kalatori_client::middleware::axum_hmac_validator;
use kalatori_client::types::{CreateInvoiceParams, GetInvoiceParams, Invoice};

use crate::types::InvoiceWithIncomingAmount;

use super::ApiState;

#[tracing::instrument(skip(state))]
async fn create_invoice<D: DaoInterface>(
    State(state): State<ApiState<D>>,
    Json(params): Json<CreateInvoiceParams>,
) -> Json<Invoice> {
    let invoice = state
        .create_invoice(params)
        .await
        .unwrap();

    let with_amount = InvoiceWithIncomingAmount {
        invoice,
        incoming_amount: Decimal::ZERO,
    };

    let result = state.build_public_invoice(with_amount);
    Json(result)
}

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
        .route("/v3/invoice/create", post(create_invoice))
        .layer(axum::middleware::from_fn_with_state(config, axum_hmac_validator))
}

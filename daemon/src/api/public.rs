use axum::Json;
use axum::extract::{
    Query,
    State as ExtractState,
};
use axum::http::StatusCode;
use axum::response::{
    Html,
    IntoResponse,
    Response,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::swaps::SwapsExecutorError;
use crate::types::{CreateOneInchSwapParams, GetPricesParams, GetPricesResponse, PublicOneInchPreparedSwap, PublicOneInchSwap, SubmitOneInchSwapParams};

use super::ApiState;
use super::utils::{
    ApiResult,
    AppJson,
    AppQuery,
};

#[derive(Debug, PartialEq, Eq, Deserialize)]
struct IndexParams {
    #[serde(default)]
    invoice_id: String,
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
struct Params {
    invoice_id: Uuid,
}

async fn index(Query(params): Query<IndexParams>) -> Html<String> {
    let raw_html = include_str!("../../../static/index.html");
    let html = raw_html.replace("{{INVOICE_ID}}", &params.invoice_id);
    Html(html)
}

async fn invoice(
    ExtractState(state): ExtractState<ApiState>,
    Query(payload): Query<Params>,
) -> Response {
    let invoice = state
        .get_invoice(payload.invoice_id)
        .await;

    match invoice {
        // If the invoice exists and is active, return it
        Ok(Some(invoice)) if invoice.invoice.status.is_active() => {
            (StatusCode::OK, Json(invoice)).into_response()
        },
        // TODO: update errors
        // If the invoice does not exist or is not active, return 404
        Ok(Some(_) | None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Invoice not found"})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Internal server error: {}", e)})),
        )
            .into_response(),
    }
}

#[tracing::instrument(skip_all)]
async fn create_swap(
    ExtractState(state): ExtractState<ApiState>,
    AppJson(params): AppJson<CreateOneInchSwapParams>,
) -> ApiResult<PublicOneInchPreparedSwap, SwapsExecutorError> {
    let swap = state
        .create_swap(params)
        .await?
        .into_public();

    Ok(swap.into())
}

#[tracing::instrument(skip_all)]
async fn submit_swap(
    ExtractState(state): ExtractState<ApiState>,
    AppJson(params): AppJson<SubmitOneInchSwapParams>,
) -> ApiResult<PublicOneInchSwap, SwapsExecutorError> {
    let swap = state
        .submit_swap(params)
        .await?
        .into_public();

    Ok(swap.into())
}

#[tracing::instrument(skip_all)]
async fn get_prices(
    ExtractState(state): ExtractState<ApiState>,
    AppQuery(params): AppQuery<GetPricesParams>,
) -> ApiResult<GetPricesResponse, SwapsExecutorError> {
    let prices = state
        .get_prices(params)
        .await?;

    Ok(prices.into())
}

pub fn routes() -> axum::Router<ApiState> {
    axum::Router::new()
        .route("/", axum::routing::get(index))
        .route("/invoice", axum::routing::get(invoice))
        .route("/swap/create", axum::routing::post(create_swap))
        .route("/swap/submit", axum::routing::post(submit_swap))
        .route("/swap/get-prices", axum::routing::get(get_prices))
}

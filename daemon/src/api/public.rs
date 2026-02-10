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

use crate::configs::ShopMetaConfig;

use super::ApiState;
use super::utils::SuccessWrapper;

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

async fn shop_meta(ExtractState(state): ExtractState<ApiState>) -> SuccessWrapper<ShopMetaConfig> {
    state.get_shop_meta().into()
}

pub fn routes() -> axum::Router<ApiState> {
    axum::Router::new()
        .route("/", axum::routing::get(index))
        .route("/invoice", axum::routing::get(invoice))
        .route("/info", axum::routing::get(shop_meta))
}

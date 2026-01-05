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

use crate::dao::DaoInterface;
use crate::server::ApiState;

#[derive(Debug, PartialEq, Eq, Deserialize)]
struct Params {
    invoice_id: Uuid,
}

async fn index(Query(params): Query<Params>) -> Html<String> {
    let raw_html = include_str!("../../static/index.html");
    let html = raw_html.replace(
        "{{INVOICE_ID}}",
        &params.invoice_id.to_string(),
    );
    Html(html)
}

async fn invoice<D: DaoInterface + Clone + 'static>(
    ExtractState(state): ExtractState<ApiState<D>>,
    Query(payload): Query<Params>,
) -> Response {
    let invoice = state
        .state
        .get_invoice(payload.invoice_id)
        .await;

    match invoice {
        Ok(Some(invoice)) => (StatusCode::OK, Json(invoice)).into_response(),
        Ok(None) => (
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

pub fn public_routes<D: DaoInterface + Clone + 'static>() -> axum::Router<ApiState<D>> {
    axum::Router::new()
        .route("/v1", axum::routing::get(index))
        .route(
            "/v1/invoice",
            axum::routing::get(invoice),
        )
}

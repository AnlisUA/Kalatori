use crate::definitions::api_v2::{ServerHealth, ServerStatus};
use crate::state::State;
use axum::{extract::State as ExtractState, http::StatusCode, Json};

pub async fn status(
    ExtractState(state): ExtractState<State>,
) -> (
    [(axum::http::header::HeaderName, &'static str); 1],
    Json<ServerStatus>,
) {
    #[expect(clippy::match_wild_err_arm)]
    match state.server_status().await {
        Ok(status) => (
            [(axum::http::header::CACHE_CONTROL, "no-store")],
            Json(status),
        ),
        // TODO: change panic to something else? 
        // Probably this handler should return some error status in response and k8s must make a decision about killing it.
        // If we need behaviour of panic in case of db connection lost, it's better to do it in some background task, 
        // not in the status handler
        Err(_) => panic!("db connection is down, state is lost"), // You can modify this as needed
    }
}

pub async fn health(
    ExtractState(state): ExtractState<State>,
) -> (
    [(axum::http::header::HeaderName, &'static str); 1],
    Json<ServerHealth>,
) {
    #[expect(clippy::match_wild_err_arm)]
    match state.server_health().await {
        Ok(status) => (
            [(axum::http::header::CACHE_CONTROL, "no-store")],
            Json(status),
        ),
        // TODO: same as for status handler
        Err(_) => panic!("db connection is down, state is lost"),
    }
}

pub async fn audit(ExtractState(_state): ExtractState<State>) -> StatusCode {
    StatusCode::NOT_IMPLEMENTED
}

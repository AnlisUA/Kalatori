use axum::Json;
use axum::extract::State as ExtractState;

use crate::dao::DaoInterface;
use crate::server::ApiState;
use crate::legacy_types::{
    ServerHealth,
    ServerStatus,
    RpcInfo,
    Health,
};

pub async fn status<D: DaoInterface + Clone + 'static>(
    ExtractState(state): ExtractState<ApiState<D>>
) -> (
    [(
        axum::http::header::HeaderName,
        &'static str,
    ); 1],
    Json<ServerStatus>,
) {
    #[expect(clippy::match_wild_err_arm)]
    let status = ServerStatus {
        server_info: state.legacy_api_data.server_info.clone(),
        supported_currencies: state.legacy_api_data.currencies.clone(),
    };

    (
        [(
            axum::http::header::CACHE_CONTROL,
            "no-store",
        )],
        Json(status),
    )
}

fn overall_health(connected_rpcs: &[RpcInfo]) -> Health {
    if connected_rpcs
        .iter()
        .all(|rpc| rpc.status == Health::Ok)
    {
        Health::Ok
    } else if connected_rpcs
        .iter()
        .any(|rpc| rpc.status == Health::Ok)
    {
        Health::Degraded
    } else {
        Health::Critical
    }
}

pub async fn health<D: DaoInterface + Clone + 'static>(
    ExtractState(state): ExtractState<ApiState<D>>
) -> (
    [(
        axum::http::header::HeaderName,
        &'static str,
    ); 1],
    Json<ServerHealth>,
) {
    let connected_rpcs: Vec<_> = state.legacy_api_data.rpc_endpoints
        .iter()
        .map(|rpc_url| RpcInfo {
            chain_name: "statemint".to_string(),
            rpc_url: rpc_url.to_string(),
            status: Health::Ok,
        })
        .collect();

    let status = overall_health(&connected_rpcs);

    let server_health = ServerHealth {
        server_info: state.legacy_api_data.server_info.clone(),
        connected_rpcs,
        status,
    };

    (
        [(
            axum::http::header::CACHE_CONTROL,
            "no-store",
        )],
        Json(server_health),
    )
}

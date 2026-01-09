use crate::dao::DaoInterface;

use super::ApiState;

pub fn dev_routes<D: DaoInterface>() -> axum::Router<ApiState<D>> {
    axum::Router::new()
}

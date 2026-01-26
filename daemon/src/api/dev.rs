use super::ApiState;
use super::utils::{
    fallback_handler,
    method_not_allowed_fallback_handler,
};

pub fn routes() -> axum::Router<ApiState> {
    axum::Router::new()
        .fallback(fallback_handler)
        .method_not_allowed_fallback(method_not_allowed_fallback_handler)
}

use axum::http::StatusCode;
use axum::response::{Response, IntoResponse, Json};
use axum::extract::FromRequest;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use serde::Serialize;

use kalatori_client::types::{ApiResultStructured, ApiError};

use super::ApiErrorExt;

#[derive(Debug)]
pub(super) struct SuccessWrapper<T: Serialize>(T);

impl<T: Serialize> From<T> for SuccessWrapper<T> {
    fn from(value: T) -> Self {
        SuccessWrapper(value)
    }
}

impl <T: Serialize> IntoResponse for SuccessWrapper<T> {
    fn into_response(self) -> Response {
        (
            StatusCode::OK,
            Json(ApiResultStructured::Ok { result: self.0 })
        ).into_response()
    }
}

#[derive(Debug)]
pub(super) struct ErrorWrapper<E: ApiErrorExt>(E);

impl<E: ApiErrorExt> From<E> for ErrorWrapper<E> {
    fn from(value: E) -> Self {
        ErrorWrapper(value)
    }
}

impl <E: ApiErrorExt> IntoResponse for ErrorWrapper<E> {
    fn into_response(self) -> Response {
        (
            self.0.http_status_code(),
            Json(ApiResultStructured::<()>::Err { error: self.0.into_api_error() })
        ).into_response()
    }
}

#[derive(thiserror::Error, Debug)]
pub(super) enum AppExtractorError {
    #[error(transparent)]
    Json(#[from] JsonRejection),
    #[error(transparent)]
    Query(#[from] QueryRejection),
}

impl IntoResponse for AppExtractorError {
    fn into_response(self) -> Response {
        let api_error = match self {
            AppExtractorError::Json(rejection) => ApiError {
                // TODO: improve error codes and messages based on rejection reason
                category: "INVALID_REQUEST".to_string(),
                code: "INVALID_JSON".to_string(),
                message: format!("JSON extraction error: {}", rejection),
            },
            AppExtractorError::Query(rejection) => ApiError {
                category: "INVALID_REQUEST".to_string(),
                code: "INVALID_QUERY_PARAMS".to_string(),
                message: format!("Query extraction error: {}", rejection),
            },
        };

        (
            StatusCode::BAD_REQUEST,
            Json(ApiResultStructured::<()>::Err { error: api_error })
        ).into_response()
    }
}

#[derive(FromRequest)]
#[from_request(via(axum::extract::Json), rejection(AppExtractorError))]
pub(super) struct AppJson<T>(pub T);

#[derive(axum::extract::FromRequestParts)]
#[from_request(via(axum::extract::Query), rejection(AppExtractorError))]
pub(super) struct AppQuery<T>(pub T);

pub type ApiResult<T, E> = Result<SuccessWrapper<T>, ErrorWrapper<E>>;

pub(super) async fn fallback_handler() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json(ApiResultStructured::<()>::Err { error: ApiError {
            category: "INVALID_REQUEST".to_string(),
            code: "ROUTE_NOT_FOUND".to_string(),
            message: "The requested route was not found.".to_string(),
        }})
    )
}

pub(super) async fn method_not_allowed_fallback_handler() -> impl IntoResponse {
    (
        StatusCode::METHOD_NOT_ALLOWED,
        Json(ApiResultStructured::<()>::Err { error: ApiError {
            category: "INVALID_REQUEST".to_string(),
            code: "METHOD_NOT_ALLOWED".to_string(),
            message: "Only GET and POST methods are allowed.".to_string(),
        }})
    )
}

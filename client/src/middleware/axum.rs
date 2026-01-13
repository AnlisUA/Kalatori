use axum::middleware::Next;
use axum::extract::{Request, State, OriginalUri, Json};
use axum::response::{IntoResponse, Response};
use axum::body::Body;
use axum::http::StatusCode;

use crate::types::{ApiError, ApiResultStructured};
use crate::utils::HmacConfig;

use super::{HmacValidationError, validate_request};

impl IntoResponse for HmacValidationError {
    fn into_response(self) -> Response {
        let (status, category, code, message) = match self {
            Self::MissingSignature => (
                StatusCode::UNAUTHORIZED,
                "INVALID_REQUET",
                "SIGNATURE_HEADER_NOT_SET",
                "Header x-kalatori-signature should be set",
            ),
            Self::InvalidSignatureFormat => (
                StatusCode::BAD_REQUEST,
                "INVALID_REQUEST",
                "SIGNATURE_HEADER_INVALID_FORMAT",
                "Invalid signature format",
            ),
            Self::SignatureMismatch => (
                StatusCode::UNAUTHORIZED,
                "AUTHENTICATION_FAILED",
                "SIGNATURE_VERIFICATION_FAILED",
                "Signature verification failed",
            ),
            Self::MissingTimestamp => (
                StatusCode::BAD_REQUEST,
                "INVALID_REQUEST",
                "TIMESTAMP_HEADER_NOT_SET",
                "Header x-kalatori-timestamp should be set",
            ),
            Self::InvalidTimestampFormat => (
                StatusCode::BAD_REQUEST,
                "INVALID_REQUEST",
                "TIMESTAMP_HEADER_INVALID_FORMAT",
                "Invalid timestamp format",
            ),
            // TODO: add details about expiration time
            Self::RequestExpired { .. } => (
                StatusCode::UNAUTHORIZED,
                "AUTHENTICATION_FAILED",
                "REQUEST_SIGNATURE_EXPIRED",
                "Request signature expired",
            ),
            Self::BodyReadError => {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "INTERNAL_ERROR",
                    "INTERNAL_ERROR",
                    "Failed to read request body",
                )
            },
            Self::MethodNotAllowed => {
                (
                    StatusCode::METHOD_NOT_ALLOWED,
                    "INVALID_REQUEST",
                    "METHOD_NOT_ALLOWED",
                    "Only GET and POST methods are allowed",
                )
            },
        };

        let error = ApiError {
            category: category.to_string(),
            code: code.to_string(),
            message: message.to_string(),
        };

        (status, Json(ApiResultStructured::<()>::Err { error })).into_response()
    }
}

pub async fn axum_hmac_validator(
    State(config): State<HmacConfig>,
    OriginalUri(original_uri): OriginalUri,
    request: Request,
    next: Next,
) -> Result<Response, HmacValidationError> {
    let (parts, body) = request.into_parts();

    let body_bytes = axum::body::to_bytes(body, usize::MAX)
        .await
        .map_err(|_| HmacValidationError::BodyReadError)?;

    validate_request(
        &config,
        original_uri,
        &parts.method,
        &parts.headers,
        &body_bytes,
    )?;

    // Reconstruct the request with the body
    let reconstructed_request = Request::from_parts(parts, Body::from(body_bytes));

    Ok(next.run(reconstructed_request).await)
}

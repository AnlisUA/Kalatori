use axum::middleware::Next;
use axum::extract::{Request, State, OriginalUri};
use axum::response::{IntoResponse, Response};
use axum::body::Body;
use axum::http::StatusCode;

use crate::utils::HmacConfig;

use super::{HmacValidationError, validate_request};

impl IntoResponse for HmacValidationError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::MissingSignature => (StatusCode::UNAUTHORIZED, "Missing signature header"),
            Self::InvalidSignatureFormat => (StatusCode::BAD_REQUEST, "Invalid signature format"),
            Self::SignatureMismatch => (StatusCode::UNAUTHORIZED, "Signature verification failed"),
            Self::MissingTimestamp => (StatusCode::BAD_REQUEST, "Missing timestamp header"),
            Self::InvalidTimestampFormat => (StatusCode::BAD_REQUEST, "Invalid timestamp format"),
            Self::RequestExpired { age_seconds, max_age } => {
                return (
                    StatusCode::UNAUTHORIZED,
                    format!("Request expired: age {age_seconds}s exceeds max {max_age}s"),
                )
                    .into_response();
            },
            Self::BodyReadError => {
                (StatusCode::INTERNAL_SERVER_ERROR, "Failed to read request body")
            },
            Self::MethodNotAllowed => {
                (StatusCode::METHOD_NOT_ALLOWED, "Only GET and POST methods are allowed")
            },
        };

        (status, message).into_response()
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

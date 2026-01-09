use std::sync::Arc;

use axum::{
    body::Body,
    extract::Request,
    http::{HeaderMap, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use axum::http::Method;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use tracing::debug;

/// HMAC-SHA256 signature validator
type HmacSha256 = Hmac<Sha256>;

const SIGNATURE_HEADER: &str = "X-KALATORI-SIGNATURE";
const TIMESTAMP_HEADER: &str = "X-KALATORI-TIMESTAMP";

/// Configuration for HMAC validation middleware
#[derive(Clone)]
pub struct HmacConfig {
    /// The secret key used for HMAC calculation
    secret_key: Arc<[u8]>,
    /// Maximum age of the request in seconds (prevents replay attacks)
    max_age_seconds: u64,
}

impl HmacConfig {
    pub fn new(secret_key: impl AsRef<[u8]>, max_age_seconds: u64) -> Self {
        Self {
            secret_key: Arc::from(secret_key.as_ref()),
            max_age_seconds,
        }
    }
}

/// Error type for HMAC validation failures
#[derive(Debug)]
pub enum HmacValidationError {
    /// Signature header is missing
    MissingSignature,
    /// Signature format is invalid (not valid hex)
    InvalidSignatureFormat,
    /// Signature does not match calculated HMAC
    SignatureMismatch,
    /// Timestamp header is missing (when timestamp validation is enabled)
    MissingTimestamp,
    /// Timestamp format is invalid
    InvalidTimestampFormat,
    /// Request is too old (timestamp validation)
    RequestExpired { age_seconds: u64, max_age: u64 },
    /// Failed to read request body
    BodyReadError,
    /// Method not allowed (only GET and POST supported)
    MethodNotAllowed,
}

impl IntoResponse for HmacValidationError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::MissingSignature => (StatusCode::UNAUTHORIZED, "Missing signature header"),
            Self::InvalidSignatureFormat => (StatusCode::BAD_REQUEST, "Invalid signature format"),
            Self::SignatureMismatch => (StatusCode::UNAUTHORIZED, "Signature verification failed"),
            Self::MissingTimestamp => (StatusCode::BAD_REQUEST, "Missing timestamp header"),
            Self::InvalidTimestampFormat => (StatusCode::BAD_REQUEST, "Invalid timestamp format"),
            Self::RequestExpired { age_seconds, max_age } => {
                debug!(
                    age_seconds = age_seconds,
                    max_age = max_age,
                    "Request expired due to timestamp"
                );
                return (
                    StatusCode::UNAUTHORIZED,
                    format!("Request expired: age {age_seconds}s exceeds max {max_age}s"),
                )
                    .into_response();
            },
            Self::BodyReadError => {
                (StatusCode::INTERNAL_SERVER_ERROR, "Failed to read request body")
            },
            Self::MethodNotAllowed => {  // ✅ ADD THIS
                (StatusCode::METHOD_NOT_ALLOWED, "Only GET and POST methods are allowed")
            },
        };

        (status, message).into_response()
    }
}

/// Extracts a header value as a string
fn extract_header<'a>(headers: &'a HeaderMap<HeaderValue>, name: &str) -> Option<&'a str> {
    headers.get(name)?.to_str().ok()
}

/// Validates timestamp and checks if request is not expired
fn validate_timestamp(
    timestamp_str: &str,
    max_age_seconds: u64,
) -> Result<(), HmacValidationError> {
    let timestamp: u64 = timestamp_str
        .parse()
        .map_err(|_| HmacValidationError::InvalidTimestampFormat)?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("System time before UNIX epoch")
        .as_secs();

    let age = now.saturating_sub(timestamp);

    if age > max_age_seconds {
        return Err(HmacValidationError::RequestExpired {
            age_seconds: age,
            max_age: max_age_seconds,
        });
    }

    Ok(())
}

fn sorted_query_string(query: &str) -> String {
    let mut pairs: Vec<(&str, &str)> = query
        .split('&')
        .filter_map(|pair| {
            let mut split = pair.splitn(2, '=');
            let key = split.next()?;
            let value = split.next().unwrap_or("");
            Some((key, value))
        })
        .collect();

    pairs.sort_by(|a, b| a.0.cmp(b.0));

    pairs
        .into_iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("&")
}

/// Calculates HMAC-SHA256
fn calculate_hmac(
    secret_key: &[u8],
    method: &str,
    path: &str,
    body_or_query: &[u8],
    timestamp: &str,
) -> Hmac<Sha256> {
    let mut mac = HmacSha256::new_from_slice(secret_key)
        .expect("HMAC can take key of any size");

    mac.update(method.as_bytes());
    mac.update(b"\n");
    mac.update(path.as_bytes());
    mac.update(b"\n");
    mac.update(body_or_query);
    mac.update(b"\n");
    mac.update(timestamp.as_bytes());

    mac
}

#[tracing::instrument(skip_all)]
pub async fn hmac_validator(
    axum::extract::State(config): axum::extract::State<HmacConfig>,
    axum::extract::OriginalUri(original_uri): axum::extract::OriginalUri,
    request: Request,
    next: Next,
) -> Result<Response, HmacValidationError> {
    let (parts, body) = request.into_parts();

    // Extract and validate signature header
    let signature_hex = extract_header(&parts.headers, SIGNATURE_HEADER)
        .ok_or(HmacValidationError::MissingSignature)?;

    // Decode signature from hex
    let expected_signature = const_hex::decode(signature_hex)
        .map_err(|_| HmacValidationError::InvalidSignatureFormat)?;

    // Extract timestamp
    let timestamp_value = extract_header(&parts.headers, TIMESTAMP_HEADER)
        .ok_or(HmacValidationError::MissingTimestamp)?;

    // Validate timestamp
    validate_timestamp(timestamp_value, config.max_age_seconds)?;

    let body_bytes = axum::body::to_bytes(body, usize::MAX)
        .await
        .map_err(|_| HmacValidationError::BodyReadError)?;

    let uri = parts.uri.query().unwrap_or("");
    let sorted_uri = sorted_query_string(uri);

    let query_or_body = match parts.method {
        Method::GET => sorted_uri.as_bytes(),
        Method::POST => body_bytes.as_ref(),
        _ => {
            return Err(HmacValidationError::MethodNotAllowed);
        }
    };

    let method = parts.method.as_str().to_uppercase();
    let path = original_uri.path();

    // Calculate HMAC
    let calculated_signature = calculate_hmac(
        &config.secret_key,
        &method,
        path,
        &query_or_body,
        timestamp_value,
    );

    calculated_signature.verify_slice(expected_signature.as_slice())
        .map_err(|_| HmacValidationError::SignatureMismatch)?;

    debug!("HMAC signature validated successfully");

    // Reconstruct the request with the body
    let reconstructed_request = Request::from_parts(parts, Body::from(body_bytes));

    Ok(next.run(reconstructed_request).await)
}

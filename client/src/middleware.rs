#[cfg(feature = "axum-middleware")]
mod axum;

use hmac::Mac;
use http::{
    HeaderMap,
    HeaderValue,
    Method,
    Uri,
};

use crate::utils::{
    HmacConfig,
    HmacSha256,
    SIGNATURE_HEADER,
    TIMESTAMP_HEADER,
    hmac_from_request_parts,
    timestamp_secs,
};

#[cfg(feature = "axum-middleware")]
pub use axum::axum_hmac_validator;

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

/// Extracts a header value as a string
fn extract_header<'a>(
    headers: &'a HeaderMap<HeaderValue>,
    name: &str,
) -> Option<&'a str> {
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

    let now = timestamp_secs();
    let age = now.saturating_sub(timestamp);

    if age > max_age_seconds {
        return Err(HmacValidationError::RequestExpired {
            age_seconds: age,
            max_age: max_age_seconds,
        });
    }

    Ok(())
}

fn validate_timestamp_and_signature(
    config: &HmacConfig,
    hmac: HmacSha256,
    timestamp: &str,
    expected_signature: &[u8],
) -> Result<(), HmacValidationError> {
    validate_timestamp(timestamp, config.max_age_seconds)?;

    hmac.verify_slice(expected_signature)
        .map_err(|_| HmacValidationError::SignatureMismatch)?;

    Ok(())
}

pub(crate) fn validate_request(
    config: &HmacConfig,
    uri: &Uri,
    method: &Method,
    headers: &HeaderMap<HeaderValue>,
    body_bytes: &[u8],
) -> Result<(), HmacValidationError> {
    // Extract and validate signature header
    let signature_hex =
        extract_header(headers, SIGNATURE_HEADER).ok_or(HmacValidationError::MissingSignature)?;

    // Decode signature from hex
    let expected_signature = const_hex::decode(signature_hex)
        .map_err(|_| HmacValidationError::InvalidSignatureFormat)?;

    // Extract timestamp
    let timestamp =
        extract_header(headers, TIMESTAMP_HEADER).ok_or(HmacValidationError::MissingTimestamp)?;

    let path = uri.path();
    let query_params = uri.query();

    let hmac = hmac_from_request_parts(
        config,
        method,
        path,
        query_params,
        body_bytes,
        timestamp,
    )
    .ok_or(HmacValidationError::MethodNotAllowed)?;

    validate_timestamp_and_signature(
        config,
        hmac,
        timestamp,
        &expected_signature,
    )?;

    Ok(())
}

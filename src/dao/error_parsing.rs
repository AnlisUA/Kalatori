use std::str::FromStr;

/// Parsed trigger error with structured fields
#[derive(Debug, Clone)]
pub struct TriggerError<S> {
    pub old_status: S,
    pub new_status: S,
}

pub trait StatusTransitionError: FromStr<Err: std::fmt::Debug> {
    type ErrorType: From<TriggerError<Self>>;

    const ERROR_TYPE_PREFIX: &'static str;

    fn from_sqlx_error(db_error: &sqlx::Error) -> Option<Self::ErrorType> {
        parse_trigger_error::<Self>(db_error, Self::ERROR_TYPE_PREFIX).map(Self::ErrorType::from)
    }
}

/// Parse SQLite trigger error message with generic status type
/// Format: "ERROR_TYPE|old_status=VALUE|new_status=VALUE"
///
/// The status type S must implement FromStr to parse status strings
pub(crate) fn parse_trigger_error<S: FromStr>(db_error: &sqlx::Error, prefix: &str) -> Option<TriggerError<S>>
where
    S: FromStr,
    S::Err: std::fmt::Debug,
{
    if let sqlx::Error::Database(err) = db_error {
        let msg = err.message();

        // Determine error type and strip prefix
        let params = msg.strip_prefix(prefix)?;

        // Parse key=value pairs
        let mut old_status_str = None;
        let mut new_status_str = None;

        for param in params.split('|') {
            if let Some((key, value)) = param.split_once('=') {
                match key {
                    "old_status" => old_status_str = Some(value),
                    "new_status" => new_status_str = Some(value),
                    _ => {},
                }
            }
        }

        // Parse status strings into typed enums
        if let (Some(old_str), Some(new_str)) = (old_status_str, new_status_str) {
            let old_status = S::from_str(old_str).ok()?;
            let new_status = S::from_str(new_str).ok()?;

            return Some(TriggerError {
                old_status,
                new_status,
            });
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        InvoiceStatus,
        PayoutStatus,
    };

    #[test]
    fn test_status_parsing_works() {
        // Verify that our status types can parse themselves
        assert_eq!(
            InvoiceStatus::from_str("Paid").unwrap(),
            InvoiceStatus::Paid
        );
        assert_eq!(
            PayoutStatus::from_str("FailedRetriable").unwrap(),
            PayoutStatus::FailedRetriable
        );
    }
}

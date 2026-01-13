use std::str::FromStr;

/// Parsed trigger error with structured fields
#[derive(Debug, Clone)]
pub struct StatusTriggerError<S> {
    pub old_status: S,
    pub new_status: S,
}

pub trait StatusTransitionError: FromStr<Err: std::fmt::Debug> {
    type ErrorType: From<StatusTriggerError<Self>>;

    const ERROR_TYPE_PREFIX: &'static str;

    fn from_sqlx_error(db_error: &sqlx::Error) -> Option<Self::ErrorType> {
        parse_trigger_error::<Self>(db_error, Self::ERROR_TYPE_PREFIX).map(Self::ErrorType::from)
    }
}

fn parse_error_with_statuses<S>(
    db_error: &sqlx::Error,
    prefix: &str,
) -> Option<(Option<S>, Option<S>)>
where S: FromStr,
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
                    "old_status" => old_status_str = value.parse().ok(),
                    "new_status" => new_status_str = value.parse().ok(),
                    _ => {},
                }
            }
        }

        return Some((old_status_str, new_status_str));
    }

    None
}

/// Parse `SQLite` trigger error message with generic status type
/// Format: "`ERROR_TYPE|old_status=VALUE|new_status=VALUE`"
///
/// The status type S must implement `FromStr` to parse status strings
fn parse_trigger_error<S>(
    db_error: &sqlx::Error,
    prefix: &str,
) -> Option<StatusTriggerError<S>>
where
    S: FromStr,
    S::Err: std::fmt::Debug,
{
    let (old_status_opt, new_status_opt) =
        parse_error_with_statuses::<S>(db_error, prefix)?;

    if let (Some(old_status), Some(new_status)) = (old_status_opt, new_status_opt) {
        Some(StatusTriggerError {
            old_status,
            new_status,
        })
    } else {
        None
    }
}

pub fn parse_update_not_allowed_error<S>(
    db_error: &sqlx::Error,
    prefix: &str,
) -> Option<S>
where S: FromStr,
      S::Err: std::fmt::Debug,
{
    parse_error_with_statuses(db_error, prefix)?.0
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

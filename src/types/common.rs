//! Common types shared across multiple modules

use std::fmt;

use serde::{Deserialize, Serialize};
use sqlx::Type;

/// Initiator type for payouts and refunds
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum InitiatorType {
    System,
    Admin,
}

impl fmt::Display for InitiatorType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::System => write!(f, "System"),
            Self::Admin => write!(f, "Admin"),
        }
    }
}

impl std::str::FromStr for InitiatorType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "System" => Ok(Self::System),
            "Admin" => Ok(Self::Admin),
            _ => Err(format!("Unknown initiator type: {s}")),
        }
    }
}

/// Module defining various types used across the Kalatori application.
/// Each domain-specific type (or collection of types) is organized into its own
/// submodule.
///
/// For testing purposes, it's also recommended to create fixtures functions
/// within each submodule to facilitate easy generation of test data. For
/// example: ```ignore
/// // In invoice.rs
/// #[cfg(test)]
/// fn default_invoice() -> Invoice {
///    // Create and return a default Invoice instance for testing
/// }
/// ```
mod common;
mod invoice;
mod payout;
mod refund;
mod transaction;

// Re-export commonly used types for convenience
#[expect(unused_imports)]
pub use common::*;
pub use invoice::*;
#[expect(unused_imports)]
pub use payout::*;
#[expect(unused_imports)]
pub use refund::*;
pub use transaction::*;

//! Logging constants for structured logging across the application.
//!
//! These constants are used in tracing fields to enable consistent log categorization
//! and filtering in production.

#[expect(dead_code)]
/// Log category constants for identifying the source subsystem
pub mod category {
    pub const CHAIN_CLIENT: &str = "chain_client";
    pub const DATABASE: &str = "database";
}

#[expect(dead_code)]
/// Log operation constants for identifying specific operations within subsystems
pub mod operation {
    // Chain client operations
    pub const CONNECT_CLIENT: &str = "connect_client";
    pub const FETCH_METADATA: &str = "fetch_metadata";
    pub const FETCH_BALANCE: &str = "fetch_balance";
    pub const FETCH_ASSET_INFO: &str = "fetch_asset_info";
    pub const FETCH_STORAGE: &str = "fetch_storage";
    pub const SUBMIT_TRANSACTION: &str = "submit_transaction";
    pub const WATCH_TRANSACTION: &str = "watch_transaction";
    pub const BUILD_TRANSFER: &str = "build_transfer";
    pub const SIGN_TRANSACTION: &str = "sign_transaction";
    pub const PROCESS_BLOCK: &str = "process_block";
    pub const SUBSCRIBE_TRANSFERS: &str = "subscribe_transfers";

    // Payout operations
    pub const EXECUTE_PAYOUT: &str = "execute_payout";
    pub const SCHEDULE_PAYOUT: &str = "schedule_payout";

    // Database operations
    pub const INSERT_ORDER: &str = "insert_order";
    pub const UPDATE_ORDER: &str = "update_order";
    pub const FETCH_ORDER: &str = "fetch_order";
    pub const INSERT_TRANSACTION: &str = "insert_transaction";
}

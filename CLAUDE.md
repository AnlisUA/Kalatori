# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Kalatori is a self-hosted, non-custodial blockchain payment gateway daemon for processing crypto payments on Polkadot's Asset Hub parachain. It's designed to give merchants complete sovereignty over their payment infrastructure while offering customers an intuitive multi-asset payment experience.

**License**: GPLv3
**Language**: Rust (edition 2024, MSRV 1.85)
**Current Status**: Public Beta / MVP - vast majority of code can and should be rewritten

## Product Vision

Product documentation lives in `../obsidian-to-coda/Kalatori/`. Key goals:
- **Merchant focus**: Full control over data/infrastructure, non-custodial, transparent operations
- **Customer experience**: Clear payment instructions, reliable status feedback, mistake-proof flows
- **Future capabilities**: Underpay/overpay handling, refunds, scheduled sweeps, e-commerce plugins

Refer to product docs for feature development direction.

## Common Commands

All auxiliary commands are defined in the `Makefile`.

### Setup and Build
```bash
# Install subxt-cli (required for metadata generation)
make install-subxt-cli

# Download Asset Hub node metadata (required before building)
make download-node-metadata-ci

# Build release binary
make build-release
```

### Development Workflow
```bash
# Setup: Create Docker network (one-time)
make create-network

# Copy example configs
make copy-configs

# Run with Chopsticks (local test chain) - starts chopsticks + builds and runs daemon
make run

# Run with real Asset Hub (after copying production config)
make copy-ah-production-config
make run-release

# Stop chopsticks when done
make stop-chopsticks
```

### Code Quality
```bash
# Run checks (same as CI)
make cargo-check        # Basic compilation check
make cargo-clippy       # Linting (strict: -D warnings -D clippy::pedantic)
make cargo-fmt          # Format checking
make cargo-deny         # Dependency license/security checks
```

Prefer to use make targets for running checks and tests instead of calling cargo directly.

### Testing
```bash
# Black-box integration tests (requires running daemon)
cd tests
docker-compose up       # Starts test environment + daemon

# In another terminal:
cd tests/kalatori-api-test-suite
yarn
yarn test

# Run specific test
yarn test -t "should create, repay, and automatically withdraw an order in USDC"
```

**Test environment**: Uses Chopsticks (Substrate chain fork simulator) + TypeScript/Jest test suite hitting live API endpoints.

## Architecture Overview

Kalatori Daemon receives requests to create invoice (order) with specific amount and asset, generate unique payment account for that order, monitor blockchain for incoming payment to that account, mark order as paid when payment is detected, and automatically withdraw funds to merchant's recipient address.

It should be called from external systems (e.g. e-commerce platform) via HTTP API.

It should also handle cases of overpay and underpay, refunds, and provide audit logs for all transactions.

### Core Pattern

Kalatori currently uses an **actor model** with isolated async tasks communicating via `tokio::sync::mpsc` channels. Each major component runs independently and uses oneshot channels for request-response patterns.

We are currently refactoring the acrhitecture to reduce complexity and improve maintainability. The future vision is outlined at the end of this document.

```
HTTP Request → Server (Axum) → State (orchestrator)
                                  ├→ Database (currently sled, should be replaced with sqlite)
                                  ├→ ChainManager → ChainTracker(s) // Legacy implementation, will be replaced with single chain client (Asset Hub only) in the next version
                                  └→ Signer (secret management, should be removed in future, no longer needed cause we only store seed phrase in `SecretBox`)
```

### Component Responsibilities

#### `src/state.rs` - Central Orchestrator
- Currently long-lived async task with mpsc receiver loop, should be simplified to a clonable struct (`Arc<State>`) with direct async method calls
- Coordinates all subsystem interactions (Database, ChainManager, Signer)
- Handles: order creation, status queries, payment marking, withdrawals, callbacks

#### `src/database.rs` - Financial Ledger
- Currently uses `sled` embedded key-value store (no external DB required), will be replaced with `sqlite` for better reliability and querying capabilities
- Currently has tables: `orders`, `transactions`, `pending_transactions`, `server_info`. Will use SQL tables defined in `./migrations`
- Current order state machine: PaymentStatus (Pending → Paid), WithdrawalStatus (Waiting → Completed/Forced/Failed). Will be extended to handle underpay/overpay/refunds in future.
- Handles periodic account cleanup (expired orders based on death timestamp)

#### `src/chain.rs` + `src/chain/tracker.rs` - Blockchain Interaction
- **ChainManager**: Routes requests to appropriate chain tracker
- **ChainTracker**: Per-chain actor monitoring finalized blocks via `subxt` subscription
- Scans ALL watched accounts every block
- Detects `Assets::transfer` and `Assets::transfer_all` extrinsics
- Multiple RPC endpoints with automatic failover and health tracking (Ok/Degraded/Critical)
- 120s watchdog timeout to detect frozen connections

#### `src/chain/payout.rs` - Withdrawal Handler
- Spawns separate async task per payout attempt (will be refactored to single actor which handles all transfers)
- Derives keypair from seed phrase for order-specific account
- Uses Asset Hub's `transfer_all` extrinsic (sweeps entire balance)
- Pays fees in same asset (Asset Hub feature)
- Records transaction before submission

#### `src/signer.rs` - Secret Management
- Currently: isolated actor holding seed phrase (zeroize on shutdown). Will be removed in future versions cause the problem of memory cleanup was solved using `SecretBox`.
- BIP39 mnemonic → sr25519 keypair derivation
- **HD derivation path**: `seed//<recipient_base58>//<order_id>`
- Never exposes private keys - only derives public keys on request
- Deterministic: same seed + order ID = same payment account (for withdrawal signing)

#### `src/server.rs` + `src/handlers/` - HTTP API
- Axum web server on `/v2/` base path
- Key endpoints: `POST /order/:order_id`, `GET /status`, `GET /health`, `GET /audit`
- Uses State extractors to inject `StateInterface` into handlers
- Graceful shutdown via `CancellationToken`

#### `src/configs.rs` - Configuration System
Five JSON config files (all optional if using env vars, except `chain.json`'s assets):
1. **chain.json**: Chain name, RPC endpoints, asset list (mandatory assets field)
2. **payments.json**: Recipient address, account lifetime (default 24h), remark
3. **seed.json**: BIP39 seed phrase
4. **database.json**: Database path, temporary mode flag
5. **web_server.json**: Host, port (default 0.0.0.0:16726)

**Env var override system**:
- Pattern: `{PREFIX}_{CONFIG}_{FIELD}` (e.g., `KALATORI_PAYMENTS_RECIPIENT`)
- Custom prefix: `KALATORI_APP_ENV_PREFIX` (changes `KALATORI` to custom prefix)
- Seed phrase auto-deleted from environment after loading (security)
- Config directory: `KALATORI_CONFIG_DIR_PATH`

### Blockchain Interaction (Subxt)

**Metadata-driven approach**:
- Uses `subxt` with compile-time metadata (`metadata.scale` file)
- Runtime types generated via `#[subxt::subxt]` macro in `src/chain/runtime.rs`
- **IMPORTANT**: Must regenerate metadata when updating subxt or connecting to new chain version

**Payment detection flow**:
1. Subscribe to finalized blocks
2. For each block, scan ALL watched payment accounts (no optimization)
3. Query balance via `runtime::storage().assets().account(asset_id, account_id)`
4. If balance >= expected amount → mark order as paid
5. Trigger payout task (sweep to recipient)

### Background Task Management

**TaskTracker** (`src/utils/task_tracker.rs`):
- Wraps `tokio_util::task::TaskTracker` with error collection
- Named tasks for debugging: `task_tracker.spawn("task name", async_task)`
- Centralized error handling via unbounded mpsc channel
- Any task error triggers application shutdown

**Shutdown sequence**:
1. Signal received (SIGTERM/SIGINT) or fatal error → `CancellationToken` cancelled
2. State orchestrates: ChainManager.shutdown() → Database.shutdown() → Signer.shutdown()
3. TaskTracker waits for all tasks to complete
4. Clean exit

### Account Derivation System

**Hierarchical Deterministic (HD) key derivation**:
```
Seed Phrase (BIP39) → sr25519 Root
  → //<recipient_address_base58>
    → //<order_id>
      → Unique Payment Account
```

**Process**:
1. Order created with ID "abc123"
2. Signer derives: `seed//<recipient>//abc123` → public key (ss58 format)
3. Public key becomes payment account address
4. Customer sends funds to payment account
5. For withdrawal, same derivation regenerates keypair to sign `transfer_all` transaction

**Security**: Currently seed only lives in Signer actor, private keys never leave the module. In future, seed will be stored in `SecretBox` and Signer module removed.

## Important Patterns and Conventions

### Error Handling
- Custom `Error` enum in `src/error.rs` with `thiserror` derive
- `PrettyCause` trait for user-friendly error formatting
- Fatal errors trigger shutdown via TaskTracker
- Panic hook configured to log via tracing and trigger shutdown

## Error Handling Principles

These principles guide error type design across the codebase, particularly in `chain_client` and related modules.

### Principle 1: Only Enumerate Errors Requiring Different Handling

**Core Rule**: Create separate error variants ONLY when the calling code needs to behave differently based on the variant.

**Decision Test**: For any two error scenarios, ask: "Does the caller need to DO something different?"
- If YES (different retry logic, user message, metrics, etc.) → Separate variants
- If NO (same handling, only context differs) → Single variant with context fields

**Examples:**

✅ **Good - Variants have different handling:**
```rust
pub enum TransactionError {
    // Different handling: try different RPC endpoint
    NetworkError { endpoint: String },

    // Different handling: mark as failed, notify user
    InsufficientBalance {
        transaction_id: TxId,
        required: Option<Decimal>,
        available: Option<Decimal>,
    },

    // Different handling: wait for mortality period, lookup via API
    SubmissionStatusUnknown {
        transaction_hash: H256,
    },
}
```

❌ **Bad - Same handling, unnecessary variants:**
```rust
pub enum ChainError {
    // All handled identically: log and retry
    BlockTimestampFetchFailed,
    BlockHashFetchFailed,
    BlockExtrinsicsFetchFailed,
    // Should be: BlockDataFetchFailed { data_type: String }
}
```

**Anti-Patterns to Avoid:**

1. **Over-specification**: Don't create variants that differ only in log messages
2. **String-based discrimination**: If you find yourself doing `if error_msg.contains("balance")`, you probably need a variant
3. **Lost type safety**: Use enums for context fields when reasonable (not just `String`)

**Mitigations for Weaknesses:**

This principle trades some type safety for maintainability. Mitigate with:

1. **Structured logging** - Include error classification in logs:
   ```rust
   tracing::warn!(
       error.type = "storage_query_failed",
       error.operation = "fetch_balance",
       asset_id = %asset_id,
       "Storage query failed"
   );
   ```

2. **Enum context fields** - Use typed enums instead of strings when possible:
   ```rust
   pub enum Operation { FetchBalance, FetchMetadata }
   pub struct Error { operation: Operation }  // Not String
   ```

3. **Document handling** - Explain recovery strategy in error docs:
   ```rust
   /// Network error during submission.
   /// **Recovery:** Try different RPC endpoint immediately.
   NetworkError { endpoint: String },
   ``, 

4. **Error code constants** - For matching and metrics:
   ```rust
   impl Error {
       pub const NETWORK_ERROR: &'static str = "network_error";
       pub fn error_code(&self) -> &'static str { ... }
   }
   ```

**Context**: Our architecture uses a worker-based retry system with database-backed transaction state. Retry logic lives in the worker (caller), not in error types. Errors should classify WHAT went wrong, not prescribe HOW to fix it.

### Principle 2: Log Raw Errors at the Point They Occur

**Core Rule**: When converting a library error (like `subxt::Error`) to a custom error type, log the full original error at the exact conversion point before transformation.

**Why**: Once you convert `subxt::Error` → `ChainError`, you lose rich library error details (request IDs, internal state, specific error codes). Logs preserve this information for debugging.

**Pattern:**

```rust
// ❌ BAD: Convert without logging
.map_err(|e| ChainError::ConnectionFailed { endpoint })?

// ✅ GOOD: Log with structured fields, then convert
.map_err(|e| {
    tracing::debug!(
        error.category = "chain_client",
        error.operation = "fetch_balance",
        asset_id = %asset_id,
        account = %account,
        error.source = ?e,  // Full library error
        "Balance fetch failed"
    );
    ChainError::StorageQueryFailed { ... }
})?
```

**Log Level Guidelines:**

| Level | When to Use | Example |
|-------|-------------|---------|
| DEBUG | Error conversions, expected failures | Balance fetch for new account |
| INFO | Significant business events | "Payout completed", "Order paid" |
| WARN | Recoverable errors, degraded state | "RPC endpoint degraded" |
| ERROR | Critical failures requiring attention | "All RPC endpoints down" |

**Correlation IDs for Multi-Step Operations:**

Generate correlation_id at entry points (HTTP handlers, worker job pickup), then use `#[instrument]` for nested functions:

```rust
// Entry point: Generate correlation_id and create root span
async fn handle_payout_request(payout_id: u64, client: Client, db: Database) {
    let correlation_id = Uuid::new_v4();
    let span = tracing::info_span!(
        "payout_request",
        correlation_id = %correlation_id,
        payout_id = %payout_id
    );

    process_payout(payout_id, &client, &db)
        .instrument(span)
        .await
}

// Nested functions: Use #[instrument], automatically inherit correlation_id
#[instrument(skip(client, db))]
async fn process_payout(payout_id: u64, client: &Client, db: &Database) -> Result<()> {
    // All logs here include correlation_id and payout_id from parent span
    let balance = client.fetch_balance(...).await?;
    let tx = client.build_transfer(...).await?;
    Ok(())
}

#[instrument(skip(self))]
async fn fetch_balance(&self, asset_id: u32, account: &AccountId) -> Result<Decimal> {
    // Still includes correlation_id from root span
    self.client.storage().fetch(...).await
        .map_err(|e| {
            tracing::debug!(
                error.source = ?e,
                asset_id = %asset_id,
                // correlation_id automatically included from span
                "Balance fetch failed"
            );
            ChainError::StorageQueryFailed { ... }
        })
}
```

**Standard Categories** (define in `src/utils/logging.rs`):

```rust
pub mod log_category {
    pub const CHAIN_CLIENT: &str = "chain_client";
    pub const PAYOUT: &str = "payout";
    pub const DATABASE: &str = "database";
    pub const API: &str = "api";
}

pub mod log_operation {
    pub const FETCH_BALANCE: &str = "fetch_balance";
    pub const SUBMIT_TX: &str = "submit_transaction";
    pub const EXECUTE_PAYOUT: &str = "execute_payout";
    // ...
}
```

**The Layer Rule** - Avoid duplicate logging:

- **Layer 3** (conversion boundary): Log raw library error with structured fields
- **Layer 2** (intermediate): Don't log, just convert custom error types
- **Layer 1** (handler): Log business-level error for user/ops

```rust
// Layer 3: chain_client (conversion boundary)
#[instrument(skip(self))]
async fn fetch_balance(...) -> Result<Decimal, ChainError> {
    self.client.storage().fetch(...).await
        .map_err(|e| {
            tracing::debug!(error.source = ?e, ...);  // ← Log here
            ChainError::StorageQueryFailed { ... }
        })
}

// Layer 2: payout logic
#[instrument(skip(client))]
async fn execute_payout(...) -> Result<(), PayoutError> {
    fetch_balance(...).await
        .map_err(|e| PayoutError::from(e))?  // ← Don't log, just convert
}

// Layer 1: payout worker
match execute_payout(...).await {
    Err(e) => tracing::warn!(payout_id, error = %e, ...)  // ← Log business error
}
```

**Production Configuration:**

```bash
# Default: INFO level, DEBUG for chain_client only
RUST_LOG=info,kalatori::chain_client=debug

# Output format: JSON for aggregation
RUST_LOG_FORMAT=json
```

**Context**: We use INFO level in production with DEBUG for detailed modules. Structured JSON output enables future log aggregation. Correlation IDs are critical for debugging multi-step payout/order workflows.

### Principle 3: Include Useful and Required Information Only

**Core Rule**: Error struct fields should pass the "actionability test" - include information that enables decision-making, recovery, or user communication. Avoid fields that belong in logs or database.

**The Actionability Test** - For each field, ask:

1. **Does this change what code DOES?** → Required
2. **Is it needed for user communication?** → Useful
3. **Can it be reconstructed from context?** → Remove (caller already has it)
4. **Is it only for debugging?** → Remove (put in logs via Principle 2)

**Examples:**

```rust
// ❌ BAD: Duplicates caller's context
async fn execute_payout(
    payout_id: u64,
    sender: AccountId,
    recipient: AccountId,
) -> Result<(), PayoutError> {
    // ...
    Err(PayoutError {
        payout_id,    // ← Caller already has these
        sender,       // ← Caller already has these
        recipient,    // ← Caller already has these
    })
}

// ✅ GOOD: Minimal error, caller has context
async fn execute_payout(
    payout_id: u64,
    sender: AccountId,
    recipient: AccountId,
) -> Result<(), PayoutError> {
    // ...
    Err(PayoutError::TransferFailed)  // ← Caller has payout_id in scope
}
```

**Project-Specific Rules:**

1. **Never include `endpoint`** - Logged at error site, available via `client.current_endpoint()` if needed
2. **Never include timestamps** - Always in logs and database records
3. **Never include retry state** - Worker manages retries via database (retry_count, last_attempt_at, etc.)
4. **Never include transaction hash for pre-finalization errors** - Caller has internal transaction ID; hash is unreliable on Asset Hub
5. **Include blockchain coordinates only when blockchain becomes source of truth** - After finalization, need (block_number, extrinsic_index) to re-query chain

**Source of Truth Pattern:**

| Lifecycle Stage | Source of Truth | What Error Needs |
|-----------------|-----------------|------------------|
| Planned (in DB) | Database | Nothing (caller has internal ID) |
| Submitted (unknown) | Database | Nothing (caller has internal ID) |
| Finalized | Blockchain | Coordinates (block_number, extrinsic_index) |

```rust
// Pre-finalization: No identifier needed
pub enum TransactionError {
    SubmissionStatusUnknown,  // ← Caller has internal_tx_id
}

// Caller code:
let internal_tx_id = db.create_planned_transaction(...)?;
match client.submit_transaction(...).await {
    Err(TransactionError::SubmissionStatusUnknown) => {
        // internal_tx_id is RIGHT HERE in scope
        db.mark_transaction_unknown_state(internal_tx_id)?;
    }
}

// Post-finalization: Blockchain coordinates needed
pub enum TransactionError<T: ChainConfig> {
    ExecutionFailed {
        transaction_id: T::TransactionId,  // e.g., (block_number, extrinsic_index)
        error_code: String,
    }
}

// Caller code:
match result {
    Err(TransactionError::ExecutionFailed { transaction_id, .. }) => {
        // Can retry fetching from blockchain using coordinates
        client.refetch_transaction_info(transaction_id).await?;
    }
}
```

**Use `Option<T>` When:**
- Information genuinely might not be available (chain error doesn't include amounts)
- Handling can degrade gracefully

```rust
// ✅ Good use of Option
InsufficientBalance {
    transaction_id: TxId,           // ← Always have this
    required: Option<Decimal>,      // ← Chain might not provide
    available: Option<Decimal>,     // ← Might not have fetched
}

// Handling degrades gracefully:
match error {
    InsufficientBalance { required: Some(r), available: Some(a), .. } => {
        format!("Need {} more", r - a)  // Best case
    }
    InsufficientBalance { .. } => {
        "Insufficient balance".to_string()  // Fallback
    }
}
```

**Prefer Struct Variants Over Tuples:**

```rust
// ❌ Unclear
FetchTransactionInfoError((BlockNumber, H256))  // Which is which?

// ✅ Clear
FetchTransactionInfoError {
    block_number: u32,
    transaction_hash: H256,
}
```

**Where Additional Info Lives:**

Document in error type where to find information not included:

```rust
/// Connection operation failed.
///
/// **Available information:**
/// - Operation: In error type
/// - Endpoint: `client.current_endpoint()`
/// - Timestamp: In logs with correlation_id
/// - Retry state: In database payout/transaction record
pub enum ChainError {
    ConnectionFailed { operation: String },
}
```

**Context**: Our architecture has multiple sources of truth: database (planned transactions, retry state), logs (timestamps, endpoints, detailed errors), and blockchain (finalized transactions). Errors only include what's not available elsewhere or what's needed for immediate handling decisions.

### Principle 4: Separate Error Enums for Different Domains

**Core Rule**: Create multiple focused error types for different **usage contexts** (not technical categories). Split by what the caller is doing, not by error's technical nature.

**The Domain Test**: Errors belong in the same enum if they:
1. Share the same calling context (same functions produce/handle them)
2. Require similar recovery strategies
3. Represent the same abstraction level

**Project Domains** (chain_client):

```rust
// 1. Initialization
pub enum ClientError {
    AllEndpointsUnreachable,
    MetadataFetchFailed,
    InvalidConfiguration { field: String },
    UnknownAssetId { asset_id: u32 },  // Validated at init AND runtime
}

// 2. One-off blockchain queries
pub enum QueryError {
    RpcRequestFailed,        // Try different endpoint
    NotFound { query_type: String },
    DecodeFailed { data_type: String },
}

// 3. Block streaming
pub enum SubscriptionError {
    SubscriptionFailed,      // Restart subscription
    StreamClosed,
    BlockProcessingFailed { block_number: u32 },  // Skip block
}

// 4. Transaction lifecycle
pub enum TransactionError<T: ChainConfig> {
    BuildFailed { reason: String },
    SubmissionStatusUnknown,  // Mark unknown in DB
    ExecutionFailed {
        transaction_id: T::TransactionId,  // Post-finalization
        error_code: String,
    },
    InsufficientBalance {
        transaction_id: T::TransactionId,
        required: Option<Decimal>,
        available: Option<Decimal>,
    },
    UnknownAsset {
        transaction_id: T::TransactionId,
        asset_id: T::AssetId,
    },
}
```

**Why separate QueryError and SubscriptionError?**
Different recovery: queries retry immediately with different endpoint; subscriptions restart entire stream.

**Cross-Domain Conversion:**

Use `From` for obvious conversions:
```rust
impl From<KeyringError> for TransactionError<T> {
    fn from(e: KeyringError) -> Self {
        tracing::debug!(error.source = ?e, ...);  // Log conversion (Principle 2)
        TransactionError::BuildFailed { reason: "Signing failed".into() }
    }
}
```

Use `.map_err()` when context matters:
```rust
client.fetch_balance(...).await
    .map_err(|e| PayoutError::PreflightCheckFailed {
        check: "balance", underlying: e.to_string()
    })?
```

**API Layer Boundary:**

Internal errors never leak to public API. Convert at handler:
```rust
async fn handler(...) -> Result<Json<Response>, ApiError> {
    state.execute(...).await
        .map_err(|e| match e {
            InternalError::Specific { .. } => ApiError {
                code: "error_code",
                description: "User message",
                extra_data: Some(json!({ ... })),
            },
            // ... conversions
        })?;
}
```

**Avoid Unifier Enums Internally:**

✅ **Preferred** - Separate return types:
```rust
async fn fetch_balance(...) -> Result<Decimal, QueryError>;
async fn subscribe_transfers(...) -> Result<Stream, SubscriptionError>;
```

✅ **Acceptable** - Flattened enum (not nested unifier):
```rust
pub enum ClientError {
    AllEndpointsUnreachable,   // From connection concern
    MetadataFetchFailed,       // From query concern
    InvalidConfiguration { field: String },
}
```

❌ **Avoid** - Deep unifier hierarchies:
```rust
pub enum ChainError {
    Client(ClientError),
    Query(QueryError),
    // ...
}
```

**Relationship to Principle 1:**
- Principle 1 (within domain): Only enumerate errors requiring different handling
- Principle 4 (between domains): Separate error types for different usage contexts

**Context**: Usage-based domains align with recovery strategies. Each domain has focused error handling: initialization fails fast, queries retry with failover, subscriptions restart stream, transactions use DB-backed retry worker.

### Principle 5: Internal Errors Shouldn't Leak to API

**Core Rule**: Public API responses must never expose secrets or unnecessarily verbose internal details. All handler errors implement `KalatoriApiError` trait to provide their own API representation.

**Key Innovation**: Instead of centralized conversion functions, each error type defines its own API representation via trait. This is decentralized, type-safe, and exhaustive (compiler enforces complete coverage).

**The KalatoriApiError Trait:**

```rust
pub trait KalatoriApiError: std::error::Error {
    /// Machine-readable error code (snake_case, stable across versions)
    fn code(&self) -> String;

    /// Human-readable error message (safe for display)
    fn message(&self) -> String;

    /// Optional structured data (flexible schema per error variant)
    fn data(&self) -> Option<serde_json::Value> {
        None  // Default: no extra data
    }

    /// HTTP status code for this error
    fn http_code(&self) -> StatusCode;
}
```

**Blanket IntoResponse Implementation:**

```rust
impl<T: KalatoriApiError> IntoResponse for T {
    fn into_response(self) -> Response {
        let api_error = ApiError {
            code: self.code(),
            description: self.message(),
            extra_data: self.data(),
        };

        let correlation_id = /* extract from current span */;

        (
            self.http_code(),
            [(header::HeaderName::from_static("x-correlation-id"), correlation_id)],
            Json(api_error)
        ).into_response()
    }
}
```

**Implementation Example:**

```rust
impl KalatoriApiError for PayoutError {
    fn code(&self) -> String {
        match self {
            PayoutError::InsufficientBalance { .. } => "insufficient_balance",
            PayoutError::ChainUnavailable => "service_unavailable",
            PayoutError::AccountNotFound => "account_not_found",
            PayoutError::InvalidRequest { .. } => "invalid_request",
            // Compiler enforces exhaustiveness - no `_ =>` needed!
        }.to_string()
    }

    fn message(&self) -> String {
        match self {
            PayoutError::InsufficientBalance { required, available, .. } => {
                match (required, available) {
                    (Some(r), Some(a)) => {
                        format!("Insufficient balance. Required: {}, Available: {}", r, a)
                    },
                    _ => "Insufficient balance to complete payout.".to_string(),
                }
            },
            PayoutError::ChainUnavailable => {
                "Blockchain temporarily unavailable. Please retry.".to_string()
            },
            PayoutError::AccountNotFound => {
                "Payment account not found or expired.".to_string()
            },
            PayoutError::InvalidRequest { reason } => {
                format!("Invalid request: {}", reason)
            },
        }
    }

    fn data(&self) -> Option<serde_json::Value> {
        match self {
            PayoutError::InsufficientBalance { transaction_id, required, available } => {
                Some(json!({
                    "internal_transaction_id": transaction_id,  // OK to include
                    "required": required.map(|d| d.to_string()),
                    "available": available.map(|d| d.to_string()),
                }))
            },
            PayoutError::InvalidRequest { field, reason } => {
                Some(json!({
                    "field": field,
                    "reason": reason,
                }))
            },
            _ => None,
        }
    }

    fn http_code(&self) -> StatusCode {
        match self {
            PayoutError::InsufficientBalance { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            PayoutError::ChainUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            PayoutError::AccountNotFound => StatusCode::NOT_FOUND,
            PayoutError::InvalidRequest { .. } => StatusCode::BAD_REQUEST,
        }
    }
}
```

**Handler Usage:**

Handlers simply return the error - trait converts automatically:

```rust
async fn create_payout_handler(
    State(state): State<AppState>,
    Path(payout_id): Path<u64>,
) -> Result<Json<PayoutResponse>, PayoutError> {
    state.execute_payout(payout_id).await
        .map_err(|e| {
            tracing::warn!(
                payout_id = %payout_id,
                error.internal = ?e,       // Log full internal error
                error.code = e.code(),     // API error code
                "Payout execution failed"
            );
            e  // Return error - IntoResponse handles conversion
        })?;

    Ok(Json(result))
}
```

**What NOT to Expose:**

Never include:
1. **Secrets** - Seed phrases, private keys, API tokens
2. **Security-sensitive info** - Database connection strings, authentication tokens
3. **Stack traces** - Use correlation_id in logs instead
4. **Raw library errors** - Full `subxt::Error` details (convert to meaningful message)
5. **Implementation details** - "Failed to connect to postgres", "sled database locked"

Safe to include:
1. **Identifiers** - Order IDs, internal transaction IDs, asset IDs
2. **Business data** - Amounts, balances, asset names
3. **Actionable info** - Validation errors, required fields, retry counts
4. **System state** - Queue positions, worker IDs (when useful)
5. **Blockchain data** - Block numbers, transaction hashes, extrinsic indices

**Guideline**: Include information that helps users/operators understand and act on the error. Exclude only secrets and unnecessarily verbose internals.

**ApiError Structure (Output Only):**

Built automatically by blanket implementation:

```rust
#[derive(Serialize)]
pub struct ApiError {
    pub code: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_data: Option<serde_json::Value>,
}
```

JSON output:
```json
{
  "code": "insufficient_balance",
  "description": "Insufficient balance. Required: 100.50, Available: 75.00",
  "extra_data": {
    "internal_transaction_id": 12345,
    "required": "100.50",
    "available": "75.00"
  }
}
```

HTTP headers: `X-Correlation-ID: 550e8400-...`, Status: `422 Unprocessable Entity`

**Error Code Conventions:**

Format: `snake_case`, stable across versions

Standard codes: `invalid_request`, `validation_failed`, `account_not_found`, `order_not_found`, `insufficient_balance`, `payment_already_processed`, `asset_not_supported`, `service_unavailable`, `internal_error`, `blockchain_error`, `timeout`

Stability: Can add codes, change messages, add data fields. Cannot remove/rename without version bump.

**Cross-Domain Conversion:**

When errors convert across domains, use `From` trait:

```rust
impl From<QueryError> for PayoutError {
    fn from(e: QueryError) -> Self {
        match e {
            QueryError::RpcRequestFailed => PayoutError::ChainUnavailable,
            QueryError::NotFound { .. } => PayoutError::AccountNotFound,
            QueryError::DecodeFailed { .. } => PayoutError::ChainDataCorrupted,
        }
    }
}
```

Only the final error type (returned from handler) needs `KalatoriApiError` implementation.

**Benefits:**

✅ Decentralized: Error definition and API representation in same place
✅ Type-safe: Compiler enforces exhaustiveness, no `_ =>` fallback needed
✅ Clean handlers: Auto-conversion via trait
✅ No centralized conversion boilerplate
✅ Maintainable: Add variant → compiler forces trait implementation

**Relationship to Other Principles:**
- Principle 1: Only create variants requiring different handling - trait ensures each gets proper API representation
- Principle 3: Internal errors have rich context; trait methods extract only actionable info for API
- Principle 4: Each domain error implements trait independently

**Context**: Trait-based approach trades some coupling (errors know about HTTP) for massive gains in maintainability and type safety. No centralized conversion function to keep in sync. Correlation ID (in header) links API errors to internal logs.

### Logging
- Uses `tracing` with `tracing-subscriber`
- Currently default filter configured in `src/utils/logger.rs`. Will be replaced with dynamic config in future.
- All major operations logged at appropriate levels (info/warn/error)
- We need to log all incoming requests, state changes, errors, and blockchain events for auditability

### Code Quality Lints
Strict linting enabled in `Cargo.toml`:
```toml
[lints.clippy]
pedantic = { level = "warn", priority = -1 }
arithmetic_side_effects = "warn"
shadow_reuse/shadow_same/shadow_unrelated = "warn"
```
**IMPORTANT**: Clippy runs with `-D warnings` in CI - all warnings are errors.

### Module Layout
- Uses self-named modules (e.g., `chain.rs` + `chain/` directory)
- **Never use `mod.rs`** files (enforced by `mod_module_files` clippy lint)
- Reason: Better Git history, avoids file renaming issues

### Security Considerations
- Seed phrase zeroized on shutdown (`zeroize` crate)
- No private keys in logs or API responses
- Currently all transaction signing happens in isolated Signer actor, will be improved with `SecretBox` in future
- Database stored on disk (not in-memory by default) for durability

## Known Limitations and TODOs

This is an MVP - many areas need improvement:

1. **Configuration**: Hardcoded RPC URLs in some places (see TODOs in `Makefile`)
2. **Error recovery**: No automatic retry for failed payouts
3. **Account management**: No underpay/overpay handling yet (product roadmap)
4. **Refunds**: Not implemented (product roadmap)
5. **Testing**: No unit tests visible - focus has been on integration tests
6. **Scalability**: Scans all accounts every block (O(n) per block - needs optimization for large deployments)
7. **Metadata management**: Manual update process (should be automated)

## Version Bumping and Releases

Refer to `CONTRIBUTING.md` for detailed release process:
1. Update `Cargo.toml` version
2. Generate changelog: `git cliff main/main..HEAD --tag <version> -p CHANGELOG.md`
3. Commit: `git commit -m "chore: bump version to X.Y.Z"`
4. Tag at main branch: `git tag -a vX.Y.Z -m "Release version X.Y.Z"`
5. Push tag to trigger CI release build

## Key Dependencies

- **subxt** (0.44): Polkadot SDK client for blockchain interaction
- **axum** (0.7): HTTP server framework
- **tokio**: Async runtime (multi-threaded)
- **sled** (0.34): Embedded key-value database
- **codec** (parity-scale-codec): SCALE encoding for Substrate types
- **bip39**: BIP39 mnemonic handling (with zeroize)

**IMPORTANT**: When updating `subxt` version:
1. Update `subxt_cli_version` in `Makefile` to match
2. Re-run `make install-subxt-cli`
3. Regenerate metadata: `make download-node-metadata-ci`
4. Rebuild project

## Relevant links
- V2 API spec: https://github.com/Kalapaja/kalatori-api/blob/master/kalatori.yaml

------------------------------------------------------------------------------
## Future Vision

### Architecture
- Actor model used only for chain monitoring, periodic database tasks; The rest is made via thread pooling.
- Get rid of mpsc channels in `State` module; direct method calls with async/await. For access to shared state, use `Arc<State>`.
- `DAO` should operate with new types defined in `types` module. Legacy types should be used only in v2 API handlers.
- In order to preserve backward compatibility, existing API endpoints should remain unchanged.

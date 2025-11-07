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

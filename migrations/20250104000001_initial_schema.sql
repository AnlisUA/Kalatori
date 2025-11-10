-- Initial schema migration: New structure based on product requirements
-- Uses updated naming conventions: orders -> invoices, proper datetime types
-- Simplified: single transactions table (no separate pending_transactions)
-- Financial amounts stored as TEXT to preserve decimal precision
-- Includes new entities: payouts, refunds
-- All enums use CamelCase naming
-- Minimized backward compat fields - reconstruct from config where possible

-- Invoices table (replaces orders)
CREATE TABLE IF NOT EXISTS invoices (
    -- Identity
    id TEXT PRIMARY KEY NOT NULL,  -- UUID v4 - internal ID
    order_id TEXT NOT NULL UNIQUE,  -- Merchant-provided order ID

    -- Asset information (denormalized to avoid config changes affecting data)
    asset_id INTEGER,  -- NULL for native tokens
    chain TEXT NOT NULL,

    -- Payment details
    amount TEXT NOT NULL,  -- Expected amount as decimal string (e.g., "123.456789")
    payment_address TEXT NOT NULL,

    -- Status (new unified status system)
    status TEXT NOT NULL CHECK(status IN (
        'Waiting', 'PartiallyPaid',  -- Active
        'Paid', 'OverPaid', 'AdminApproved',  -- Final
        'UnpaidExpired', 'PartiallyPaidExpired',  -- Expired
        'CustomerCanceled', 'AdminCanceled'  -- Canceled
    )) DEFAULT 'Waiting',

    -- Backward compatibility: old withdrawal_status (temporary, will be removed with sled)
    -- This will be computed from payouts table status in Rust code, but kept in DB during transition
    withdrawal_status TEXT NOT NULL CHECK(withdrawal_status IN ('Waiting', 'Failed', 'Forced', 'Completed')),

    -- Callback
    callback TEXT NOT NULL DEFAULT '',

    -- Cart metadata
    cart TEXT NOT NULL DEFAULT '{}',  -- JSONB: {cart_items?}

    -- Timestamps
    valid_till TEXT NOT NULL,  -- ISO 8601 datetime
    created_at TEXT NOT NULL DEFAULT (datetime('now')),  -- ISO 8601 datetime
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))  -- ISO 8601 datetime

    -- Note: CurrencyInfo (currency_name, decimals, rpc_url, etc.) will be reconstructed in Rust
    -- from asset_id + chain + config, no need to store redundantly
);

-- Transactions table (unified: both pending and finalized transactions)
-- Replaces both old 'transactions' and 'pending_transactions' tables
CREATE TABLE IF NOT EXISTS transactions (
    -- Identity
    id TEXT PRIMARY KEY NOT NULL,  -- UUID v4
    invoice_id TEXT NOT NULL,  -- References invoices.id

    -- Asset information
    asset_id INTEGER NOT NULL,
    chain TEXT NOT NULL,
    amount TEXT NOT NULL,  -- Decimal string (excluding fees)

    -- Addresses (needed for refunds - sender is who we refund to)
    sender TEXT NOT NULL,  -- For incoming: customer address (refund destination), for outgoing: payment address
    recipient TEXT NOT NULL,  -- For incoming: payment address, for outgoing: payout/refund destination

    -- Blockchain location (NULL until finalized)
    block_number INTEGER,  -- NULL for pending transactions
    position_in_block INTEGER,  -- NULL for pending transactions (extrinsic index)
    tx_hash TEXT,  -- NULL for pending transactions, hex-encoded when finalized

    -- Origin (what triggered this transaction)
    origin TEXT NOT NULL DEFAULT '{}',  -- JSONB: {refund_id?, payout_id?, internal_transfer_id?}

    -- Status and type
    status TEXT NOT NULL CHECK(status IN ('Waiting', 'InProgress', 'Completed', 'Failed')) DEFAULT 'Waiting',
    transaction_type TEXT NOT NULL CHECK(transaction_type IN ('Incoming', 'Outgoing')),

    -- Metadata for outgoing transactions
    outgoing_meta TEXT NOT NULL DEFAULT '{}',  -- JSONB: {extrinsic_bytes?, built_at?, sent_at?, confirmed_at?, failed_at?, failure_message?}

    -- Timestamps
    created_at TEXT NOT NULL DEFAULT (datetime('now')),

    -- Backward compatibility fields (temporary - ONLY for sled migration deduplication)
    transaction_bytes TEXT,  -- Hex-encoded extrinsic (old field, used as unique key in old system)

    FOREIGN KEY (invoice_id) REFERENCES invoices(id) ON DELETE CASCADE
);

-- Payouts table (NEW - replaces old implicit payout mechanism)
-- Payout = transfer from Payment Address to Payout Address (merchant's final wallet)
CREATE TABLE IF NOT EXISTS payouts (
    -- Identity
    id TEXT PRIMARY KEY NOT NULL,  -- UUID v4
    invoice_id TEXT NOT NULL,  -- References invoices.id

    -- Asset information
    asset_id INTEGER NOT NULL,
    chain TEXT NOT NULL,

    -- Addresses
    source_address TEXT NOT NULL,  -- Payment address
    destination_address TEXT NOT NULL,  -- Payout address (merchant's final wallet)

    -- Amount is always "transfer all" for payouts (sweep entire balance)
    -- No explicit amount field - uses transfer_all mechanism

    -- Initiator
    initiator_type TEXT NOT NULL CHECK(initiator_type IN ('System', 'Admin')),
    initiator_id TEXT,  -- Optional UUID v4 (NULL for System-initiated payouts)

    -- Status
    status TEXT NOT NULL CHECK(status IN ('Waiting', 'InProgress', 'Completed', 'Failed')) DEFAULT 'Waiting',

    -- Timestamps
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),

    FOREIGN KEY (invoice_id) REFERENCES invoices(id) ON DELETE CASCADE
);

-- Refunds table (NEW - not in old system)
CREATE TABLE IF NOT EXISTS refunds (
    -- Identity
    id TEXT PRIMARY KEY NOT NULL,  -- UUID v4
    invoice_id TEXT NOT NULL,  -- References invoices.id

    -- Asset information
    asset_id INTEGER NOT NULL,
    chain TEXT NOT NULL,
    amount TEXT NOT NULL,  -- Decimal string

    -- Addresses
    source_address TEXT NOT NULL,  -- Payment address or reserve account
    destination_address TEXT NOT NULL,  -- Usually the sender from original payment
    allow_transfer_all INTEGER NOT NULL DEFAULT 0,  -- Boolean (SQLite uses INTEGER: 0=false, 1=true)

    -- Initiator
    initiator_type TEXT NOT NULL CHECK(initiator_type IN ('System', 'Admin')),
    initiator_id TEXT,  -- Optional UUID v4 (NULL for System)

    -- Status
    status TEXT NOT NULL CHECK(status IN ('Waiting', 'InProgress', 'Completed', 'Failed')) DEFAULT 'Waiting',

    -- Timestamps
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),

    FOREIGN KEY (invoice_id) REFERENCES invoices(id) ON DELETE CASCADE
);

-- Server info table (singleton - metadata about daemon instance)
CREATE TABLE IF NOT EXISTS server_info (
    instance_id TEXT PRIMARY KEY NOT NULL,
    version TEXT NOT NULL,
    remark TEXT
);

-- Indexes for common query patterns

-- Invoice queries: by status, by time, combined filters
CREATE INDEX IF NOT EXISTS idx_invoices_order_id ON invoices(order_id);
CREATE INDEX IF NOT EXISTS idx_invoices_status ON invoices(status);
CREATE INDEX IF NOT EXISTS idx_invoices_created_at ON invoices(created_at);
CREATE INDEX IF NOT EXISTS idx_invoices_valid_till ON invoices(valid_till);
CREATE INDEX IF NOT EXISTS idx_invoices_status_created ON invoices(status, created_at);
CREATE INDEX IF NOT EXISTS idx_invoices_status_valid_till ON invoices(status, valid_till);
CREATE INDEX IF NOT EXISTS idx_invoices_payment_address ON invoices(payment_address);

-- Transaction queries
CREATE INDEX IF NOT EXISTS idx_transactions_invoice_id ON transactions(invoice_id);
CREATE INDEX IF NOT EXISTS idx_transactions_type ON transactions(transaction_type);
CREATE INDEX IF NOT EXISTS idx_transactions_status ON transactions(status);
CREATE INDEX IF NOT EXISTS idx_transactions_created_at ON transactions(created_at);

-- Payout queries
CREATE INDEX IF NOT EXISTS idx_payouts_invoice_id ON payouts(invoice_id);
CREATE INDEX IF NOT EXISTS idx_payouts_status ON payouts(status);
CREATE INDEX IF NOT EXISTS idx_payouts_created_at ON payouts(created_at);

-- Refund queries
CREATE INDEX IF NOT EXISTS idx_refunds_invoice_id ON refunds(invoice_id);
CREATE INDEX IF NOT EXISTS idx_refunds_status ON refunds(status);
CREATE INDEX IF NOT EXISTS idx_refunds_created_at ON refunds(created_at);

-- Triggers to auto-update timestamps

CREATE TRIGGER IF NOT EXISTS update_invoices_timestamp
AFTER UPDATE ON invoices
FOR EACH ROW
BEGIN
    UPDATE invoices SET updated_at = datetime('now') WHERE id = NEW.id;
END;

CREATE TRIGGER IF NOT EXISTS update_payouts_timestamp
AFTER UPDATE ON payouts
FOR EACH ROW
BEGIN
    UPDATE payouts SET updated_at = datetime('now') WHERE id = NEW.id;
END;

CREATE TRIGGER IF NOT EXISTS update_refunds_timestamp
AFTER UPDATE ON refunds
FOR EACH ROW
BEGIN
    UPDATE refunds SET updated_at = datetime('now') WHERE id = NEW.id;
END;

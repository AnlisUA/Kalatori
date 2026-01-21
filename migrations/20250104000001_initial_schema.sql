-- Financial amounts stored as TEXT to preserve decimal precision
-- All enums use CamelCase naming
-- UUID v4 stored as BLOB (16 bytes) for efficiency

-- Invoices table (replaces orders)
CREATE TABLE IF NOT EXISTS invoices (
    -- Identity
    id BLOB PRIMARY KEY NOT NULL,  -- UUID v4 - internal ID
    order_id TEXT NOT NULL UNIQUE,  -- Merchant-provided order ID

    -- Asset information (denormalized to avoid config changes affecting data)
    asset_id TEXT NOT NULL,
    asset_name TEXT NOT NULL,
    chain TEXT NOT NULL,

    -- Payment details
    amount TEXT NOT NULL,  -- Expected amount as decimal string (e.g., "123.456789")
    payment_address TEXT NOT NULL,

    -- Status (new unified status system)
    status TEXT NOT NULL CHECK(status IN (
        'Waiting', 'PartiallyPaid',  -- Active
        'Paid', 'OverPaid',  -- Final
        'UnpaidExpired', 'PartiallyPaidExpired',  -- Expired
        'CustomerCanceled', 'AdminCanceled'  -- Canceled
    )) DEFAULT 'Waiting',

    -- Cart metadata
    cart TEXT NOT NULL DEFAULT '{}',  -- JSONB: {cart_items?}

    -- Redirect URL
    redirect_url TEXT NOT NULL,

    -- Timestamps
    valid_till TEXT NOT NULL,  -- ISO 8601 datetime
    created_at TEXT NOT NULL DEFAULT (datetime('now')),  -- ISO 8601 datetime
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))   -- ISO 8601 datetime
);

-- Transactions table (unified: both pending and finalized transactions)
-- Replaces both old 'transactions' and 'pending_transactions' tables
CREATE TABLE IF NOT EXISTS transactions (
    -- Identity
    id BLOB PRIMARY KEY NOT NULL,  -- UUID v4
    invoice_id BLOB NOT NULL,  -- References invoices.id

    -- Asset information
    asset_id TEXT NOT NULL,
    asset_name TEXT NOT NULL,
    chain TEXT NOT NULL,
    amount TEXT NOT NULL,  -- Decimal string (excluding fees)

    -- Addresses (needed for refunds - sender is who we refund to)
    source_address TEXT NOT NULL,  -- For incoming: customer address (refund destination), for outgoing: payment address
    destination_address TEXT NOT NULL,  -- For incoming: payment address, for outgoing: payout/refund destination

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
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),

    FOREIGN KEY (invoice_id) REFERENCES invoices(id) ON DELETE CASCADE
);

-- Payouts table (NEW - replaces old implicit payout mechanism)
-- Payout = transfer from Payment Address to Payout Address (merchant's final wallet)
CREATE TABLE IF NOT EXISTS payouts (
    -- Identity
    id BLOB PRIMARY KEY NOT NULL,  -- UUID v4
    invoice_id BLOB NOT NULL,  -- References invoices.id

    -- Asset information
    asset_id TEXT NOT NULL,
    asset_name TEXT NOT NULL,
    chain TEXT NOT NULL,
    amount TEXT NOT NULL,  -- Decimal string

    -- Addresses
    source_address TEXT NOT NULL,  -- Payment address
    destination_address TEXT NOT NULL,  -- Payout address (merchant's final wallet)

    -- Initiator
    initiator_type TEXT NOT NULL CHECK(initiator_type IN ('System', 'Admin')),
    initiator_id TEXT,  -- Optional UUID v4 (NULL for System-initiated payouts)

    -- Status
    status TEXT NOT NULL CHECK(status IN ('Waiting', 'InProgress', 'Completed', 'FailedRetriable', 'Failed')) DEFAULT 'Waiting',

    -- Timestamps
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),

    -- Retry mechanism
    retry_count INTEGER NOT NULL DEFAULT 0,
    last_attempt_at TEXT,
    next_retry_at TEXT,
    failure_message TEXT,

    FOREIGN KEY (invoice_id) REFERENCES invoices(id) ON DELETE CASCADE
);

-- Refunds table (NEW - not in old system)
CREATE TABLE IF NOT EXISTS refunds (
    -- Identity
    id BLOB PRIMARY KEY NOT NULL,  -- UUID v4
    invoice_id BLOB NOT NULL,  -- References invoices.id

    -- Asset information
    asset_id TEXT NOT NULL,
    asset_name TEXT NOT NULL,
    chain TEXT NOT NULL,
    amount TEXT NOT NULL,  -- Decimal string

    -- Addresses
    source_address TEXT NOT NULL,  -- Payment address or reserve account
    destination_address TEXT NOT NULL,  -- Usually the sender from original payment

    -- Initiator
    initiator_type TEXT NOT NULL CHECK(initiator_type IN ('System', 'Admin')),
    initiator_id TEXT,  -- Optional UUID v4 (NULL for System)

    -- Status
    status TEXT NOT NULL CHECK(status IN ('Waiting', 'InProgress', 'Completed', 'FailedRetriable', 'Failed')) DEFAULT 'Waiting',

    -- Timestamps
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),

    -- Retry mechanism
    retry_count INTEGER NOT NULL DEFAULT 0,
    last_attempt_at TEXT,
    next_retry_at TEXT,
    failure_message TEXT,

    FOREIGN KEY (invoice_id) REFERENCES invoices(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS webhook_events (
    id BLOB PRIMARY KEY NOT NULL,  -- UUID v4
    entity_id BLOB NOT NULL,  -- References invoices.id, payouts.id, refunds.id, transactions.id
    payload TEXT NOT NULL,  -- JSONB payload
    sent INTEGER NOT NULL DEFAULT 0,  -- 0 = false, 1 = true
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
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

-- Webhook events queries
CREATE INDEX IF NOT EXISTS idx_webhook_events_sent_entity_created_id ON webhook_events(sent, entity_id, created_at, id);

-- ============================================================================
-- Status Transition Triggers
-- ============================================================================
-- These triggers enforce valid status transitions at the database level.
-- Error format: "ERROR_TYPE|old_status=VALUE|new_status=VALUE|id=VALUE"
-- This allows parsing in application code without additional diagnostic queries.

-- Invoice status transition enforcement
-- Valid transitions:
-- Waiting -> PartiallyPaid, Paid, OverPaid, UnpaidExpired, AdminCanceled, CustomerCanceled
-- PartiallyPaid -> Paid, OverPaid, PartiallyPaidExpired, AdminCanceled
-- All final/expired/canceled statuses -> no further transitions allowed
CREATE TRIGGER IF NOT EXISTS enforce_invoice_status_transition
BEFORE UPDATE OF status ON invoices
FOR EACH ROW
BEGIN
    SELECT CASE
        WHEN OLD.status = 'Waiting' AND NEW.status != OLD.status AND NEW.status NOT IN
            ('PartiallyPaid', 'Paid', 'OverPaid', 'UnpaidExpired', 'AdminCanceled', 'CustomerCanceled')
        THEN RAISE(ABORT, 'INVOICE_STATUS_TRANSITION|old_status=' || OLD.status || '|new_status=' || NEW.status)

        WHEN OLD.status = 'PartiallyPaid' AND NEW.status != OLD.status AND NEW.status NOT IN
            ('Paid', 'OverPaid', 'PartiallyPaidExpired', 'AdminCanceled')
        THEN RAISE(ABORT, 'INVOICE_STATUS_TRANSITION|old_status=' || OLD.status || '|new_status=' || NEW.status)

        WHEN OLD.status IN ('Paid', 'OverPaid', 'UnpaidExpired', 'PartiallyPaidExpired', 'CustomerCanceled', 'AdminCanceled')
            AND NEW.status != OLD.status
        THEN RAISE(ABORT, 'INVOICE_STATUS_TRANSITION|old_status=' || OLD.status || '|new_status=' || NEW.status)
    END;
END;

-- Payout status transition enforcement
CREATE TRIGGER IF NOT EXISTS enforce_payout_status_transition
BEFORE UPDATE OF status ON payouts
FOR EACH ROW
BEGIN
    SELECT CASE
        WHEN OLD.status = 'Waiting' AND NEW.status != OLD.status AND NEW.status NOT IN ('InProgress')
        THEN RAISE(ABORT, 'PAYOUT_STATUS_TRANSITION|old_status=' || OLD.status || '|new_status=' || NEW.status)

        WHEN OLD.status = 'InProgress' AND NEW.status != OLD.status AND NEW.status NOT IN ('Completed', 'FailedRetriable', 'Failed')
        THEN RAISE(ABORT, 'PAYOUT_STATUS_TRANSITION|old_status=' || OLD.status || '|new_status=' || NEW.status)

        WHEN OLD.status = 'FailedRetriable' AND NEW.status != OLD.status AND NEW.status NOT IN ('InProgress')
        THEN RAISE(ABORT, 'PAYOUT_STATUS_TRANSITION|old_status=' || OLD.status || '|new_status=' || NEW.status)

        WHEN OLD.status IN ('Completed', 'Failed') AND NEW.status != OLD.status
        THEN RAISE(ABORT, 'PAYOUT_STATUS_TRANSITION|old_status=' || OLD.status || '|new_status=' || NEW.status)
    END;
END;

-- Refund status transition enforcement
CREATE TRIGGER IF NOT EXISTS enforce_refund_status_transition
BEFORE UPDATE OF status ON refunds
FOR EACH ROW
BEGIN
    SELECT CASE
        WHEN OLD.status = 'Waiting' AND NEW.status != OLD.status AND NEW.status NOT IN ('InProgress')
        THEN RAISE(ABORT, 'REFUND_STATUS_TRANSITION|old_status=' || OLD.status || '|new_status=' || NEW.status)

        WHEN OLD.status = 'InProgress' AND NEW.status != OLD.status AND NEW.status NOT IN ('Completed', 'FailedRetriable', 'Failed')
        THEN RAISE(ABORT, 'REFUND_STATUS_TRANSITION|old_status=' || OLD.status || '|new_status=' || NEW.status)

        WHEN OLD.status = 'FailedRetriable' AND NEW.status != OLD.status AND NEW.status NOT IN ('InProgress')
        THEN RAISE(ABORT, 'REFUND_STATUS_TRANSITION|old_status=' || OLD.status || '|new_status=' || NEW.status)

        WHEN OLD.status IN ('Completed', 'Failed') AND NEW.status != OLD.status
        THEN RAISE(ABORT, 'REFUND_STATUS_TRANSITION|old_status=' || OLD.status || '|new_status=' || NEW.status)
    END;
END;

-- Transaction status transition enforcement
CREATE TRIGGER IF NOT EXISTS enforce_transaction_status_transition
BEFORE UPDATE OF status ON transactions
FOR EACH ROW
BEGIN
    SELECT CASE
        WHEN OLD.status = 'Waiting' AND NEW.status != OLD.status AND NEW.status NOT IN ('InProgress', 'Completed')
        THEN RAISE(ABORT, 'TRANSACTION_STATUS_TRANSITION|old_status=' || OLD.status || '|new_status=' || NEW.status)

        WHEN OLD.status = 'InProgress' AND NEW.status != OLD.status AND NEW.status NOT IN ('Completed', 'Failed')
        THEN RAISE(ABORT, 'TRANSACTION_STATUS_TRANSITION|old_status=' || OLD.status || '|new_status=' || NEW.status)

        WHEN OLD.status IN ('Completed', 'Failed') AND NEW.status != OLD.status
        THEN RAISE(ABORT, 'TRANSACTION_STATUS_TRANSITION|old_status=' || OLD.status || '|new_status=' || NEW.status)
    END;
END;

CREATE TRIGGER IF NOT EXISTS update_amount_and_cart_only_for_waiting_invoice
BEFORE UPDATE OF amount, cart ON invoices
FOR EACH ROW
BEGIN
    SELECT CASE
        WHEN OLD.status != 'Waiting'
        THEN RAISE(ABORT, 'INVOICE_UPDATE_NOT_ALLOWED|old_status=' || OLD.status)
    END;
END;

-- Migration: Add Polygon chain support
-- This migration updates CHECK constraints to allow 'Polygon' as a valid chain type.
-- Note: SQLite doesn't support ALTER CONSTRAINT, so we need to recreate tables or
-- use a workaround. Since CHECK constraints in SQLite are enforced at insert/update
-- time and we can't easily modify them, we'll:
-- 1. Create new tables with updated constraints
-- 2. Copy data from old tables
-- 3. Drop old tables
-- 4. Rename new tables

-- Disable foreign key checks during migration
PRAGMA foreign_keys = OFF;

-- ============================================================================
-- INVOICES TABLE
-- ============================================================================

CREATE TABLE invoices_new (
    id BLOB PRIMARY KEY NOT NULL,
    order_id TEXT NOT NULL UNIQUE,
    asset_id TEXT NOT NULL,
    asset_name TEXT NOT NULL,
    chain TEXT NOT NULL CHECK(chain IN ('PolkadotAssetHub', 'Polygon')),
    amount TEXT NOT NULL,
    payment_address TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN (
        'Waiting', 'PartiallyPaid',
        'Paid', 'OverPaid',
        'UnpaidExpired', 'PartiallyPaidExpired',
        'CustomerCanceled', 'AdminCanceled'
    )) DEFAULT 'Waiting',
    cart TEXT NOT NULL DEFAULT '{}',
    redirect_url TEXT NOT NULL,
    valid_till TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT INTO invoices_new SELECT * FROM invoices;
DROP TABLE invoices;
ALTER TABLE invoices_new RENAME TO invoices;

-- Recreate indexes for invoices
CREATE INDEX IF NOT EXISTS idx_invoices_order_id ON invoices(order_id);
CREATE INDEX IF NOT EXISTS idx_invoices_status ON invoices(status);
CREATE INDEX IF NOT EXISTS idx_invoices_created_at ON invoices(created_at);
CREATE INDEX IF NOT EXISTS idx_invoices_valid_till ON invoices(valid_till);
CREATE INDEX IF NOT EXISTS idx_invoices_status_created ON invoices(status, created_at);
CREATE INDEX IF NOT EXISTS idx_invoices_status_valid_till ON invoices(status, valid_till);
CREATE INDEX IF NOT EXISTS idx_invoices_payment_address ON invoices(payment_address);

-- ============================================================================
-- TRANSACTIONS TABLE
-- ============================================================================

CREATE TABLE transactions_new (
    id BLOB PRIMARY KEY NOT NULL,
    invoice_id BLOB NOT NULL,
    asset_id TEXT NOT NULL,
    asset_name TEXT NOT NULL,
    chain TEXT NOT NULL CHECK(chain IN ('PolkadotAssetHub', 'Polygon')),
    amount TEXT NOT NULL,
    source_address TEXT NOT NULL,
    destination_address TEXT NOT NULL,
    block_number INTEGER,
    position_in_block INTEGER,
    tx_hash TEXT,
    origin TEXT NOT NULL DEFAULT '{}',
    status TEXT NOT NULL CHECK(status IN ('Waiting', 'InProgress', 'Completed', 'Failed')) DEFAULT 'Waiting',
    transaction_type TEXT NOT NULL CHECK(transaction_type IN ('Incoming', 'Outgoing')),
    outgoing_meta TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (invoice_id) REFERENCES invoices(id) ON DELETE CASCADE
);

INSERT INTO transactions_new SELECT * FROM transactions;
DROP TABLE transactions;
ALTER TABLE transactions_new RENAME TO transactions;

-- Recreate indexes for transactions
CREATE INDEX IF NOT EXISTS idx_transactions_invoice_id ON transactions(invoice_id);
CREATE INDEX IF NOT EXISTS idx_transactions_type ON transactions(transaction_type);
CREATE INDEX IF NOT EXISTS idx_transactions_status ON transactions(status);
CREATE INDEX IF NOT EXISTS idx_transactions_created_at ON transactions(created_at);

-- ============================================================================
-- PAYOUTS TABLE
-- ============================================================================

CREATE TABLE payouts_new (
    id BLOB PRIMARY KEY NOT NULL,
    invoice_id BLOB NOT NULL,
    asset_id TEXT NOT NULL,
    asset_name TEXT NOT NULL,
    chain TEXT NOT NULL CHECK(chain IN ('PolkadotAssetHub', 'Polygon')),
    amount TEXT NOT NULL,
    source_address TEXT NOT NULL,
    destination_address TEXT NOT NULL,
    initiator_type TEXT NOT NULL CHECK(initiator_type IN ('System', 'Admin')),
    initiator_id TEXT,
    status TEXT NOT NULL CHECK(status IN ('Waiting', 'InProgress', 'Completed', 'FailedRetriable', 'Failed')) DEFAULT 'Waiting',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    retry_count INTEGER NOT NULL DEFAULT 0,
    last_attempt_at TEXT,
    next_retry_at TEXT,
    failure_message TEXT,
    FOREIGN KEY (invoice_id) REFERENCES invoices(id) ON DELETE CASCADE
);

INSERT INTO payouts_new SELECT * FROM payouts;
DROP TABLE payouts;
ALTER TABLE payouts_new RENAME TO payouts;

-- Recreate indexes for payouts
CREATE INDEX IF NOT EXISTS idx_payouts_invoice_id ON payouts(invoice_id);
CREATE INDEX IF NOT EXISTS idx_payouts_status ON payouts(status);
CREATE INDEX IF NOT EXISTS idx_payouts_created_at ON payouts(created_at);

-- ============================================================================
-- REFUNDS TABLE
-- ============================================================================

CREATE TABLE refunds_new (
    id BLOB PRIMARY KEY NOT NULL,
    invoice_id BLOB NOT NULL,
    asset_id TEXT NOT NULL,
    asset_name TEXT NOT NULL,
    chain TEXT NOT NULL CHECK(chain IN ('PolkadotAssetHub', 'Polygon')),
    amount TEXT NOT NULL,
    source_address TEXT NOT NULL,
    destination_address TEXT NOT NULL,
    initiator_type TEXT NOT NULL CHECK(initiator_type IN ('System', 'Admin')),
    initiator_id TEXT,
    status TEXT NOT NULL CHECK(status IN ('Waiting', 'InProgress', 'Completed', 'FailedRetriable', 'Failed')) DEFAULT 'Waiting',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    retry_count INTEGER NOT NULL DEFAULT 0,
    last_attempt_at TEXT,
    next_retry_at TEXT,
    failure_message TEXT,
    FOREIGN KEY (invoice_id) REFERENCES invoices(id) ON DELETE CASCADE
);

INSERT INTO refunds_new SELECT * FROM refunds;
DROP TABLE refunds;
ALTER TABLE refunds_new RENAME TO refunds;

-- Recreate indexes for refunds
CREATE INDEX IF NOT EXISTS idx_refunds_invoice_id ON refunds(invoice_id);
CREATE INDEX IF NOT EXISTS idx_refunds_status ON refunds(status);
CREATE INDEX IF NOT EXISTS idx_refunds_created_at ON refunds(created_at);

-- ============================================================================
-- RECREATE TRIGGERS
-- ============================================================================

-- Invoice status transition enforcement
DROP TRIGGER IF EXISTS enforce_invoice_status_transition;
CREATE TRIGGER enforce_invoice_status_transition
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
DROP TRIGGER IF EXISTS enforce_payout_status_transition;
CREATE TRIGGER enforce_payout_status_transition
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
DROP TRIGGER IF EXISTS enforce_refund_status_transition;
CREATE TRIGGER enforce_refund_status_transition
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
DROP TRIGGER IF EXISTS enforce_transaction_status_transition;
CREATE TRIGGER enforce_transaction_status_transition
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

-- Invoice update restriction trigger
DROP TRIGGER IF EXISTS update_amount_and_cart_only_for_waiting_invoice;
CREATE TRIGGER update_amount_and_cart_only_for_waiting_invoice
BEFORE UPDATE OF amount, cart ON invoices
FOR EACH ROW
BEGIN
    SELECT CASE
        WHEN OLD.status != 'Waiting'
        THEN RAISE(ABORT, 'INVOICE_UPDATE_NOT_ALLOWED|old_status=' || OLD.status)
    END;
END;

-- Re-enable foreign key checks
PRAGMA foreign_keys = ON;
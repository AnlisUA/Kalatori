-- Front-end swaps table
-- Tracks cross-chain or single-chain swap details for invoices paid via front-end bridge
CREATE TABLE IF NOT EXISTS front_end_swaps (
    -- Identity
    id BLOB PRIMARY KEY NOT NULL,  -- UUID v4
    invoice_id BLOB NOT NULL,  -- References invoices.id

    -- Swap details
    from_amount_units TEXT NOT NULL,  -- u128 stored as TEXT to preserve precision
    from_chain_id INTEGER NOT NULL,  -- Source chain ID (e.g., Ethereum = 1)
    from_asset_id TEXT NOT NULL,  -- Source asset contract address (hex)
    transaction_hash TEXT NOT NULL,  -- Source chain transaction hash

    -- Timestamps
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),

    FOREIGN KEY (invoice_id) REFERENCES invoices(id) ON DELETE CASCADE
);

-- Indexes
CREATE INDEX IF NOT EXISTS idx_front_end_swaps_invoice_id ON front_end_swaps(invoice_id);
CREATE INDEX IF NOT EXISTS idx_front_end_swaps_created_at ON front_end_swaps(created_at);

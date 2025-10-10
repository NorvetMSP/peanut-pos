-- Return policies scoped by tenant and optional location
CREATE TABLE IF NOT EXISTS return_policies (
    tenant_id UUID NOT NULL,
    location_id UUID NULL,
    allow_window_days INTEGER NOT NULL CHECK (allow_window_days >= 0 AND allow_window_days <= 3650),
    restock_fee_bps INTEGER NOT NULL CHECK (restock_fee_bps >= 0 AND restock_fee_bps <= 10000),
    receipt_required BOOLEAN NOT NULL DEFAULT TRUE,
    manager_override_allowed BOOLEAN NOT NULL DEFAULT TRUE,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, location_id)
);

CREATE INDEX IF NOT EXISTS idx_return_policies_tenant ON return_policies(tenant_id);
CREATE INDEX IF NOT EXISTS idx_return_policies_location ON return_policies(location_id);

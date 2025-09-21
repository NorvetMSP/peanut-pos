CREATE TABLE IF NOT EXISTS gdpr_tombstones (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    customer_id UUID,
    request_type TEXT NOT NULL CHECK (request_type IN ('export', 'delete')),
    status TEXT NOT NULL DEFAULT 'pending',
    requested_by UUID,
    requested_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    processed_at TIMESTAMPTZ,
    metadata JSONB
);

CREATE INDEX IF NOT EXISTS idx_gdpr_tombstones_tenant
    ON gdpr_tombstones (tenant_id);

CREATE INDEX IF NOT EXISTS idx_gdpr_tombstones_status
    ON gdpr_tombstones (status);

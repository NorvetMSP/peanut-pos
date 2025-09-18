CREATE TABLE integration_keys (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    label TEXT NOT NULL,
    api_key_hash TEXT NOT NULL UNIQUE,
    key_suffix VARCHAR(12) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at TIMESTAMPTZ
);

CREATE INDEX idx_integration_keys_tenant ON integration_keys(tenant_id);
CREATE INDEX idx_integration_keys_active ON integration_keys(api_key_hash) WHERE revoked_at IS NULL;

CREATE TABLE IF NOT EXISTS tenant_data_keys (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    key_version INTEGER NOT NULL CHECK (key_version > 0),
    encrypted_key BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    rotated_at TIMESTAMPTZ,
    active BOOLEAN NOT NULL DEFAULT TRUE
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_tenant_data_keys_tenant_active
    ON tenant_data_keys (tenant_id)
    WHERE active;

CREATE UNIQUE INDEX IF NOT EXISTS idx_tenant_data_keys_version
    ON tenant_data_keys (tenant_id, key_version);

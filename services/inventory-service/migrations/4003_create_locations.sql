-- 4003_create_locations.sql
-- Introduce per-tenant physical/virtual locations to support multi-location inventory.
-- Safe: purely additive.

CREATE EXTENSION IF NOT EXISTS "pgcrypto";

CREATE TABLE locations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL,
    code TEXT NOT NULL,                -- short stable identifier (e.g. "WH1", "STORE_012")
    name TEXT NOT NULL,                -- display name
    timezone TEXT NOT NULL DEFAULT 'UTC',
    active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (tenant_id, code)
);

CREATE INDEX idx_locations_tenant ON locations(tenant_id);

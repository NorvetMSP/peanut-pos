-- Per-tenant/location/pos tax rate overrides in basis points (0..10000)
CREATE TABLE IF NOT EXISTS tax_rate_overrides (
  tenant_id UUID NOT NULL,
  location_id UUID NULL,
  pos_instance_id UUID NULL,
  rate_bps INTEGER NOT NULL CHECK (rate_bps >= 0 AND rate_bps <= 10000),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Scoped uniqueness to prevent duplicates while allowing NULL semantics
-- 1) Unique per (tenant, pos) for pos-specific rows
CREATE UNIQUE INDEX IF NOT EXISTS uq_tax_rate_tenant_pos
  ON tax_rate_overrides(tenant_id, pos_instance_id)
  WHERE pos_instance_id IS NOT NULL;

-- 2) Unique per (tenant, location) for location rows where pos is NULL
CREATE UNIQUE INDEX IF NOT EXISTS uq_tax_rate_tenant_loc_nullpos
  ON tax_rate_overrides(tenant_id, location_id)
  WHERE location_id IS NOT NULL AND pos_instance_id IS NULL;

-- 3) Unique per tenant for global rows (both NULL)
CREATE UNIQUE INDEX IF NOT EXISTS uq_tax_rate_tenant_global
  ON tax_rate_overrides(tenant_id)
  WHERE location_id IS NULL AND pos_instance_id IS NULL;

-- Helpful indexes for lookup precedence (pos > location > tenant)
CREATE INDEX IF NOT EXISTS idx_tax_rate_overrides_tenant_pos ON tax_rate_overrides(tenant_id, pos_instance_id);
CREATE INDEX IF NOT EXISTS idx_tax_rate_overrides_tenant_loc ON tax_rate_overrides(tenant_id, location_id);
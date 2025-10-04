-- Per-tenant/location/pos tax rate overrides in basis points (0..10000)
CREATE TABLE IF NOT EXISTS tax_rate_overrides (
  tenant_id UUID NOT NULL,
  location_id UUID NULL,
  pos_instance_id UUID NULL,
  rate_bps INTEGER NOT NULL CHECK (rate_bps >= 0 AND rate_bps <= 10000),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  PRIMARY KEY (tenant_id, COALESCE(location_id, '00000000-0000-0000-0000-000000000000'::uuid), COALESCE(pos_instance_id, '00000000-0000-0000-0000-000000000000'::uuid))
);

-- Helpful indexes for lookup precedence (pos > location > tenant)
CREATE INDEX IF NOT EXISTS idx_tax_rate_overrides_tenant_pos ON tax_rate_overrides(tenant_id, pos_instance_id);
CREATE INDEX IF NOT EXISTS idx_tax_rate_overrides_tenant_loc ON tax_rate_overrides(tenant_id, location_id);
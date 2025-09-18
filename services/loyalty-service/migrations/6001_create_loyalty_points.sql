CREATE TABLE IF NOT EXISTS loyalty_points (
    customer_id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    points INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_loyalty_points_tenant ON loyalty_points(tenant_id);

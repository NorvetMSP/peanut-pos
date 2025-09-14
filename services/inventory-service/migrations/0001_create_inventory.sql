CREATE TABLE inventory (
    product_id UUID NOT NULL,
    tenant_id UUID NOT NULL,
    quantity INTEGER NOT NULL DEFAULT 0,
    threshold INTEGER NOT NULL DEFAULT 5,
    PRIMARY KEY (product_id, tenant_id)
);
CREATE INDEX idx_inventory_tenant ON inventory(tenant_id);

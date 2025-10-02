-- 4004_create_inventory_items_multilocation.sql
-- Multi-location inventory table. Does not drop legacy single-row inventory yet.
-- Phase 1: dual-write (optional) behind feature flag.

CREATE TABLE inventory_items (
    tenant_id UUID NOT NULL,
    product_id UUID NOT NULL,
    location_id UUID NOT NULL REFERENCES locations(id) ON DELETE CASCADE,
    quantity INTEGER NOT NULL DEFAULT 0,
    threshold INTEGER NOT NULL DEFAULT 5,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, product_id, location_id)
);

CREATE INDEX idx_inventory_items_tenant ON inventory_items(tenant_id);
CREATE INDEX idx_inventory_items_product ON inventory_items(product_id);

-- Reservation status enum via lightweight table (portable if enum migrations are avoided):
CREATE TABLE reservation_status (
    status TEXT PRIMARY KEY
);
INSERT INTO reservation_status(status) VALUES
    ('ACTIVE'), ('RELEASED'), ('EXPIRED'), ('CONSUMED')
ON CONFLICT DO NOTHING;

-- Augment reservations with location + status + expiry.
ALTER TABLE inventory_reservations
    ADD COLUMN location_id UUID NULL REFERENCES locations(id) ON DELETE SET NULL,
    ADD COLUMN status TEXT NOT NULL DEFAULT 'ACTIVE' REFERENCES reservation_status(status),
    ADD COLUMN expires_at TIMESTAMPTZ NULL;

CREATE INDEX IF NOT EXISTS idx_inventory_reservations_expires ON inventory_reservations (expires_at) WHERE expires_at IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_inventory_reservations_status ON inventory_reservations (status);

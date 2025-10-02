-- 4005_backfill_inventory_items.sql
-- Backfill script: for each legacy inventory row create a default location row and matching inventory_items rows.
-- Idempotent: uses ON CONFLICT DO NOTHING.
-- Assumes a synthetic DEFAULT location per tenant.

-- Create default location per tenant if none exists.
WITH tenants AS (
    SELECT DISTINCT tenant_id FROM inventory
)
INSERT INTO locations (tenant_id, code, name, timezone)
SELECT t.tenant_id, 'DEFAULT', 'Default Location', 'UTC'
FROM tenants t
LEFT JOIN locations l ON l.tenant_id = t.tenant_id AND l.code = 'DEFAULT'
WHERE l.id IS NULL;

-- Insert inventory_items rows mapped to DEFAULT location.
INSERT INTO inventory_items (tenant_id, product_id, location_id, quantity, threshold)
SELECT inv.tenant_id, inv.product_id, l.id, inv.quantity, inv.threshold
FROM inventory inv
JOIN locations l ON l.tenant_id = inv.tenant_id AND l.code = 'DEFAULT'
ON CONFLICT (tenant_id, product_id, location_id) DO NOTHING;

ALTER TABLE orders ADD COLUMN store_id UUID;
ALTER TABLE orders ADD COLUMN customer_name TEXT;
ALTER TABLE orders ADD COLUMN customer_email TEXT;

ALTER TABLE order_items ADD COLUMN product_name TEXT;

CREATE INDEX IF NOT EXISTS idx_orders_tenant_created_at ON orders (tenant_id, created_at DESC);

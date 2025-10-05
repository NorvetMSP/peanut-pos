ALTER TABLE orders ADD COLUMN IF NOT EXISTS exchange_of_order_id UUID NULL REFERENCES orders(id) ON DELETE SET NULL;
CREATE INDEX IF NOT EXISTS idx_orders_exchange_of ON orders(exchange_of_order_id);

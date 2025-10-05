-- Basic payments table for MVP
CREATE TABLE IF NOT EXISTS payments (
  id UUID PRIMARY KEY,
  tenant_id UUID NOT NULL,
  order_id UUID NOT NULL,
  method TEXT NOT NULL, -- 'cash','card'
  amount NUMERIC NOT NULL,
  status TEXT NOT NULL, -- 'captured','voided','failed'
  change_cents INTEGER NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  CONSTRAINT fk_payments_order FOREIGN KEY (order_id) REFERENCES orders(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_payments_tenant ON payments(tenant_id, created_at);
CREATE INDEX IF NOT EXISTS idx_payments_order ON payments(order_id);

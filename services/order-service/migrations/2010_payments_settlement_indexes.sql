-- Composite index covering tenant_id, created_at, and method; partial on captured.
-- Note: avoid expression indexes that require IMMUTABLE functions when using timestamptz.
CREATE INDEX IF NOT EXISTS idx_payments_settlement_tenant_created_method_captured
ON payments (tenant_id, created_at, method)
WHERE status = 'captured';

-- Supporting index if the planner prefers narrower index without method for scans.
CREATE INDEX IF NOT EXISTS idx_payments_settlement_tenant_created_captured
ON payments (tenant_id, created_at)
WHERE status = 'captured';

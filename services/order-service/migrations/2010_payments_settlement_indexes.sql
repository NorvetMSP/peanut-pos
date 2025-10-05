-- Settlement report performance indexes
-- Optimizes query:
-- SELECT method, COUNT(*), SUM(amount) FROM payments
-- WHERE tenant_id = $1 AND status = 'captured' AND created_at::date = $2
-- GROUP BY method

-- Composite index covering tenant_id, created_at date (via expression), and method for grouping; partial on captured.
CREATE INDEX IF NOT EXISTS idx_payments_settlement_tenant_date_method_captured
ON payments (tenant_id, (date_trunc('day', created_at)), method)
WHERE status = 'captured';

-- Fallback simpler partial index by tenant and day if planner prefers it.
CREATE INDEX IF NOT EXISTS idx_payments_settlement_tenant_day_captured
ON payments (tenant_id, (date_trunc('day', created_at)))
WHERE status = 'captured';

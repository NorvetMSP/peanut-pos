-- Dedicated migration for shared audit_events read model (TA-AUD-2)
CREATE TABLE IF NOT EXISTS audit_events (
    event_id UUID PRIMARY KEY,
    event_version INT NOT NULL,
    tenant_id UUID NOT NULL,
    actor_id UUID NULL,
    actor_name TEXT NULL,
    actor_email TEXT NULL,
    entity_type TEXT NOT NULL,
    entity_id UUID NULL,
    action TEXT NOT NULL,
    severity TEXT NOT NULL,
    source_service TEXT NOT NULL,
    occurred_at TIMESTAMPTZ NOT NULL,
    trace_id UUID NULL,
    payload JSONB NOT NULL,
    meta JSONB NOT NULL,
    ingested_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_audit_events_tenant_time ON audit_events (tenant_id, occurred_at DESC);
CREATE INDEX IF NOT EXISTS idx_audit_events_tenant_action ON audit_events (tenant_id, action);
CREATE INDEX IF NOT EXISTS idx_audit_events_tenant_entity ON audit_events (tenant_id, entity_type, entity_id);
CREATE INDEX IF NOT EXISTS idx_audit_events_trace ON audit_events (trace_id);
CREATE INDEX IF NOT EXISTS idx_audit_events_severity ON audit_events (severity);

-- Inbox de-dup table for idempotent consumption (analytics)
CREATE TABLE IF NOT EXISTS inbox (
    id BIGSERIAL PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    topic TEXT NOT NULL,
    message_key TEXT NOT NULL,
    first_seen TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (tenant_id, topic, message_key)
);

CREATE INDEX IF NOT EXISTS idx_inbox_tenant_topic ON inbox(tenant_id, topic);

-- 4007_create_inbox_table.sql
-- Inbox table for consumer de-duplication in inventory-service

BEGIN;

CREATE TABLE IF NOT EXISTS inbox (
  id BIGSERIAL PRIMARY KEY,
  tenant_id TEXT NOT NULL,
  message_key TEXT NOT NULL,
  topic TEXT NOT NULL,
  seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  UNIQUE (tenant_id, message_key, topic)
);

COMMIT;
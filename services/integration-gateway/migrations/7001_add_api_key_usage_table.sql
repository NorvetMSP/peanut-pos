CREATE TABLE IF NOT EXISTS api_key_usage (
    tenant_id UUID NOT NULL,
    key_hash TEXT NOT NULL,
    key_suffix TEXT NOT NULL,
    window_start TIMESTAMPTZ NOT NULL,
    window_end TIMESTAMPTZ NOT NULL,
    request_count BIGINT NOT NULL,
    rejected_count BIGINT NOT NULL DEFAULT 0,
    last_seen_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (tenant_id, key_hash, window_start)
);

CREATE INDEX IF NOT EXISTS idx_api_key_usage_window_end
    ON api_key_usage (window_end DESC);

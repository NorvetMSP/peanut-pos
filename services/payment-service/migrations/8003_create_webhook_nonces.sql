-- 8003: Webhook nonce replay protection

CREATE TABLE IF NOT EXISTS webhook_nonces (
    nonce TEXT PRIMARY KEY,
    ts    TIMESTAMPTZ NOT NULL DEFAULT now(),
    provider TEXT
);

CREATE INDEX IF NOT EXISTS idx_webhook_nonces_ts ON webhook_nonces(ts);

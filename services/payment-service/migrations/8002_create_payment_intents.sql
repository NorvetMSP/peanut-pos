-- 8002: create payment_intents table (MVP)
-- Note: aligns with P6-01 (Payment intent model)

CREATE TABLE IF NOT EXISTS payment_intents (
    id                  TEXT PRIMARY KEY,
    order_id            TEXT NOT NULL,
    amount_minor        BIGINT NOT NULL,
    currency            TEXT NOT NULL,
    state               TEXT NOT NULL CHECK (state IN ('created','authorized','captured','refunded','voided','failed')),
    provider            TEXT,
    provider_ref        TEXT,
    idempotency_key     TEXT,
    metadata_json       JSONB,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_payment_intents_order_id ON payment_intents(order_id);
CREATE UNIQUE INDEX IF NOT EXISTS uq_payment_intents_idempotency ON payment_intents(idempotency_key) WHERE idempotency_key IS NOT NULL;

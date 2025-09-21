ALTER TABLE customers
    ADD COLUMN email_encrypted BYTEA,
    ADD COLUMN phone_encrypted BYTEA,
    ADD COLUMN email_hash BYTEA,
    ADD COLUMN phone_hash BYTEA,
    ADD COLUMN pii_key_version INTEGER,
    ADD COLUMN pii_encrypted_at TIMESTAMPTZ;

DROP INDEX IF EXISTS idx_customers_email_unique;
DROP INDEX IF EXISTS idx_customers_phone_unique;

CREATE UNIQUE INDEX IF NOT EXISTS idx_customers_email_hash_unique
    ON customers (tenant_id, email_hash)
    WHERE email_hash IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS idx_customers_phone_hash_unique
    ON customers (tenant_id, phone_hash)
    WHERE phone_hash IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_customers_email_hash
    ON customers (tenant_id, email_hash)
    WHERE email_hash IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_customers_phone_hash
    ON customers (tenant_id, phone_hash)
    WHERE phone_hash IS NOT NULL;

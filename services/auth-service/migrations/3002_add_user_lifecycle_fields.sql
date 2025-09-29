ALTER TABLE users
    ADD COLUMN created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    ADD COLUMN is_active BOOLEAN NOT NULL DEFAULT TRUE,
    ADD COLUMN last_password_reset TIMESTAMPTZ,
    ADD COLUMN force_password_reset BOOLEAN NOT NULL DEFAULT FALSE;

CREATE INDEX idx_users_tenant_active ON users (tenant_id, is_active);

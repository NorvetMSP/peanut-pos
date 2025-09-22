CREATE TABLE auth_refresh_tokens (
    jti UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    token_hash BYTEA NOT NULL,
    issued_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ
);

CREATE INDEX idx_auth_refresh_tokens_user ON auth_refresh_tokens (user_id);
CREATE INDEX idx_auth_refresh_tokens_expires ON auth_refresh_tokens (expires_at);

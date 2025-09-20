CREATE TABLE auth_signing_keys (
    kid TEXT PRIMARY KEY,
    public_pem TEXT NOT NULL,
    private_pem TEXT NOT NULL,
    alg TEXT NOT NULL DEFAULT 'RS256',
    n TEXT NOT NULL,
    e TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    rotated_at TIMESTAMPTZ,
    active BOOLEAN NOT NULL DEFAULT TRUE
);

CREATE UNIQUE INDEX idx_auth_signing_keys_active_true ON auth_signing_keys (active) WHERE active;

# Payment Webhooks — HMAC Verification Plan (P6-02)

Goal

- Verify provider webhooks with HMAC signatures, reject replays, and support secret rotation with minimal downtime.

Headers (proposed)

- `X-Signature`: `sha256=<hex>`
- `X-Timestamp`: epoch seconds
- `X-Nonce`: UUID v4
- Optional provider header passthrough, e.g., `X-Provider: valor|stripe|...`

Canonical string

- Join values with newlines; body hash is lowercase hex SHA-256 of raw body bytes:
  - `ts:<X-Timestamp>`
  - `nonce:<X-Nonce>`
  - `body_sha256:<sha256(body)>`

HMAC

- Algorithm: HMAC-SHA256(secret, canonicalString) → hex lower
- Compare using constant-time equality

Replay protection

- Table: `webhook_nonces(nonce TEXT PK, ts TIMESTAMPTZ NOT NULL, provider TEXT NULL)`
- Reject if nonce exists or timestamp skew is outside ±300s (configurable)
- TTL job: delete nonces older than 24h

Secret management & rotation

- Store active secret and optional next secret per provider (env or Vault)
- Verify against active; if fail, try next
- Rotate by setting `next`, deploy, then promote `next` to `active`

Middleware shape (Axum)

- Extract headers and body bytes (buffering limited size)
- Build canonical string, compute HMAC, check timestamp skew and nonce
- On success, insert nonce (idempotent) and pass along; else 401 with error code

Error codes

- `sig_missing`, `sig_mismatch`, `timestamp_skew`, `nonce_replay`, `malformed`

Testing

- Unit tests with fixed inputs/vectors and known secrets
- Negative tests (bad sig, skew, replay)

Next

- Implement in `payment-service` middleware; add provider-specific adapters if needed

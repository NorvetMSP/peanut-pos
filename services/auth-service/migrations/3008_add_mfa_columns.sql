ALTER TABLE users
    ADD COLUMN mfa_secret TEXT,
    ADD COLUMN mfa_pending_secret TEXT,
    ADD COLUMN mfa_enrolled_at TIMESTAMPTZ,
    ADD COLUMN mfa_failed_attempts SMALLINT NOT NULL DEFAULT 0,
    ADD COLUMN mfa_last_challenge_at TIMESTAMPTZ;

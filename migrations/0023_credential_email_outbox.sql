-- V2.3: encrypted, durable delivery of generated temporary credentials.
CREATE TABLE latex_core.email_outbox (
    id UUID PRIMARY KEY,
    recipient_email TEXT NOT NULL,
    email_type TEXT NOT NULL,
    account_user_id UUID NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    credential_role TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'PENDING',
    attempts INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    claimed_at TIMESTAMPTZ NULL,
    last_error TEXT NULL,
    secret_ciphertext BYTEA NULL,
    secret_nonce BYTEA NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    sent_at TIMESTAMPTZ NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    CONSTRAINT email_outbox_recipient_length CHECK (char_length(recipient_email) BETWEEN 3 AND 320),
    CONSTRAINT email_outbox_type CHECK (email_type IN ('TEMPORARY_CREDENTIAL')),
    CONSTRAINT email_outbox_role CHECK (credential_role IN ('student','mentor')),
    CONSTRAINT email_outbox_status CHECK (status IN ('PENDING','SENDING','SENT','FAILED','EXPIRED')),
    CONSTRAINT email_outbox_attempts_nonnegative CHECK (attempts >= 0),
    CONSTRAINT email_outbox_secret_pair CHECK (
        (secret_ciphertext IS NULL AND secret_nonce IS NULL) OR
        (secret_ciphertext IS NOT NULL AND secret_nonce IS NOT NULL)
    )
);

CREATE INDEX email_outbox_claim_idx
    ON latex_core.email_outbox (next_attempt_at, created_at)
    WHERE status = 'PENDING';
CREATE INDEX email_outbox_account_idx
    ON latex_core.email_outbox (account_user_id, created_at DESC);
CREATE INDEX email_outbox_status_idx
    ON latex_core.email_outbox (status, created_at DESC);

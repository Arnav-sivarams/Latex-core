-- V2.2: one-time imported credentials and mandatory first-login password change.
ALTER TABLE latex_core.user_credentials
    ADD COLUMN must_change_password BOOLEAN NOT NULL DEFAULT FALSE;

CREATE INDEX user_credentials_temporary_state_idx
    ON latex_core.user_credentials (must_change_password, user_id)
    WHERE must_change_password;

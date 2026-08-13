ALTER TABLE latex_core.user_credentials
    ADD COLUMN enabled BOOLEAN NOT NULL DEFAULT TRUE;

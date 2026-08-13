-- M10: account credentials, opaque sessions, and project presentation metadata.
CREATE TABLE latex_core.user_credentials (
    user_id UUID PRIMARY KEY REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    email TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT user_credentials_email_length CHECK (char_length(email) BETWEEN 3 AND 320),
    CONSTRAINT user_credentials_password_hash_nonempty CHECK (char_length(password_hash) BETWEEN 1 AND 1024)
);

CREATE TABLE latex_core.sessions (
    token_digest TEXT PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT sessions_digest_format CHECK (token_digest ~ '^[0-9a-f]{64}$')
);
CREATE INDEX sessions_user_expiry_idx ON latex_core.sessions (user_id, expires_at);

CREATE TABLE latex_core.projects (
    workspace_id UUID PRIMARY KEY REFERENCES latex_core.workspaces(id) ON DELETE RESTRICT,
    owner_user_id UUID NOT NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT projects_name_length CHECK (char_length(name) BETWEEN 1 AND 200)
);
CREATE INDEX projects_owner_updated_idx ON latex_core.projects (owner_user_id, updated_at DESC);

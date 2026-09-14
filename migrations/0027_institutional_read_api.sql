-- Authenticated, read-only institutional integration clients. Secrets are
-- returned once by the application; only SHA-256 verifiers are persisted.
CREATE TABLE latex_core.integration_clients (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    token_prefix TEXT NOT NULL,
    token_hash BYTEA NOT NULL UNIQUE,
    scopes TEXT[] NOT NULL,
    institution_wide BOOLEAN NOT NULL DEFAULT FALSE,
    report_ids UUID[] NOT NULL DEFAULT ARRAY[]::UUID[],
    expires_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    created_by_admin_user_id UUID NOT NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    rotated_at TIMESTAMPTZ,
    last_used_at TIMESTAMPTZ,
    CONSTRAINT integration_clients_name_length CHECK (char_length(name) BETWEEN 1 AND 120),
    CONSTRAINT integration_clients_prefix_length CHECK (char_length(token_prefix) BETWEEN 8 AND 32),
    CONSTRAINT integration_clients_hash_length CHECK (octet_length(token_hash) = 32),
    CONSTRAINT integration_clients_scope_values CHECK (scopes <@ ARRAY[
        'institution.directory.read', 'institution.contacts.read',
        'reports.read', 'reports.files.read', 'reports.pdf.read',
        'reviews.published.read'
    ]::TEXT[]),
    CONSTRAINT integration_clients_coverage CHECK (institution_wide OR cardinality(report_ids) > 0)
);
CREATE INDEX integration_clients_active_hash_idx
    ON latex_core.integration_clients (token_hash)
    WHERE revoked_at IS NULL;

CREATE TABLE latex_core.integration_access_log (
    id BIGSERIAL PRIMARY KEY,
    client_id UUID NOT NULL REFERENCES latex_core.integration_clients(id) ON DELETE RESTRICT,
    method TEXT NOT NULL,
    route_name TEXT NOT NULL,
    resource_type TEXT,
    resource_id TEXT,
    outcome TEXT NOT NULL,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT integration_access_method_read_only CHECK (method = 'GET'),
    CONSTRAINT integration_access_outcome_values CHECK (outcome IN ('ALLOWED', 'DENIED')),
    CONSTRAINT integration_access_route_length CHECK (char_length(route_name) BETWEEN 1 AND 120),
    CONSTRAINT integration_access_resource_length CHECK (resource_id IS NULL OR char_length(resource_id) <= 200)
);
CREATE INDEX integration_access_client_time_idx
    ON latex_core.integration_access_log (client_id, occurred_at DESC, id DESC);

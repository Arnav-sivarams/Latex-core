CREATE SCHEMA latex_core;

CREATE TABLE latex_core.tenants (
    id UUID PRIMARY KEY,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE latex_core.users (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT users_tenant_fk FOREIGN KEY (tenant_id) REFERENCES latex_core.tenants(id) ON DELETE RESTRICT
);
CREATE INDEX users_tenant_id_idx ON latex_core.users (tenant_id);

CREATE TABLE latex_core.workspaces (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    owner_user_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT workspaces_tenant_fk FOREIGN KEY (tenant_id) REFERENCES latex_core.tenants(id) ON DELETE RESTRICT,
    CONSTRAINT workspaces_owner_user_fk FOREIGN KEY (owner_user_id) REFERENCES latex_core.users(id) ON DELETE RESTRICT
);
CREATE INDEX workspaces_tenant_id_idx ON latex_core.workspaces (tenant_id);
CREATE INDEX workspaces_owner_user_id_idx ON latex_core.workspaces (owner_user_id);

CREATE TABLE latex_core.workspace_events (
    workspace_id UUID NOT NULL,
    sequence BIGINT NOT NULL,
    event_id UUID NOT NULL,
    base_version BIGINT NOT NULL,
    event_type TEXT NOT NULL,
    event_schema_version INTEGER NOT NULL,
    payload JSONB NOT NULL,
    created_by_user_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT workspace_events_pk PRIMARY KEY (workspace_id, sequence),
    CONSTRAINT workspace_events_event_id_unique UNIQUE (event_id),
    CONSTRAINT workspace_events_sequence_positive CHECK (sequence >= 1),
    CONSTRAINT workspace_events_base_version_nonnegative CHECK (base_version >= 0),
    CONSTRAINT workspace_events_sequence_matches_base CHECK (sequence = base_version + 1),
    CONSTRAINT workspace_events_schema_version_positive CHECK (event_schema_version >= 1),
    CONSTRAINT workspace_events_type_length CHECK (char_length(event_type) BETWEEN 1 AND 64),
    CONSTRAINT workspace_events_payload_object CHECK (jsonb_typeof(payload) = 'object'),
    CONSTRAINT workspace_events_workspace_fk FOREIGN KEY (workspace_id) REFERENCES latex_core.workspaces(id) ON DELETE RESTRICT,
    CONSTRAINT workspace_events_user_fk FOREIGN KEY (created_by_user_id) REFERENCES latex_core.users(id) ON DELETE RESTRICT
);
CREATE INDEX workspace_events_workspace_created_idx ON latex_core.workspace_events (workspace_id, created_at);

CREATE TABLE latex_core.snapshots (
    snapshot_id TEXT PRIMARY KEY,
    manifest_blob_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT snapshots_id_digest CHECK (snapshot_id ~ '^[0-9a-f]{64}$'),
    CONSTRAINT snapshots_manifest_digest CHECK (manifest_blob_hash ~ '^[0-9a-f]{64}$')
);

CREATE TABLE latex_core.workspace_snapshots (
    workspace_id UUID NOT NULL,
    workspace_version BIGINT NOT NULL,
    snapshot_id TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT workspace_snapshots_pk PRIMARY KEY (workspace_id, workspace_version),
    CONSTRAINT workspace_snapshots_version_nonnegative CHECK (workspace_version >= 0),
    CONSTRAINT workspace_snapshots_workspace_fk FOREIGN KEY (workspace_id) REFERENCES latex_core.workspaces(id) ON DELETE RESTRICT,
    CONSTRAINT workspace_snapshots_snapshot_fk FOREIGN KEY (snapshot_id) REFERENCES latex_core.snapshots(snapshot_id) ON DELETE RESTRICT
);
CREATE INDEX workspace_snapshots_snapshot_id_idx ON latex_core.workspace_snapshots (snapshot_id);
CREATE INDEX workspace_snapshots_workspace_snapshot_idx ON latex_core.workspace_snapshots (workspace_id, snapshot_id);

CREATE TABLE latex_core.workspace_heads (
    workspace_id UUID PRIMARY KEY,
    durable_version BIGINT NOT NULL DEFAULT 0,
    latest_snapshot_version BIGINT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT workspace_heads_durable_nonnegative CHECK (durable_version >= 0),
    CONSTRAINT workspace_heads_snapshot_nonnegative CHECK (latest_snapshot_version IS NULL OR latest_snapshot_version >= 0),
    CONSTRAINT workspace_heads_snapshot_not_ahead CHECK (latest_snapshot_version IS NULL OR latest_snapshot_version <= durable_version),
    CONSTRAINT workspace_heads_workspace_fk FOREIGN KEY (workspace_id) REFERENCES latex_core.workspaces(id) ON DELETE RESTRICT,
    CONSTRAINT workspace_heads_snapshot_fk FOREIGN KEY (workspace_id, latest_snapshot_version) REFERENCES latex_core.workspace_snapshots(workspace_id, workspace_version) ON DELETE RESTRICT
);

CREATE TABLE latex_core.compile_jobs (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    user_id UUID NOT NULL,
    workspace_id UUID NOT NULL,
    snapshot_id TEXT NOT NULL,
    compile_key TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    engine TEXT NOT NULL,
    tex_environment_id TEXT NOT NULL,
    latexmk_profile TEXT NOT NULL,
    shell_policy TEXT NOT NULL,
    synctex BOOLEAN NOT NULL,
    cost_class TEXT NOT NULL,
    priority SMALLINT NOT NULL DEFAULT 0,
    state TEXT NOT NULL,
    worker_id UUID NULL,
    lease_until TIMESTAMPTZ NULL,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    claimed_at TIMESTAMPTZ NULL,
    started_at TIMESTAMPTZ NULL,
    finished_at TIMESTAMPTZ NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_error JSONB NULL,
    CONSTRAINT compile_jobs_user_idempotency_unique UNIQUE (user_id, idempotency_key),
    CONSTRAINT compile_jobs_snapshot_digest CHECK (snapshot_id ~ '^[0-9a-f]{64}$'),
    CONSTRAINT compile_jobs_compile_digest CHECK (compile_key ~ '^[0-9a-f]{64}$'),
    CONSTRAINT compile_jobs_idempotency_format CHECK (idempotency_key ~ '^[A-Za-z0-9._:-]{1,128}$'),
    CONSTRAINT compile_jobs_engine_values CHECK (engine IN ('pdflatex', 'lualatex', 'xelatex')),
    CONSTRAINT compile_jobs_shell_values CHECK (shell_policy IN ('safe', 'restricted', 'compatibility')),
    CONSTRAINT compile_jobs_cost_values CHECK (cost_class IN ('small', 'normal', 'heavy')),
    CONSTRAINT compile_jobs_state_values CHECK (state IN ('queued', 'claimed', 'running', 'succeeded', 'failed', 'cancelled')),
    CONSTRAINT compile_jobs_tex_environment_format CHECK (tex_environment_id ~ '^[A-Za-z0-9._:@+-]{1,128}$'),
    CONSTRAINT compile_jobs_latexmk_profile_format CHECK (latexmk_profile ~ '^[a-z0-9._-]{1,64}$'),
    CONSTRAINT compile_jobs_attempt_nonnegative CHECK (attempt_count >= 0),
    CONSTRAINT compile_jobs_last_error_object CHECK (last_error IS NULL OR jsonb_typeof(last_error) = 'object'),
    CONSTRAINT compile_jobs_active_lease CHECK (state NOT IN ('claimed', 'running') OR (worker_id IS NOT NULL AND lease_until IS NOT NULL)),
    CONSTRAINT compile_jobs_queued_no_lease CHECK (state <> 'queued' OR (worker_id IS NULL AND lease_until IS NULL)),
    CONSTRAINT compile_jobs_tenant_fk FOREIGN KEY (tenant_id) REFERENCES latex_core.tenants(id) ON DELETE RESTRICT,
    CONSTRAINT compile_jobs_user_fk FOREIGN KEY (user_id) REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    CONSTRAINT compile_jobs_workspace_fk FOREIGN KEY (workspace_id) REFERENCES latex_core.workspaces(id) ON DELETE RESTRICT,
    CONSTRAINT compile_jobs_snapshot_fk FOREIGN KEY (snapshot_id) REFERENCES latex_core.snapshots(snapshot_id) ON DELETE RESTRICT
);
CREATE INDEX compile_jobs_queue_idx ON latex_core.compile_jobs (cost_class, priority DESC, created_at, id) WHERE state = 'queued';
CREATE INDEX compile_jobs_expired_lease_idx ON latex_core.compile_jobs (lease_until) WHERE state IN ('claimed', 'running');
CREATE INDEX compile_jobs_user_state_idx ON latex_core.compile_jobs (user_id, state);
CREATE INDEX compile_jobs_workspace_idx ON latex_core.compile_jobs (workspace_id);
CREATE INDEX compile_jobs_compile_key_idx ON latex_core.compile_jobs (compile_key);

CREATE TABLE latex_core.compilation_artifacts (
    artifact_id UUID PRIMARY KEY,
    job_id UUID NOT NULL,
    compile_key TEXT NOT NULL,
    kind TEXT NOT NULL,
    logical_name TEXT NOT NULL,
    blob_hash TEXT NOT NULL,
    size_bytes BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT compilation_artifacts_job_kind_name_unique UNIQUE (job_id, kind, logical_name),
    CONSTRAINT compilation_artifacts_compile_digest CHECK (compile_key ~ '^[0-9a-f]{64}$'),
    CONSTRAINT compilation_artifacts_blob_digest CHECK (blob_hash ~ '^[0-9a-f]{64}$'),
    CONSTRAINT compilation_artifacts_kind_values CHECK (kind IN ('pdf', 'log', 'synctex', 'fls', 'aux', 'bcf', 'toc', 'other')),
    CONSTRAINT compilation_artifacts_name_nonempty CHECK (char_length(logical_name) > 0),
    CONSTRAINT compilation_artifacts_size_nonnegative CHECK (size_bytes >= 0),
    CONSTRAINT compilation_artifacts_job_fk FOREIGN KEY (job_id) REFERENCES latex_core.compile_jobs(id) ON DELETE RESTRICT
);
CREATE INDEX compilation_artifacts_job_idx ON latex_core.compilation_artifacts (job_id);
CREATE INDEX compilation_artifacts_compile_key_idx ON latex_core.compilation_artifacts (compile_key);
CREATE INDEX compilation_artifacts_blob_hash_idx ON latex_core.compilation_artifacts (blob_hash);

CREATE TABLE latex_core.compile_cache (
    compile_key TEXT PRIMARY KEY,
    source_job_id UUID NOT NULL,
    artifact_manifest_blob_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_accessed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT compile_cache_key_digest CHECK (compile_key ~ '^[0-9a-f]{64}$'),
    CONSTRAINT compile_cache_manifest_digest CHECK (artifact_manifest_blob_hash ~ '^[0-9a-f]{64}$'),
    CONSTRAINT compile_cache_source_job_fk FOREIGN KEY (source_job_id) REFERENCES latex_core.compile_jobs(id) ON DELETE RESTRICT
);
CREATE INDEX compile_cache_last_accessed_idx ON latex_core.compile_cache (last_accessed_at);

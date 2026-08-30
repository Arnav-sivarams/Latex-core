-- S4: immutable V2 paper versions and exact-state build scheduling around the
-- existing durable compile queue. All history and successful artifacts remain
-- append-only.
CREATE TABLE latex_core.paper_versions (
    id UUID PRIMARY KEY,
    paper_id UUID NOT NULL,
    workspace_id UUID NOT NULL REFERENCES latex_core.workspaces(id) ON DELETE RESTRICT,
    document_epoch BIGINT NOT NULL,
    version_number BIGINT NOT NULL,
    version_type TEXT NOT NULL,
    name TEXT NULL,
    created_by_user_id UUID NOT NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    workspace_version BIGINT NOT NULL,
    snapshot_id TEXT NOT NULL REFERENCES latex_core.snapshots(snapshot_id) ON DELETE RESTRICT,
    manifest JSONB NOT NULL,
    state_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT paper_versions_workspace_number_unique UNIQUE (workspace_id, version_number),
    CONSTRAINT paper_versions_document_epoch_positive CHECK (document_epoch >= 1),
    CONSTRAINT paper_versions_number_positive CHECK (version_number >= 1),
    CONSTRAINT paper_versions_workspace_version_nonnegative CHECK (workspace_version >= 0),
    CONSTRAINT paper_versions_type_values CHECK (version_type IN (
        'manual_checkpoint', 'compile_checkpoint', 'review_round',
        'pre_restore_safety', 'admin_restoration', 'template_update', 'submission'
    )),
    CONSTRAINT paper_versions_name_length CHECK (name IS NULL OR char_length(name) BETWEEN 1 AND 200),
    CONSTRAINT paper_versions_manifest_object CHECK (jsonb_typeof(manifest) = 'object'),
    CONSTRAINT paper_versions_state_hash_digest CHECK (state_hash ~ '^[0-9a-f]{64}$')
);
CREATE INDEX paper_versions_workspace_created_idx
    ON latex_core.paper_versions (workspace_id, version_number DESC);
CREATE INDEX paper_versions_snapshot_idx
    ON latex_core.paper_versions (snapshot_id);

CREATE TABLE latex_core.v2_paper_builds (
    id UUID PRIMARY KEY,
    paper_id UUID NOT NULL,
    workspace_id UUID NOT NULL REFERENCES latex_core.workspaces(id) ON DELETE RESTRICT,
    compile_job_id UUID NOT NULL UNIQUE REFERENCES latex_core.compile_jobs(id) ON DELETE RESTRICT,
    version_id UUID NOT NULL UNIQUE REFERENCES latex_core.paper_versions(id) ON DELETE RESTRICT,
    document_epoch BIGINT NOT NULL,
    source_sequence BIGINT NOT NULL,
    state_hash TEXT NOT NULL,
    trigger_type TEXT NOT NULL,
    status TEXT NOT NULL,
    promoted_at TIMESTAMPTZ NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT v2_paper_builds_epoch_positive CHECK (document_epoch >= 1),
    CONSTRAINT v2_paper_builds_sequence_nonnegative CHECK (source_sequence >= 0),
    CONSTRAINT v2_paper_builds_state_hash_digest CHECK (state_hash ~ '^[0-9a-f]{64}$'),
    CONSTRAINT v2_paper_builds_trigger_values CHECK (trigger_type IN ('auto', 'manual')),
    CONSTRAINT v2_paper_builds_status_values CHECK (status IN ('queued', 'running', 'succeeded', 'failed'))
);
CREATE INDEX v2_paper_builds_workspace_created_idx
    ON latex_core.v2_paper_builds (workspace_id, created_at DESC);
CREATE INDEX v2_paper_builds_workspace_hash_idx
    ON latex_core.v2_paper_builds (workspace_id, state_hash, status);

-- One row is the PostgreSQL-serialized scheduler for a paper. Pending source is
-- an immutable snapshot description, not a queued compile job; replacement is
-- therefore a true newest-only coalesce.
CREATE TABLE latex_core.v2_paper_build_state (
    workspace_id UUID PRIMARY KEY REFERENCES latex_core.workspaces(id) ON DELETE RESTRICT,
    paper_id UUID NOT NULL,
    desired_state_hash TEXT NULL,
    desired_source_sequence BIGINT NULL,
    active_build_id UUID NULL REFERENCES latex_core.v2_paper_builds(id) ON DELETE RESTRICT,
    current_build_id UUID NULL REFERENCES latex_core.v2_paper_builds(id) ON DELETE RESTRICT,
    pending_snapshot_id TEXT NULL REFERENCES latex_core.snapshots(snapshot_id) ON DELETE RESTRICT,
    pending_manifest JSONB NULL,
    pending_state_hash TEXT NULL,
    pending_source_sequence BIGINT NULL,
    pending_document_epoch BIGINT NULL,
    pending_tenant_id UUID NULL REFERENCES latex_core.tenants(id) ON DELETE RESTRICT,
    pending_user_id UUID NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    pending_trigger_type TEXT NULL,
    pending_compile_key TEXT NULL,
    pending_engine TEXT NULL,
    pending_tex_environment_id TEXT NULL,
    pending_latexmk_profile TEXT NULL,
    pending_shell_policy TEXT NULL,
    pending_synctex BOOLEAN NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT v2_build_state_desired_hash CHECK (desired_state_hash IS NULL OR desired_state_hash ~ '^[0-9a-f]{64}$'),
    CONSTRAINT v2_build_state_pending_hash CHECK (pending_state_hash IS NULL OR pending_state_hash ~ '^[0-9a-f]{64}$'),
    CONSTRAINT v2_build_state_pending_compile_key CHECK (pending_compile_key IS NULL OR pending_compile_key ~ '^[0-9a-f]{64}$'),
    CONSTRAINT v2_build_state_pending_manifest CHECK (pending_manifest IS NULL OR jsonb_typeof(pending_manifest) = 'object'),
    CONSTRAINT v2_build_state_pending_trigger CHECK (pending_trigger_type IS NULL OR pending_trigger_type IN ('auto', 'manual')),
    CONSTRAINT v2_build_state_pending_all_or_none CHECK (
        (pending_snapshot_id IS NULL AND pending_manifest IS NULL AND pending_state_hash IS NULL
         AND pending_source_sequence IS NULL AND pending_document_epoch IS NULL
         AND pending_tenant_id IS NULL AND pending_user_id IS NULL AND pending_trigger_type IS NULL
         AND pending_compile_key IS NULL AND pending_engine IS NULL
         AND pending_tex_environment_id IS NULL AND pending_latexmk_profile IS NULL
         AND pending_shell_policy IS NULL AND pending_synctex IS NULL)
        OR
        (pending_snapshot_id IS NOT NULL AND pending_manifest IS NOT NULL AND pending_state_hash IS NOT NULL
         AND pending_source_sequence IS NOT NULL AND pending_document_epoch IS NOT NULL
         AND pending_tenant_id IS NOT NULL AND pending_user_id IS NOT NULL AND pending_trigger_type IS NOT NULL
         AND pending_compile_key IS NOT NULL AND pending_engine IS NOT NULL
         AND pending_tex_environment_id IS NOT NULL AND pending_latexmk_profile IS NOT NULL
         AND pending_shell_policy IS NOT NULL AND pending_synctex IS NOT NULL)
    )
);

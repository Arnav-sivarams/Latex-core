-- Sprint A: institutional classification, teams, canonical team heads, and policy/audit state.
ALTER TABLE latex_core.user_credentials
    ADD COLUMN account_type TEXT NOT NULL DEFAULT 'student',
    ADD CONSTRAINT user_credentials_account_type_values
        CHECK (account_type IN ('student', 'professor', 'admin'));

CREATE TABLE latex_core.teams (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL,
    created_by UUID NOT NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT teams_name_length CHECK (char_length(name) BETWEEN 1 AND 200)
);

CREATE TABLE latex_core.team_members (
    team_id UUID NOT NULL REFERENCES latex_core.teams(id) ON DELETE RESTRICT,
    user_id UUID NOT NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    can_write BOOLEAN NOT NULL DEFAULT FALSE,
    can_mentor BOOLEAN NOT NULL DEFAULT FALSE,
    can_manage BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (team_id, user_id)
);
CREATE INDEX team_members_user_team_idx ON latex_core.team_members (user_id, team_id);

CREATE TABLE latex_core.team_projects (
    id UUID PRIMARY KEY,
    team_id UUID NOT NULL REFERENCES latex_core.teams(id) ON DELETE RESTRICT,
    workspace_id UUID NOT NULL UNIQUE REFERENCES latex_core.workspaces(id) ON DELETE RESTRICT,
    name TEXT NOT NULL,
    canonical_generation BIGINT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT team_projects_name_length CHECK (char_length(name) BETWEEN 1 AND 200),
    CONSTRAINT team_projects_generation_positive CHECK (canonical_generation >= 1)
);
CREATE INDEX team_projects_team_updated_idx ON latex_core.team_projects (team_id, updated_at DESC);

CREATE TABLE latex_core.team_project_files (
    team_project_id UUID NOT NULL REFERENCES latex_core.team_projects(id) ON DELETE RESTRICT,
    logical_path TEXT NOT NULL,
    blob_hash TEXT NOT NULL,
    size_bytes BIGINT NOT NULL,
    file_revision BIGINT NOT NULL DEFAULT 1,
    PRIMARY KEY (team_project_id, logical_path),
    CONSTRAINT team_project_files_path_length CHECK (char_length(logical_path) BETWEEN 1 AND 1024),
    CONSTRAINT team_project_files_blob_digest CHECK (blob_hash ~ '^[0-9a-f]{64}$'),
    CONSTRAINT team_project_files_size_nonnegative CHECK (size_bytes >= 0),
    CONSTRAINT team_project_files_revision_positive CHECK (file_revision >= 1)
);

CREATE TABLE latex_core.member_drafts (
    team_project_id UUID NOT NULL REFERENCES latex_core.team_projects(id) ON DELETE RESTRICT,
    user_id UUID NOT NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    logical_path TEXT NOT NULL,
    base_file_revision BIGINT NOT NULL,
    draft_blob_hash TEXT NOT NULL,
    draft_size_bytes BIGINT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (team_project_id, user_id, logical_path),
    CONSTRAINT member_drafts_path_length CHECK (char_length(logical_path) BETWEEN 1 AND 1024),
    CONSTRAINT member_drafts_base_revision_nonnegative CHECK (base_file_revision >= 0),
    CONSTRAINT member_drafts_blob_digest CHECK (draft_blob_hash ~ '^[0-9a-f]{64}$'),
    CONSTRAINT member_drafts_size_nonnegative CHECK (draft_size_bytes >= 0)
);
CREATE INDEX member_drafts_user_updated_idx ON latex_core.member_drafts (user_id, updated_at DESC);

CREATE TABLE latex_core.file_policies (
    team_project_id UUID NOT NULL REFERENCES latex_core.team_projects(id) ON DELETE RESTRICT,
    logical_path TEXT NOT NULL,
    access_policy TEXT NOT NULL,
    set_by_user_id UUID NOT NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (team_project_id, logical_path),
    CONSTRAINT file_policies_path_length CHECK (char_length(logical_path) BETWEEN 1 AND 1024),
    CONSTRAINT file_policies_values CHECK (access_policy IN ('editable', 'read_only', 'hidden', 'admin_only'))
);

CREATE TABLE latex_core.team_project_audit (
    id UUID PRIMARY KEY,
    team_project_id UUID NOT NULL REFERENCES latex_core.team_projects(id) ON DELETE RESTRICT,
    actor_user_id UUID NOT NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    action_type TEXT NOT NULL,
    logical_path TEXT NULL,
    previous_blob_hash TEXT NULL,
    new_blob_hash TEXT NULL,
    canonical_generation BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT team_project_audit_action_length CHECK (char_length(action_type) BETWEEN 1 AND 64),
    CONSTRAINT team_project_audit_path_length CHECK (logical_path IS NULL OR char_length(logical_path) BETWEEN 1 AND 1024),
    CONSTRAINT team_project_audit_previous_digest CHECK (previous_blob_hash IS NULL OR previous_blob_hash ~ '^[0-9a-f]{64}$'),
    CONSTRAINT team_project_audit_new_digest CHECK (new_blob_hash IS NULL OR new_blob_hash ~ '^[0-9a-f]{64}$'),
    CONSTRAINT team_project_audit_generation_positive CHECK (canonical_generation >= 1)
);
CREATE INDEX team_project_audit_project_created_idx ON latex_core.team_project_audit (team_project_id, created_at DESC);

CREATE TABLE latex_core.template_account_types (
    template_id UUID NOT NULL REFERENCES latex_core.templates(id) ON DELETE CASCADE,
    account_type TEXT NOT NULL,
    PRIMARY KEY (template_id, account_type),
    CONSTRAINT template_account_types_values CHECK (account_type IN ('student', 'professor', 'admin'))
);
CREATE TABLE latex_core.template_user_grants (
    template_id UUID NOT NULL REFERENCES latex_core.templates(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES latex_core.users(id) ON DELETE CASCADE,
    granted_by_user_id UUID NOT NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (template_id, user_id)
);
CREATE INDEX template_user_grants_user_template_idx ON latex_core.template_user_grants (user_id, template_id);

-- Existing catalog entries remain visible to every institutional account type.
INSERT INTO latex_core.template_account_types (template_id, account_type)
SELECT id, account_type
FROM latex_core.templates
CROSS JOIN (VALUES ('student'), ('professor'), ('admin')) AS audience(account_type);

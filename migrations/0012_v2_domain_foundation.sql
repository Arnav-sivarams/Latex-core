-- C3: additive V2 domain foundation. Legacy identities and product data are
-- deliberately not backfilled into these tables.
CREATE TABLE latex_core.global_user_roles (
    user_id UUID PRIMARY KEY REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    role TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT global_user_roles_role_values
        CHECK (role IN ('writer', 'mentor', 'admin'))
);

-- A missing row is allowed only during the V1-to-V2 migration. Every active
-- V2 user must have exactly one row before the V2 auth cutover.
CREATE TABLE latex_core.personal_papers (
    id UUID PRIMARY KEY,
    owner_user_id UUID NOT NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    workspace_id UUID NOT NULL UNIQUE REFERENCES latex_core.workspaces(id) ON DELETE RESTRICT,
    name TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT personal_papers_name_length CHECK (char_length(name) BETWEEN 1 AND 200),
    CONSTRAINT personal_papers_status_values
        CHECK (status IN ('active', 'frozen', 'submitted', 'archived'))
);
CREATE INDEX personal_papers_owner_updated_idx
    ON latex_core.personal_papers (owner_user_id, updated_at DESC);

CREATE TABLE latex_core.paper_teams (
    id UUID PRIMARY KEY,
    workspace_id UUID NOT NULL UNIQUE REFERENCES latex_core.workspaces(id) ON DELETE RESTRICT,
    name TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    created_by_user_id UUID NOT NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT paper_teams_name_length CHECK (char_length(name) BETWEEN 1 AND 200),
    CONSTRAINT paper_teams_status_values
        CHECK (status IN ('active', 'frozen', 'submitted', 'archived'))
);
CREATE INDEX paper_teams_creator_updated_idx
    ON latex_core.paper_teams (created_by_user_id, updated_at DESC);

-- Membership grants paper access only. Capability comes exclusively from the
-- user's global role, so this relation intentionally has no local role fields.
CREATE TABLE latex_core.paper_team_members (
    paper_team_id UUID NOT NULL REFERENCES latex_core.paper_teams(id) ON DELETE RESTRICT,
    user_id UUID NOT NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    assigned_by_user_id UUID NOT NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (paper_team_id, user_id)
);
CREATE INDEX paper_team_members_user_team_idx
    ON latex_core.paper_team_members (user_id, paper_team_id);

-- BlobStore remains authoritative for immutable content. This table records
-- stable identity and mutable path metadata only.
CREATE TABLE latex_core.paper_files (
    file_id UUID PRIMARY KEY,
    workspace_id UUID NOT NULL REFERENCES latex_core.workspaces(id) ON DELETE RESTRICT,
    path TEXT NOT NULL,
    revision BIGINT NOT NULL DEFAULT 1,
    tombstoned BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    tombstoned_at TIMESTAMPTZ NULL,
    CONSTRAINT paper_files_path_length CHECK (char_length(path) BETWEEN 1 AND 1024),
    CONSTRAINT paper_files_revision_positive CHECK (revision >= 1),
    CONSTRAINT paper_files_tombstone_shape CHECK (
        (tombstoned AND tombstoned_at IS NOT NULL) OR
        (NOT tombstoned AND tombstoned_at IS NULL)
    )
);
CREATE UNIQUE INDEX paper_files_live_workspace_path_unique_idx
    ON latex_core.paper_files (workspace_id, path)
    WHERE NOT tombstoned;
CREATE INDEX paper_files_workspace_updated_idx
    ON latex_core.paper_files (workspace_id, updated_at DESC);

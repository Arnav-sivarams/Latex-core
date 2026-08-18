-- Product-foundation collaboration model.  Group membership is deliberately
-- distinct from project roles; existing Sprint-A grants are copied into the
-- project scope before the old broad booleans are removed.

ALTER TABLE latex_core.teams
    ADD COLUMN group_type TEXT NOT NULL DEFAULT 'research_team',
    ADD CONSTRAINT teams_group_type_values CHECK (group_type IN ('research_team', 'mentor_group'));

ALTER TABLE latex_core.team_members
    ADD COLUMN group_manager BOOLEAN NOT NULL DEFAULT FALSE;

UPDATE latex_core.team_members
SET group_manager = can_manage;

CREATE TABLE latex_core.team_project_members (
    team_project_id UUID NOT NULL REFERENCES latex_core.team_projects(id) ON DELETE RESTRICT,
    user_id UUID NOT NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    writer BOOLEAN NOT NULL DEFAULT FALSE,
    mentor BOOLEAN NOT NULL DEFAULT FALSE,
    project_manager BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (team_project_id, user_id)
);
CREATE INDEX team_project_members_user_project_idx
    ON latex_core.team_project_members (user_id, team_project_id);

INSERT INTO latex_core.team_project_members (team_project_id, user_id, writer, mentor, project_manager)
SELECT p.id, m.user_id, m.can_write, m.can_mentor, m.can_manage
FROM latex_core.team_projects p
JOIN latex_core.team_members m ON m.team_id = p.team_id;

ALTER TABLE latex_core.team_members
    DROP COLUMN can_write,
    DROP COLUMN can_mentor,
    DROP COLUMN can_manage;

ALTER TABLE latex_core.member_drafts
    ADD COLUMN draft_revision BIGINT NOT NULL DEFAULT 1,
    ADD CONSTRAINT member_drafts_revision_positive CHECK (draft_revision >= 1);

CREATE TABLE latex_core.permission_overrides (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    context_kind TEXT NOT NULL,
    context_id UUID NULL,
    permission TEXT NOT NULL,
    effect TEXT NOT NULL,
    granted_by_user_id UUID NOT NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT permission_overrides_context_values CHECK (context_kind IN ('global', 'team', 'project')),
    CONSTRAINT permission_overrides_context_shape CHECK (
        (context_kind = 'global' AND context_id IS NULL) OR
        (context_kind IN ('team', 'project') AND context_id IS NOT NULL)
    ),
    CONSTRAINT permission_overrides_effect_values CHECK (effect IN ('allow', 'deny')),
    CONSTRAINT permission_overrides_permission_values CHECK (permission IN (
        'project.read', 'project.create', 'project.manage', 'project.delete',
        'file.read', 'file.write', 'file.create', 'file.upload', 'file.rename', 'file.delete',
        'file.set_main', 'file.protected.write', 'compile.submit', 'artifact.read',
        'team.create', 'team.members.manage', 'team.project.manage', 'mentor_group.create',
        'review.read', 'review.comment', 'review.resolve', 'review.manage',
        'template.use', 'template.manage', 'admin.users', 'admin.teams', 'admin.projects',
        'admin.templates', 'admin.jobs', 'admin.storage', 'admin.backups', 'admin.audit',
        'admin.security', 'admin.system'
    ))
);
CREATE INDEX permission_overrides_lookup_idx
    ON latex_core.permission_overrides (user_id, context_kind, context_id);
CREATE UNIQUE INDEX permission_overrides_global_unique_idx
    ON latex_core.permission_overrides (user_id, permission)
    WHERE context_kind = 'global';
CREATE UNIQUE INDEX permission_overrides_scoped_unique_idx
    ON latex_core.permission_overrides (user_id, context_kind, context_id, permission)
    WHERE context_kind IN ('team', 'project');

CREATE TABLE latex_core.audit_events (
    id UUID PRIMARY KEY,
    actor_user_id UUID NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    event_type TEXT NOT NULL,
    resource_type TEXT NOT NULL,
    resource_id UUID NULL,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT audit_events_type_length CHECK (char_length(event_type) BETWEEN 1 AND 100),
    CONSTRAINT audit_events_resource_type_length CHECK (char_length(resource_type) BETWEEN 1 AND 100)
);
CREATE INDEX audit_events_created_idx ON latex_core.audit_events (created_at DESC);
CREATE INDEX audit_events_actor_created_idx ON latex_core.audit_events (actor_user_id, created_at DESC);

-- New, public file policy terminology. Existing values are translated rather
-- than represented as secrecy guarantees: TeX-materialized files are never
-- confidential from a user who can control the TeX source.
ALTER TABLE latex_core.file_policies
    DROP CONSTRAINT file_policies_values;
UPDATE latex_core.file_policies
SET access_policy = CASE access_policy
    WHEN 'hidden' THEN 'managed'
    WHEN 'admin_only' THEN 'managed'
    ELSE access_policy
END;
ALTER TABLE latex_core.file_policies
    ADD COLUMN origin TEXT NOT NULL DEFAULT 'project_manager',
    ADD CONSTRAINT file_policies_values CHECK (access_policy IN ('editable', 'read_only', 'managed')),
    ADD CONSTRAINT file_policies_origin_values CHECK (origin IN ('template', 'admin', 'project_manager'));

CREATE TABLE latex_core.template_policy_rules (
    template_id UUID NOT NULL REFERENCES latex_core.templates(id) ON DELETE CASCADE,
    path_pattern TEXT NOT NULL,
    access_policy TEXT NOT NULL,
    PRIMARY KEY (template_id, path_pattern),
    CONSTRAINT template_policy_rules_pattern_length CHECK (char_length(path_pattern) BETWEEN 1 AND 1024),
    CONSTRAINT template_policy_rules_policy CHECK (access_policy IN ('editable', 'read_only', 'managed'))
);
ALTER TABLE latex_core.templates
    ADD COLUMN main_file_locked BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN strict_structure BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN starter_file TEXT NULL;

ALTER TABLE latex_core.team_projects
    ADD COLUMN main_file_locked BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN strict_structure BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN template_id UUID NULL REFERENCES latex_core.templates(id) ON DELETE SET NULL,
    ADD COLUMN policy_default TEXT NOT NULL DEFAULT 'editable',
    ADD CONSTRAINT team_projects_policy_default_values CHECK (policy_default IN ('editable', 'read_only', 'managed'));

CREATE TABLE latex_core.member_change_operations (
    id UUID PRIMARY KEY,
    team_project_id UUID NOT NULL REFERENCES latex_core.team_projects(id) ON DELETE RESTRICT,
    user_id UUID NOT NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    operation_sequence BIGINT NOT NULL,
    operation_type TEXT NOT NULL,
    source_path TEXT NULL,
    destination_path TEXT NULL,
    base_file_revision BIGINT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT member_change_operations_sequence_positive CHECK (operation_sequence >= 1),
    CONSTRAINT member_change_operations_type_values CHECK (operation_type IN ('delete', 'rename', 'set_main')),
    CONSTRAINT member_change_operations_path_length CHECK (
        (source_path IS NULL OR char_length(source_path) BETWEEN 1 AND 1024) AND
        (destination_path IS NULL OR char_length(destination_path) BETWEEN 1 AND 1024)
    ),
    CONSTRAINT member_change_operations_shape CHECK (
        -- Draft-only additions have no canonical revision. The projection
        -- retains a NULL base for their private rename/delete normalization;
        -- canonical-derived operations always populate this field.
        (operation_type = 'delete' AND source_path IS NOT NULL AND destination_path IS NULL) OR
        (operation_type = 'rename' AND source_path IS NOT NULL AND destination_path IS NOT NULL) OR
        (operation_type = 'set_main' AND source_path IS NULL AND destination_path IS NOT NULL)
    ),
    UNIQUE (team_project_id, user_id, operation_sequence)
);
CREATE INDEX member_change_operations_user_project_idx
    ON latex_core.member_change_operations (team_project_id, user_id, operation_sequence);

CREATE TABLE latex_core.workspace_resume (
    user_id UUID PRIMARY KEY REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    workspace_id UUID NULL REFERENCES latex_core.workspaces(id) ON DELETE SET NULL,
    logical_path TEXT NULL,
    cursor_offset BIGINT NULL,
    selection_start BIGINT NULL,
    selection_end BIGINT NULL,
    scroll_top BIGINT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT workspace_resume_nonnegative CHECK (
        (cursor_offset IS NULL OR cursor_offset >= 0) AND
        (selection_start IS NULL OR selection_start >= 0) AND
        (selection_end IS NULL OR selection_end >= 0) AND
        (scroll_top IS NULL OR scroll_top >= 0)
    )
);

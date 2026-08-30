-- S7/RC: additive V2 governance. Historical versions, collaboration logs, and
-- template records remain immutable and are never rewritten by these tables.
CREATE TABLE latex_core.restoration_requests (
    id UUID PRIMARY KEY,
    paper_id UUID NOT NULL REFERENCES latex_core.paper_teams(id) ON DELETE RESTRICT,
    workspace_id UUID NOT NULL REFERENCES latex_core.workspaces(id) ON DELETE RESTRICT,
    requested_by_writer_user_id UUID NOT NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    target_version_id UUID NOT NULL REFERENCES latex_core.paper_versions(id) ON DELETE RESTRICT,
    state TEXT NOT NULL DEFAULT 'DRAFT',
    reason TEXT NULL,
    mentor_user_id UUID NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    mentor_decision_note TEXT NULL,
    admin_user_id UUID NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    admin_decision_note TEXT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    submitted_at TIMESTAMPTZ NULL,
    mentor_decided_at TIMESTAMPTZ NULL,
    admin_decided_at TIMESTAMPTZ NULL,
    applied_version_id UUID NULL REFERENCES latex_core.paper_versions(id) ON DELETE RESTRICT,
    CONSTRAINT restoration_requests_state_values CHECK (state IN (
        'DRAFT', 'AWAITING_MENTOR_REVIEW', 'MENTOR_REJECTED',
        'AWAITING_ADMIN_REVIEW', 'ADMIN_REJECTED', 'APPLIED'
    )),
    CONSTRAINT restoration_requests_reason_length CHECK (reason IS NULL OR char_length(reason) <= 4000),
    CONSTRAINT restoration_requests_mentor_note_length CHECK (mentor_decision_note IS NULL OR char_length(mentor_decision_note) <= 4000),
    CONSTRAINT restoration_requests_admin_note_length CHECK (admin_decision_note IS NULL OR char_length(admin_decision_note) <= 4000),
    CONSTRAINT restoration_requests_applied_shape CHECK (
        (state = 'APPLIED' AND applied_version_id IS NOT NULL) OR
        (state <> 'APPLIED' AND applied_version_id IS NULL)
    )
);
CREATE INDEX restoration_requests_writer_created_idx
    ON latex_core.restoration_requests (requested_by_writer_user_id, created_at DESC);
CREATE INDEX restoration_requests_mentor_state_idx
    ON latex_core.restoration_requests (state, paper_id, created_at DESC);

CREATE TABLE latex_core.paper_file_policies (
    file_id UUID PRIMARY KEY REFERENCES latex_core.paper_files(file_id) ON DELETE RESTRICT,
    workspace_id UUID NOT NULL REFERENCES latex_core.workspaces(id) ON DELETE RESTRICT,
    policy TEXT NOT NULL DEFAULT 'EDITABLE',
    updated_by_admin_user_id UUID NOT NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (workspace_id, file_id)
        REFERENCES latex_core.paper_files(workspace_id, file_id) ON DELETE RESTRICT,
    CONSTRAINT paper_file_policies_values CHECK (policy IN (
        'EDITABLE', 'CONTENT_READ_ONLY', 'STRUCTURE_LOCKED',
        'TEMPLATE_MANAGED', 'HIDDEN_SYSTEM'
    ))
);
CREATE INDEX paper_file_policies_workspace_idx
    ON latex_core.paper_file_policies (workspace_id, file_id);

-- The existing immutable template library remains authoritative. This relation
-- records only the exact initial template identity pinned to a V2 Paper Team.
CREATE TABLE latex_core.paper_template_pins (
    paper_id UUID PRIMARY KEY REFERENCES latex_core.paper_teams(id) ON DELETE RESTRICT,
    workspace_id UUID NOT NULL UNIQUE REFERENCES latex_core.workspaces(id) ON DELETE RESTRICT,
    template_id UUID NOT NULL REFERENCES latex_core.templates(id) ON DELETE RESTRICT,
    source_identity TEXT NOT NULL,
    pinned_by_admin_user_id UUID NOT NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    pinned_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT paper_template_pins_source_digest CHECK (source_identity ~ '^[0-9a-f]{64}$')
);

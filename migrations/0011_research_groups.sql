-- Simple equal-member collaboration. Research groups deliberately do not
-- reuse the role-scoped Team or private-draft model.
CREATE TABLE latex_core.research_groups (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL,
    owner_user_id UUID NOT NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    workspace_id UUID NOT NULL UNIQUE REFERENCES latex_core.workspaces(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT research_groups_name_length CHECK (char_length(name) BETWEEN 1 AND 200)
);
CREATE INDEX research_groups_owner_updated_idx
    ON latex_core.research_groups (owner_user_id, updated_at DESC);

CREATE TABLE latex_core.research_group_members (
    group_id UUID NOT NULL REFERENCES latex_core.research_groups(id) ON DELETE RESTRICT,
    user_id UUID NOT NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    joined_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (group_id, user_id)
);
CREATE INDEX research_group_members_user_group_idx
    ON latex_core.research_group_members (user_id, group_id);

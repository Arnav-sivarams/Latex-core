-- S6: bounded, Writer-owned reversal metadata for recent file-tree actions.
-- Source content remains in BlobStore/workspace events and is never copied here.
CREATE TABLE latex_core.reversible_structural_operations (
    id UUID PRIMARY KEY,
    paper_id UUID NOT NULL,
    workspace_id UUID NOT NULL REFERENCES latex_core.workspaces(id) ON DELETE RESTRICT,
    actor_user_id UUID NOT NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    operation_type TEXT NOT NULL,
    file_id UUID NULL REFERENCES latex_core.paper_files(file_id) ON DELETE RESTRICT,
    forward_payload JSONB NOT NULL,
    inverse_payload JSONB NOT NULL,
    workspace_version_before BIGINT NOT NULL,
    workspace_version_after BIGINT NOT NULL,
    state TEXT NOT NULL DEFAULT 'APPLIED',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    undone_at TIMESTAMPTZ NULL,
    redone_at TIMESTAMPTZ NULL,
    CONSTRAINT reversible_structural_operation_type_values CHECK (
        operation_type IN ('CREATE_FILE', 'RENAME_FILE', 'MOVE_FILE', 'DELETE_FILE', 'SET_MAIN')
    ),
    CONSTRAINT reversible_structural_operation_state_values CHECK (
        state IN ('APPLIED', 'UNDONE', 'INVALIDATED')
    ),
    CONSTRAINT reversible_structural_operation_payload_objects CHECK (
        jsonb_typeof(forward_payload) = 'object' AND jsonb_typeof(inverse_payload) = 'object'
    ),
    CONSTRAINT reversible_structural_operation_versions CHECK (
        workspace_version_before >= 0 AND workspace_version_after = workspace_version_before + 1
    ),
    CONSTRAINT reversible_structural_operation_state_shape CHECK (
        (state = 'APPLIED' AND undone_at IS NULL) OR
        (state = 'UNDONE' AND undone_at IS NOT NULL) OR
        (state = 'INVALIDATED')
    )
);
CREATE INDEX reversible_structural_operations_actor_recent_idx
    ON latex_core.reversible_structural_operations
       (paper_id, actor_user_id, created_at DESC, id DESC);
CREATE INDEX reversible_structural_operations_workspace_state_idx
    ON latex_core.reversible_structural_operations
       (workspace_id, state, created_at DESC, id DESC);

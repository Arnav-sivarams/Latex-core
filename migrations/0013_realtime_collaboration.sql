-- S3: durable Yjs/Yrs state for one collaborative document per live paper file.
ALTER TABLE latex_core.paper_files
    ADD CONSTRAINT paper_files_workspace_file_unique UNIQUE (workspace_id, file_id);

CREATE TABLE latex_core.paper_collaboration_state (
    workspace_id UUID PRIMARY KEY REFERENCES latex_core.workspaces(id) ON DELETE RESTRICT,
    document_epoch BIGINT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT paper_collaboration_state_epoch_positive CHECK (document_epoch >= 1)
);

CREATE TABLE latex_core.collaboration_updates (
    id BIGSERIAL PRIMARY KEY,
    workspace_id UUID NOT NULL,
    file_id UUID NOT NULL,
    document_epoch BIGINT NOT NULL,
    actor_user_id UUID NOT NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    update_bytes BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (workspace_id, file_id)
        REFERENCES latex_core.paper_files(workspace_id, file_id) ON DELETE RESTRICT,
    CONSTRAINT collaboration_updates_epoch_positive CHECK (document_epoch >= 1),
    CONSTRAINT collaboration_updates_nonempty CHECK (octet_length(update_bytes) > 0)
);
CREATE INDEX collaboration_updates_recovery_idx
    ON latex_core.collaboration_updates (workspace_id, file_id, document_epoch, id);

CREATE TABLE latex_core.collaboration_snapshots (
    workspace_id UUID NOT NULL,
    file_id UUID NOT NULL,
    document_epoch BIGINT NOT NULL,
    through_sequence BIGINT NOT NULL,
    compressed_state BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, file_id, document_epoch, through_sequence),
    FOREIGN KEY (workspace_id, file_id)
        REFERENCES latex_core.paper_files(workspace_id, file_id) ON DELETE RESTRICT,
    CONSTRAINT collaboration_snapshots_epoch_positive CHECK (document_epoch >= 1),
    CONSTRAINT collaboration_snapshots_sequence_nonnegative CHECK (through_sequence >= 0),
    CONSTRAINT collaboration_snapshots_nonempty CHECK (octet_length(compressed_state) > 0)
);
CREATE INDEX collaboration_snapshots_latest_idx
    ON latex_core.collaboration_snapshots
       (workspace_id, file_id, document_epoch, through_sequence DESC);

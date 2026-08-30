-- S5: additive Mentor review workflow. Review records reference immutable S4
-- versions/builds and stable S3 file identities; they never contain source edits.
CREATE TABLE latex_core.review_rounds (
    id UUID PRIMARY KEY,
    paper_id UUID NOT NULL REFERENCES latex_core.paper_teams(id) ON DELETE RESTRICT,
    workspace_id UUID NOT NULL REFERENCES latex_core.workspaces(id) ON DELETE RESTRICT,
    round_number BIGINT NOT NULL,
    baseline_version_id UUID NOT NULL REFERENCES latex_core.paper_versions(id) ON DELETE RESTRICT,
    opened_by_mentor_user_id UUID NOT NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    status TEXT NOT NULL DEFAULT 'OPEN',
    opened_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    closed_at TIMESTAMPTZ NULL,
    CONSTRAINT review_rounds_workspace_number_unique UNIQUE (workspace_id, round_number),
    CONSTRAINT review_rounds_number_positive CHECK (round_number >= 1),
    CONSTRAINT review_rounds_status_values CHECK (status IN ('OPEN', 'APPROVED', 'CLOSED')),
    CONSTRAINT review_rounds_closed_shape CHECK (
        (status = 'OPEN' AND closed_at IS NULL) OR
        (status IN ('APPROVED', 'CLOSED') AND closed_at IS NOT NULL)
    )
);
CREATE INDEX review_rounds_paper_opened_idx
    ON latex_core.review_rounds (paper_id, opened_at DESC);

CREATE TABLE latex_core.review_threads (
    id UUID PRIMARY KEY,
    review_round_id UUID NOT NULL REFERENCES latex_core.review_rounds(id) ON DELETE RESTRICT,
    workspace_id UUID NOT NULL REFERENCES latex_core.workspaces(id) ON DELETE RESTRICT,
    thread_type TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'OPEN',
    severity TEXT NOT NULL,
    category TEXT NOT NULL,
    assigned_writer_user_id UUID NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    due_at TIMESTAMPTZ NULL,
    created_by_mentor_user_id UUID NOT NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    section_label TEXT NULL,
    approved_version_id UUID NULL REFERENCES latex_core.paper_versions(id) ON DELETE RESTRICT,
    approved_build_id UUID NULL REFERENCES latex_core.v2_paper_builds(id) ON DELETE RESTRICT,
    approved_state_hash TEXT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at TIMESTAMPTZ NULL,
    CONSTRAINT review_threads_type_values CHECK (thread_type IN (
        'COMMENT', 'QUESTION', 'CHANGE_REQUEST', 'SUGGESTED_REPLACEMENT',
        'SECTION_APPROVAL', 'PAPER_APPROVAL'
    )),
    CONSTRAINT review_threads_state_values CHECK (state IN (
        'OPEN', 'ADDRESSED', 'RESOLVED', 'REOPENED', 'REJECTED'
    )),
    CONSTRAINT review_threads_severity_values CHECK (severity IN ('NOTE', 'MINOR', 'MAJOR', 'BLOCKING')),
    CONSTRAINT review_threads_category_values CHECK (category IN (
        'WRITING', 'METHODOLOGY', 'EVIDENCE', 'CITATION', 'FORMATTING',
        'FIGURE', 'TABLE', 'EQUATION', 'SUBMISSION_REQUIREMENT'
    )),
    CONSTRAINT review_threads_section_label_length CHECK (
        section_label IS NULL OR char_length(section_label) BETWEEN 1 AND 300
    ),
    CONSTRAINT review_threads_approval_hash CHECK (
        approved_state_hash IS NULL OR approved_state_hash ~ '^[0-9a-f]{64}$'
    ),
    CONSTRAINT review_threads_paper_approval_exact CHECK (
        thread_type <> 'PAPER_APPROVAL' OR
        (approved_version_id IS NOT NULL AND approved_build_id IS NOT NULL AND approved_state_hash IS NOT NULL)
    )
);
CREATE INDEX review_threads_workspace_state_idx
    ON latex_core.review_threads (workspace_id, state, updated_at DESC);
CREATE INDEX review_threads_round_idx
    ON latex_core.review_threads (review_round_id, created_at);

CREATE TABLE latex_core.review_messages (
    id UUID PRIMARY KEY,
    thread_id UUID NOT NULL REFERENCES latex_core.review_threads(id) ON DELETE RESTRICT,
    author_user_id UUID NOT NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    body TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT review_messages_body_length CHECK (char_length(body) BETWEEN 1 AND 20000)
);
CREATE INDEX review_messages_thread_created_idx
    ON latex_core.review_messages (thread_id, created_at, id);

CREATE TABLE latex_core.review_source_anchors (
    thread_id UUID PRIMARY KEY REFERENCES latex_core.review_threads(id) ON DELETE RESTRICT,
    file_id UUID NOT NULL REFERENCES latex_core.paper_files(file_id) ON DELETE RESTRICT,
    encoded_relative_start BYTEA NULL,
    encoded_relative_end BYTEA NULL,
    quoted_text TEXT NOT NULL,
    context_hash TEXT NOT NULL,
    source_sequence BIGINT NOT NULL,
    source_version_id UUID NULL REFERENCES latex_core.paper_versions(id) ON DELETE RESTRICT,
    document_epoch BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT review_source_anchors_relative_pair CHECK (
        (encoded_relative_start IS NULL) = (encoded_relative_end IS NULL)
    ),
    CONSTRAINT review_source_anchors_relative_nonempty CHECK (
        encoded_relative_start IS NULL OR
        (octet_length(encoded_relative_start) > 0 AND octet_length(encoded_relative_end) > 0)
    ),
    CONSTRAINT review_source_anchors_context_hash CHECK (context_hash ~ '^[0-9a-f]{64}$'),
    CONSTRAINT review_source_anchors_sequence_nonnegative CHECK (source_sequence >= 0),
    CONSTRAINT review_source_anchors_epoch_positive CHECK (document_epoch >= 1)
);

CREATE TABLE latex_core.review_pdf_anchors (
    id UUID PRIMARY KEY,
    thread_id UUID NOT NULL REFERENCES latex_core.review_threads(id) ON DELETE RESTRICT,
    artifact_id UUID NOT NULL REFERENCES latex_core.compilation_artifacts(artifact_id) ON DELETE RESTRICT,
    build_id UUID NOT NULL REFERENCES latex_core.v2_paper_builds(id) ON DELETE RESTRICT,
    page INTEGER NOT NULL,
    normalized_rectangles JSONB NOT NULL,
    mapping_status TEXT NOT NULL,
    mapped_file_id UUID NULL REFERENCES latex_core.paper_files(file_id) ON DELETE RESTRICT,
    mapped_line INTEGER NULL,
    mapped_column INTEGER NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT review_pdf_anchors_page_positive CHECK (page >= 1),
    CONSTRAINT review_pdf_anchors_rectangles_array CHECK (jsonb_typeof(normalized_rectangles) = 'array'),
    CONSTRAINT review_pdf_anchors_mapping_values CHECK (mapping_status IN (
        'EXACT', 'APPROXIMATE', 'PDF_ONLY', 'SOURCE_CHANGED', 'SOURCE_DELETED'
    )),
    CONSTRAINT review_pdf_anchors_mapped_shape CHECK (
        (mapped_file_id IS NULL AND mapped_line IS NULL AND mapped_column IS NULL) OR
        (mapped_file_id IS NOT NULL AND mapped_line IS NOT NULL AND mapped_line >= 1
         AND mapped_column IS NOT NULL AND mapped_column >= 0)
    )
);
CREATE INDEX review_pdf_anchors_thread_created_idx
    ON latex_core.review_pdf_anchors (thread_id, created_at DESC);

CREATE TABLE latex_core.review_suggestions (
    thread_id UUID PRIMARY KEY REFERENCES latex_core.review_threads(id) ON DELETE RESTRICT,
    replacement_text TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'PENDING',
    accepted_by_writer_user_id UUID NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    responded_at TIMESTAMPTZ NULL,
    rejection_reason TEXT NULL,
    CONSTRAINT review_suggestions_status_values CHECK (status IN ('PENDING', 'ACCEPTED', 'REJECTED')),
    CONSTRAINT review_suggestions_response_shape CHECK (
        (status = 'PENDING' AND accepted_by_writer_user_id IS NULL AND responded_at IS NULL AND rejection_reason IS NULL) OR
        (status = 'ACCEPTED' AND accepted_by_writer_user_id IS NOT NULL AND responded_at IS NOT NULL AND rejection_reason IS NULL) OR
        (status = 'REJECTED' AND accepted_by_writer_user_id IS NULL AND responded_at IS NOT NULL)
    )
);

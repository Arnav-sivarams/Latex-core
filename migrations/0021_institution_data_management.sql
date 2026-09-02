-- V2.2 institution data-management orchestration.
-- Existing standalone jobs remain valid; batches only group new file jobs.
CREATE TABLE latex_core.institution_import_batches (
    id UUID PRIMARY KEY,
    operation TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'VALIDATING',
    submitted_by_user_id UUID NOT NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    total_files INT NOT NULL DEFAULT 0,
    total_rows BIGINT NOT NULL DEFAULT 0,
    added_rows BIGINT NOT NULL DEFAULT 0,
    edited_rows BIGINT NOT NULL DEFAULT 0,
    deleted_rows BIGINT NOT NULL DEFAULT 0,
    skipped_rows BIGINT NOT NULL DEFAULT 0,
    error_rows BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    validated_at TIMESTAMPTZ,
    applied_at TIMESTAMPTZ,
    CONSTRAINT institution_import_batches_operation_values
        CHECK (operation IN ('ADD', 'EDIT', 'DELETE')),
    CONSTRAINT institution_import_batches_status_values
        CHECK (status IN ('VALIDATING', 'VALIDATED', 'APPLYING', 'APPLIED', 'PARTIAL', 'FAILED')),
    CONSTRAINT institution_import_batches_counts_nonnegative CHECK (
        total_files >= 0 AND total_rows >= 0 AND added_rows >= 0 AND
        edited_rows >= 0 AND deleted_rows >= 0 AND skipped_rows >= 0 AND error_rows >= 0
    )
);

ALTER TABLE latex_core.institution_import_jobs
    ADD COLUMN batch_id UUID REFERENCES latex_core.institution_import_batches(id) ON DELETE RESTRICT;

ALTER TABLE latex_core.institution_import_jobs
    DROP CONSTRAINT institution_import_jobs_mode_values,
    ADD CONSTRAINT institution_import_jobs_mode_values CHECK (
        mode IN ('VALIDATE_ONLY', 'MERGE', 'ADD_ONLY', 'UPDATE_ONLY', 'DELETE_ONLY')
    );

ALTER TABLE latex_core.institution_import_rows
    DROP CONSTRAINT institution_import_rows_action_values,
    ADD CONSTRAINT institution_import_rows_action_values CHECK (
        action IN ('INSERT', 'UPDATE', 'DELETE', 'SKIP', 'INVALID', 'MATERIALIZE')
    ),
    ADD COLUMN existing_payload JSONB,
    ADD CONSTRAINT institution_import_rows_existing_payload_object CHECK (
        existing_payload IS NULL OR jsonb_typeof(existing_payload) = 'object'
    );

CREATE INDEX institution_import_batches_created_idx
    ON latex_core.institution_import_batches (created_at DESC, id);
CREATE INDEX institution_import_batches_status_created_idx
    ON latex_core.institution_import_batches (status, created_at DESC, id);
CREATE INDEX institution_import_jobs_batch_idx
    ON latex_core.institution_import_jobs (batch_id, created_at, id) WHERE batch_id IS NOT NULL;

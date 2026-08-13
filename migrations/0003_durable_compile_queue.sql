-- M8/M9: retain the original schema and extend it with durable queue semantics.
ALTER TABLE latex_core.compile_jobs
    DROP CONSTRAINT compile_jobs_state_values;

ALTER TABLE latex_core.compile_jobs
    ADD CONSTRAINT compile_jobs_state_values
    CHECK (state IN ('queued', 'claimed', 'running', 'succeeded', 'failed', 'timed_out', 'cancelled'));

ALTER TABLE latex_core.compile_jobs
    ADD COLUMN cancellation_requested_at TIMESTAMPTZ NULL,
    ADD COLUMN cancellation_reason TEXT NULL,
    ADD COLUMN error_class TEXT NULL,
    ADD COLUMN cache_source_job_id UUID NULL,
    ADD CONSTRAINT compile_jobs_cache_source_fk
        FOREIGN KEY (cache_source_job_id) REFERENCES latex_core.compile_jobs(id) ON DELETE RESTRICT,
    ADD CONSTRAINT compile_jobs_terminal_lease_clear
        CHECK (state NOT IN ('succeeded', 'failed', 'timed_out', 'cancelled')
            OR (worker_id IS NULL AND lease_until IS NULL)),
    ADD CONSTRAINT compile_jobs_cancel_reason_nonempty
        CHECK (cancellation_reason IS NULL OR char_length(cancellation_reason) BETWEEN 1 AND 512),
    ADD CONSTRAINT compile_jobs_error_class_nonempty
        CHECK (error_class IS NULL OR char_length(error_class) BETWEEN 1 AND 64);

ALTER TABLE latex_core.compilation_artifacts
    ADD COLUMN content_type TEXT NOT NULL DEFAULT 'application/octet-stream',
    ADD CONSTRAINT compilation_artifacts_content_type_nonempty
        CHECK (char_length(content_type) BETWEEN 1 AND 255);

CREATE INDEX compile_jobs_user_outstanding_idx
    ON latex_core.compile_jobs (user_id, created_at)
    WHERE state IN ('queued', 'claimed', 'running');
CREATE INDEX compile_jobs_cache_source_idx
    ON latex_core.compile_jobs (cache_source_job_id)
    WHERE cache_source_job_id IS NOT NULL;

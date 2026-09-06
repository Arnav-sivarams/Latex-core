-- Run 2: secret-free operator backup and restore-drill visibility.
-- Backup contents and operator keys remain outside PostgreSQL.
CREATE TABLE latex_core.operator_recovery_events (
    id UUID PRIMARY KEY,
    operation TEXT NOT NULL,
    status TEXT NOT NULL,
    phase TEXT NOT NULL,
    backup_name TEXT,
    release_commit TEXT,
    schema_version BIGINT,
    recovery_point TIMESTAMPTZ,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT operator_recovery_operation_values
        CHECK (operation IN ('BACKUP', 'RESTORE_DRILL')),
    CONSTRAINT operator_recovery_status_values
        CHECK (status IN ('SUCCESS', 'FAILED')),
    CONSTRAINT operator_recovery_phase_length
        CHECK (char_length(phase) BETWEEN 1 AND 64),
    CONSTRAINT operator_recovery_backup_name_length
        CHECK (backup_name IS NULL OR char_length(backup_name) BETWEEN 1 AND 200),
    CONSTRAINT operator_recovery_commit_shape
        CHECK (release_commit IS NULL OR release_commit ~ '^[0-9a-f]{40,64}$'),
    CONSTRAINT operator_recovery_schema_nonnegative
        CHECK (schema_version IS NULL OR schema_version >= 0)
);

CREATE INDEX operator_recovery_events_operation_time_idx
    ON latex_core.operator_recovery_events (operation, occurred_at DESC);

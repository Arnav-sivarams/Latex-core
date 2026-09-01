-- V2.1: Team-scoped Writer leadership, Team-local restoration decisions, and
-- Leader-controlled review submissions. Historical governance and review rows
-- remain intact; nullable metadata distinguishes them from the new workflow.

ALTER TABLE latex_core.paper_team_members
    ADD COLUMN is_leader BOOLEAN NOT NULL DEFAULT FALSE;

-- Existing active Teams predate explicit leadership. Preserve every membership
-- and deterministically promote the earliest assigned global Writer. Refuse the
-- upgrade when no Writer exists rather than leaving an active Team invalid.
WITH legacy_leaders AS (
    SELECT DISTINCT ON (m.paper_team_id)
        m.paper_team_id,
        m.user_id
    FROM latex_core.paper_team_members m
    JOIN latex_core.paper_teams t ON t.id = m.paper_team_id
    JOIN latex_core.global_user_roles r ON r.user_id = m.user_id
    WHERE t.status = 'active' AND r.role = 'writer'
    ORDER BY m.paper_team_id, m.created_at, m.user_id
)
UPDATE latex_core.paper_team_members m
SET is_leader = TRUE
FROM legacy_leaders leader
WHERE m.paper_team_id = leader.paper_team_id
  AND m.user_id = leader.user_id;

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM latex_core.paper_teams t
        WHERE t.status = 'active'
          AND NOT EXISTS (
              SELECT 1
              FROM latex_core.paper_team_members m
              JOIN latex_core.global_user_roles r ON r.user_id = m.user_id
              WHERE m.paper_team_id = t.id
                AND m.is_leader
                AND r.role = 'writer'
          )
    ) THEN
        RAISE EXCEPTION
            'cannot activate V2.1 Team leadership: an active Paper Team has no Writer member';
    END IF;
END
$$;

CREATE UNIQUE INDEX paper_team_members_one_leader_idx
    ON latex_core.paper_team_members (paper_team_id)
    WHERE is_leader;

COMMENT ON COLUMN latex_core.paper_team_members.is_leader IS
    'Team-scoped capability. A Leader remains globally WRITER.';

ALTER TABLE latex_core.restoration_requests
    ADD COLUMN leader_writer_user_id UUID NULL
        REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    ADD COLUMN leader_decision_note TEXT NULL,
    ADD COLUMN leader_decided_at TIMESTAMPTZ NULL;

ALTER TABLE latex_core.restoration_requests
    DROP CONSTRAINT restoration_requests_state_values,
    ADD CONSTRAINT restoration_requests_state_values CHECK (state IN (
        'DRAFT', 'AWAITING_MENTOR_REVIEW', 'MENTOR_REJECTED',
        'AWAITING_ADMIN_REVIEW', 'ADMIN_REJECTED',
        'REQUESTED', 'LEADER_REJECTED', 'APPLIED'
    )),
    ADD CONSTRAINT restoration_requests_leader_note_length
        CHECK (leader_decision_note IS NULL OR char_length(leader_decision_note) <= 4000);

ALTER TABLE latex_core.review_rounds
    ALTER COLUMN opened_by_mentor_user_id DROP NOT NULL,
    ADD COLUMN submitted_by_leader_writer_id UUID NULL
        REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    ADD COLUMN baseline_build_id UUID NULL
        REFERENCES latex_core.v2_paper_builds(id) ON DELETE RESTRICT,
    ADD COLUMN baseline_state_hash TEXT NULL;

ALTER TABLE latex_core.review_rounds
    DROP CONSTRAINT review_rounds_status_values,
    DROP CONSTRAINT review_rounds_closed_shape,
    ADD CONSTRAINT review_rounds_status_values CHECK (status IN (
        'OPEN', 'APPROVED', 'OPEN_FOR_REVIEW', 'CLOSED'
    )),
    ADD CONSTRAINT review_rounds_closed_shape CHECK (
        (status IN ('OPEN', 'OPEN_FOR_REVIEW') AND closed_at IS NULL) OR
        (status IN ('APPROVED', 'CLOSED') AND closed_at IS NOT NULL)
    ),
    ADD CONSTRAINT review_rounds_submission_origin CHECK (
        (opened_by_mentor_user_id IS NULL) <> (submitted_by_leader_writer_id IS NULL)
    ),
    ADD CONSTRAINT review_rounds_baseline_state_hash CHECK (
        baseline_state_hash IS NULL OR baseline_state_hash ~ '^[0-9a-f]{64}$'
    ),
    ADD CONSTRAINT review_rounds_leader_baseline_shape CHECK (
        (submitted_by_leader_writer_id IS NULL AND baseline_build_id IS NULL
            AND baseline_state_hash IS NULL) OR
        (submitted_by_leader_writer_id IS NOT NULL AND baseline_build_id IS NOT NULL
            AND baseline_state_hash IS NOT NULL)
    );

CREATE UNIQUE INDEX review_rounds_one_open_workspace_idx
    ON latex_core.review_rounds (workspace_id)
    WHERE status = 'OPEN_FOR_REVIEW';

ALTER TABLE latex_core.review_threads
    DROP CONSTRAINT review_threads_type_values,
    ADD CONSTRAINT review_threads_type_values CHECK (thread_type IN (
        'COMMENT', 'SUGGESTION', 'QUESTION', 'CHANGE_REQUEST',
        'SUGGESTED_REPLACEMENT', 'SECTION_APPROVAL', 'PAPER_APPROVAL'
    ));

ALTER TABLE latex_core.paper_versions
    DROP CONSTRAINT paper_versions_type_values,
    ADD CONSTRAINT paper_versions_type_values CHECK (version_type IN (
        'manual_checkpoint', 'compile_checkpoint', 'review_round',
        'pre_restore_safety', 'admin_restoration', 'team_revert',
        'template_update', 'submission'
    ));

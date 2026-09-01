-- V2.1 hotfix: a review is open only when it was submitted by the Team
-- Leader with an exact immutable build baseline. Keep pre-V2.1 rounds and all
-- of their threads intact, but close stale rows that cannot satisfy that gate.
UPDATE latex_core.review_rounds
SET status = 'CLOSED',
    closed_at = COALESCE(closed_at, statement_timestamp())
WHERE status IN ('OPEN', 'OPEN_FOR_REVIEW')
  AND NOT (
      status = 'OPEN_FOR_REVIEW'
      AND submitted_by_leader_writer_id IS NOT NULL
      AND baseline_build_id IS NOT NULL
      AND baseline_state_hash IS NOT NULL
  );

ALTER TABLE latex_core.review_rounds
    ADD CONSTRAINT review_rounds_v21_open_shape CHECK (
        status <> 'OPEN_FOR_REVIEW' OR (
            opened_by_mentor_user_id IS NULL
            AND submitted_by_leader_writer_id IS NOT NULL
            AND baseline_build_id IS NOT NULL
            AND baseline_state_hash IS NOT NULL
        )
    );

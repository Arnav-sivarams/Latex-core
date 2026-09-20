-- Canonical display metadata is intentionally additive: existing identifiers
-- and relationships remain the institutional source of truth.
ALTER TABLE vcap.schools ADD COLUMN school_name TEXT;
ALTER TABLE vcap.departments ADD COLUMN department_name TEXT;
ALTER TABLE vcap.programmes
    ADD COLUMN programme_name TEXT,
    ADD COLUMN degree_name TEXT,
    ADD COLUMN specialization TEXT;

CREATE TABLE latex_core.team_chat_messages (
    id UUID PRIMARY KEY,
    team_id UUID NOT NULL REFERENCES latex_core.paper_teams(id) ON DELETE RESTRICT,
    author_user_id UUID NOT NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    body TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT team_chat_messages_body_length CHECK (char_length(body) BETWEEN 1 AND 4000)
);
CREATE INDEX team_chat_messages_team_created_idx
    ON latex_core.team_chat_messages (team_id, created_at DESC, id DESC);

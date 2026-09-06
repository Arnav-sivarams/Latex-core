-- Professor feedback Run 1: explicit Mentor draft publication, per-Mentor
-- participation, and narrow application-owned presentation settings.
--
-- Historical review threads were already Writer-visible, so the additive
-- publication columns deliberately default them to PUBLISHED.
ALTER TABLE latex_core.review_threads
    ADD COLUMN publication_status TEXT NOT NULL DEFAULT 'PUBLISHED',
    ADD COLUMN draft_revision BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN published_at TIMESTAMPTZ,
    ADD COLUMN publication_id UUID,
    ADD CONSTRAINT review_threads_publication_status_values
        CHECK (publication_status IN ('DRAFT', 'PUBLISHED')),
    ADD CONSTRAINT review_threads_draft_revision_nonnegative
        CHECK (draft_revision >= 0),
    ADD CONSTRAINT review_threads_publication_shape CHECK (
        (publication_status = 'DRAFT' AND published_at IS NULL AND publication_id IS NULL) OR
        (publication_status = 'PUBLISHED')
    );

UPDATE latex_core.review_threads
SET published_at = created_at
WHERE publication_status = 'PUBLISHED' AND published_at IS NULL;

CREATE INDEX review_threads_round_mentor_publication_idx
    ON latex_core.review_threads
        (review_round_id, created_by_mentor_user_id, publication_status, created_at);

CREATE TABLE latex_core.review_participations (
    review_round_id UUID NOT NULL
        REFERENCES latex_core.review_rounds(id) ON DELETE RESTRICT,
    mentor_user_id UUID NOT NULL
        REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    status TEXT NOT NULL DEFAULT 'PENDING',
    draft_revision BIGINT NOT NULL DEFAULT 0,
    submitted_revision BIGINT,
    submission_id UUID,
    submitted_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (review_round_id, mentor_user_id),
    CONSTRAINT review_participations_status_values
        CHECK (status IN ('PENDING', 'SUBMITTED', 'WITHDRAWN')),
    CONSTRAINT review_participations_revision_nonnegative CHECK (
        draft_revision >= 0 AND
        (submitted_revision IS NULL OR submitted_revision >= 0)
    ),
    CONSTRAINT review_participations_submission_shape CHECK (
        (status = 'PENDING' AND submitted_revision IS NULL AND submission_id IS NULL AND submitted_at IS NULL) OR
        (status = 'SUBMITTED' AND submitted_revision IS NOT NULL AND submission_id IS NOT NULL AND submitted_at IS NOT NULL) OR
        (status = 'WITHDRAWN' AND submission_id IS NULL AND submitted_at IS NULL)
    ),
    UNIQUE (submission_id)
);

-- Closed historical rounds need no active participation. For the one possible
-- in-flight round during an online migration, seed every assigned Mentor.
INSERT INTO latex_core.review_participations (review_round_id, mentor_user_id)
SELECT rr.id, member.user_id
FROM latex_core.review_rounds rr
JOIN latex_core.paper_team_members member ON member.paper_team_id = rr.paper_id
JOIN latex_core.global_user_roles role
  ON role.user_id = member.user_id AND role.role = 'mentor'
WHERE rr.status = 'OPEN_FOR_REVIEW'
ON CONFLICT DO NOTHING;

CREATE TABLE latex_core.user_editor_preferences (
    user_id UUID PRIMARY KEY REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    font_size_px SMALLINT NOT NULL DEFAULT 14,
    theme TEXT NOT NULL DEFAULT 'LIGHT',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT user_editor_preferences_font_size CHECK (font_size_px BETWEEN 12 AND 26),
    CONSTRAINT user_editor_preferences_theme CHECK (theme IN ('LIGHT', 'DARK'))
);

CREATE TABLE latex_core.application_branding (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    logo_blob_hash TEXT,
    logo_media_type TEXT,
    logo_width INTEGER,
    logo_height INTEGER,
    updated_by_user_id UUID REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT application_branding_hash CHECK (
        logo_blob_hash IS NULL OR logo_blob_hash ~ '^[0-9a-f]{64}$'
    ),
    CONSTRAINT application_branding_media_type CHECK (
        logo_media_type IS NULL OR logo_media_type IN ('image/png', 'image/jpeg', 'image/webp')
    ),
    CONSTRAINT application_branding_dimensions CHECK (
        (logo_blob_hash IS NULL AND logo_media_type IS NULL AND logo_width IS NULL AND logo_height IS NULL) OR
        (logo_blob_hash IS NOT NULL AND logo_media_type IS NOT NULL AND
         logo_width BETWEEN 1 AND 2048 AND logo_height BETWEEN 1 AND 2048)
    )
);

INSERT INTO latex_core.application_branding (singleton) VALUES (TRUE);

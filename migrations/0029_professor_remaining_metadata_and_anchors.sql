-- Project-scoped institutional display labels and truthful reviewed source ranges.
-- Canonical VCAP identities remain unchanged and historical anchors are not reconstructed.

ALTER TABLE latex_core.paper_project_metadata
    ADD COLUMN department_display_names JSONB NOT NULL DEFAULT '[]'::jsonb,
    ADD COLUMN school_display_names JSONB NOT NULL DEFAULT '[]'::jsonb,
    ADD CONSTRAINT paper_project_metadata_department_names_array
        CHECK (jsonb_typeof(department_display_names) = 'array'),
    ADD CONSTRAINT paper_project_metadata_school_names_array
        CHECK (jsonb_typeof(school_display_names) = 'array');

ALTER TABLE latex_core.review_source_anchors
    ADD COLUMN start_line INTEGER,
    ADD COLUMN end_line INTEGER,
    ADD CONSTRAINT review_source_anchors_line_pair CHECK (
        (start_line IS NULL AND end_line IS NULL) OR
        (start_line >= 1 AND end_line >= start_line)
    );

COMMENT ON COLUMN latex_core.paper_project_metadata.department_display_names IS
    'Paper-scoped labels for applicable canonical department IDs when VCAP has no display-name column.';
COMMENT ON COLUMN latex_core.paper_project_metadata.school_display_names IS
    'Paper-scoped labels for applicable canonical school IDs when VCAP has no display-name column.';
COMMENT ON COLUMN latex_core.review_source_anchors.start_line IS
    'Optional line observed in the immutable reviewed source when the anchor was created; never reconstructed later.';
COMMENT ON COLUMN latex_core.review_source_anchors.end_line IS
    'Optional inclusive ending line observed with start_line in the immutable reviewed source.';

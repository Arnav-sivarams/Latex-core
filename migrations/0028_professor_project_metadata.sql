-- Project-specific semantic metadata and explicit Main-template arrangements.
-- Institutional VCAP source tables remain unchanged.

ALTER TABLE latex_core.templates
    ADD COLUMN front_matter_arrangement TEXT NOT NULL DEFAULT 'REPORT_CONTENT_ONLY',
    ADD CONSTRAINT templates_front_matter_arrangement CHECK (front_matter_arrangement IN (
        'REPORT_CONTENT_ONLY', 'SEPARATE_FILES', 'SINGLE_SOURCE'
    ));

UPDATE latex_core.templates
SET front_matter_arrangement = 'SEPARATE_FILES'
WHERE front_matter_compatible;

CREATE TABLE latex_core.paper_project_metadata (
    paper_team_id UUID PRIMARY KEY REFERENCES latex_core.paper_teams(id) ON DELETE RESTRICT,
    executive_summary TEXT,
    project_type TEXT,
    datasets JSONB NOT NULL DEFAULT '[]'::jsonb,
    source_code_snippets JSONB NOT NULL DEFAULT '[]'::jsonb,
    publications JSONB NOT NULL DEFAULT '[]'::jsonb,
    setup_completed_at TIMESTAMPTZ,
    revision BIGINT NOT NULL DEFAULT 1,
    updated_by_user_id UUID REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT paper_project_metadata_project_type CHECK (
        project_type IS NULL OR project_type IN ('capstone', 'project-1')
    ),
    CONSTRAINT paper_project_metadata_summary_length CHECK (
        executive_summary IS NULL OR char_length(executive_summary) <= 20000
    ),
    CONSTRAINT paper_project_metadata_datasets_array CHECK (jsonb_typeof(datasets) = 'array'),
    CONSTRAINT paper_project_metadata_snippets_array CHECK (jsonb_typeof(source_code_snippets) = 'array'),
    CONSTRAINT paper_project_metadata_publications_array CHECK (jsonb_typeof(publications) = 'array'),
    CONSTRAINT paper_project_metadata_revision_positive CHECK (revision > 0)
);

COMMENT ON TABLE latex_core.paper_project_metadata IS
    'Typed Team-entered project metadata. Canonical title, people, programme, department, school, term, and Guide identities remain resolved from their authoritative records.';

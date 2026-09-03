-- V2.3 Run 2: immutable Front Matter Packs and rendered Team document metadata.
-- This is an application concern and intentionally adds nothing to vcap.

ALTER TABLE latex_core.templates
    ADD COLUMN front_matter_compatible BOOLEAN NOT NULL DEFAULT FALSE;

CREATE TABLE latex_core.front_matter_packs (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    description TEXT,
    manifest_json JSONB NOT NULL,
    content_hash TEXT NOT NULL,
    created_by_user_id UUID NOT NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    archived_at TIMESTAMPTZ,
    CONSTRAINT front_matter_packs_name_length CHECK (char_length(name) BETWEEN 1 AND 200),
    CONSTRAINT front_matter_packs_description_length CHECK (description IS NULL OR char_length(description) <= 2000),
    CONSTRAINT front_matter_packs_manifest_object CHECK (jsonb_typeof(manifest_json) = 'object'),
    CONSTRAINT front_matter_packs_content_hash CHECK (content_hash ~ '^[0-9a-f]{64}$')
);

CREATE TABLE latex_core.front_matter_pack_files (
    pack_id UUID NOT NULL REFERENCES latex_core.front_matter_packs(id) ON DELETE RESTRICT,
    path TEXT NOT NULL,
    blob_hash TEXT NOT NULL,
    size_bytes BIGINT NOT NULL,
    media_type TEXT NOT NULL,
    PRIMARY KEY (pack_id, path),
    CONSTRAINT front_matter_pack_files_path_length CHECK (char_length(path) BETWEEN 1 AND 1024),
    CONSTRAINT front_matter_pack_files_blob_hash CHECK (blob_hash ~ '^[0-9a-f]{64}$'),
    CONSTRAINT front_matter_pack_files_size_nonnegative CHECK (size_bytes >= 0),
    CONSTRAINT front_matter_pack_files_media_type_length CHECK (char_length(media_type) BETWEEN 1 AND 100)
);

CREATE TABLE latex_core.paper_front_matter_pins (
    paper_team_id UUID PRIMARY KEY REFERENCES latex_core.paper_teams(id) ON DELETE RESTRICT,
    front_matter_pack_id UUID NOT NULL REFERENCES latex_core.front_matter_packs(id) ON DELETE RESTRICT,
    dominant_programme_code TEXT REFERENCES vcap.programmes(programme_code) ON DELETE RESTRICT,
    resolution_method TEXT NOT NULL,
    assigned_by_user_id UUID REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    status TEXT NOT NULL DEFAULT 'READY',
    missing_required_fields JSONB NOT NULL DEFAULT '[]'::jsonb,
    last_error TEXT,
    pinned_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT paper_front_matter_resolution_method CHECK (resolution_method IN (
        'PROGRAMME_DEFAULT', 'GLOBAL_FALLBACK', 'MANUAL_OVERRIDE'
    )),
    CONSTRAINT paper_front_matter_status CHECK (status IN (
        'READY', 'NEEDS_INFORMATION', 'INCOMPATIBLE_TEMPLATE', 'RENDER_FAILED'
    )),
    CONSTRAINT paper_front_matter_missing_array CHECK (jsonb_typeof(missing_required_fields) = 'array'),
    CONSTRAINT paper_front_matter_error_length CHECK (last_error IS NULL OR char_length(last_error) <= 4000)
);

CREATE TABLE latex_core.paper_front_matter_values (
    paper_team_id UUID NOT NULL REFERENCES latex_core.paper_teams(id) ON DELETE RESTRICT,
    field_key TEXT NOT NULL,
    value_json JSONB NOT NULL,
    value_source TEXT NOT NULL,
    updated_by_user_id UUID REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (paper_team_id, field_key),
    CONSTRAINT paper_front_matter_field_key_length CHECK (char_length(field_key) BETWEEN 1 AND 100),
    CONSTRAINT paper_front_matter_value_source CHECK (value_source IN ('AUTO', 'PACK_DEFAULT', 'TEAM_OVERRIDE'))
);

CREATE TABLE latex_core.paper_front_matter_sections (
    paper_team_id UUID NOT NULL REFERENCES latex_core.paper_teams(id) ON DELETE RESTRICT,
    section_key TEXT NOT NULL,
    enabled BOOLEAN NOT NULL,
    updated_by_user_id UUID REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (paper_team_id, section_key),
    CONSTRAINT paper_front_matter_section_key_length CHECK (char_length(section_key) BETWEEN 1 AND 100)
);

-- Institutional edits enqueue affected Teams; a bounded worker can claim rows
-- transactionally without introducing another broker.
CREATE TABLE latex_core.front_matter_rerender_queue (
    paper_team_id UUID PRIMARY KEY REFERENCES latex_core.paper_teams(id) ON DELETE RESTRICT,
    reason TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'QUEUED',
    attempts INT NOT NULL DEFAULT 0,
    available_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    claimed_at TIMESTAMPTZ,
    last_error TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT front_matter_rerender_state CHECK (state IN ('QUEUED', 'CLAIMED', 'FAILED')),
    CONSTRAINT front_matter_rerender_attempts CHECK (attempts >= 0),
    CONSTRAINT front_matter_rerender_reason_length CHECK (char_length(reason) BETWEEN 1 AND 200),
    CONSTRAINT front_matter_rerender_error_length CHECK (last_error IS NULL OR char_length(last_error) <= 4000)
);

CREATE TABLE latex_core.paper_team_materialization_warnings (
    id UUID PRIMARY KEY,
    paper_team_id UUID NOT NULL REFERENCES latex_core.paper_teams(id) ON DELETE RESTRICT,
    warning_code TEXT NOT NULL,
    detail TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at TIMESTAMPTZ,
    CONSTRAINT paper_team_materialization_warning_code CHECK (warning_code IN (
        'FRONT_MATTER_TEMPLATE_INCOMPATIBLE', 'FRONT_MATTER_RENDER_FAILED'
    )),
    CONSTRAINT paper_team_materialization_warning_detail CHECK (char_length(detail) BETWEEN 1 AND 4000)
);

ALTER TABLE latex_core.programme_template_defaults
    ALTER COLUMN template_id DROP NOT NULL,
    ADD COLUMN front_matter_pack_id UUID REFERENCES latex_core.front_matter_packs(id) ON DELETE RESTRICT;

ALTER TABLE latex_core.institution_template_config
    ADD COLUMN global_fallback_front_matter_pack_id UUID REFERENCES latex_core.front_matter_packs(id) ON DELETE RESTRICT;

ALTER TABLE latex_core.paper_versions DROP CONSTRAINT paper_versions_type_values;
ALTER TABLE latex_core.paper_versions
    ADD CONSTRAINT paper_versions_type_values CHECK (version_type IN (
        'manual_checkpoint', 'compile_checkpoint', 'review_round',
        'pre_restore_safety', 'admin_restoration', 'team_revert', 'template_update', 'submission',
        'front_matter_update'
    ));

CREATE INDEX front_matter_pack_files_blob_idx
    ON latex_core.front_matter_pack_files (blob_hash);
CREATE INDEX paper_front_matter_pins_pack_idx
    ON latex_core.paper_front_matter_pins (front_matter_pack_id, paper_team_id);
CREATE INDEX programme_template_defaults_front_matter_idx
    ON latex_core.programme_template_defaults (front_matter_pack_id, programme_code)
    WHERE front_matter_pack_id IS NOT NULL;
CREATE INDEX front_matter_rerender_claim_idx
    ON latex_core.front_matter_rerender_queue (state, available_at, updated_at);
CREATE INDEX paper_team_materialization_warnings_team_idx
    ON latex_core.paper_team_materialization_warnings (paper_team_id, resolved_at, created_at);

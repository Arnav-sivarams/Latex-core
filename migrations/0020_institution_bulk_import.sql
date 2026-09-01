-- V2.2 Run 1: additive institutional source-of-record and bulk-import foundation.
-- Login identities and V2 capabilities remain authoritative in latex_core.
CREATE SCHEMA IF NOT EXISTS vcap;

CREATE TABLE vcap.departments (
    department_id UUID PRIMARY KEY
);

CREATE TABLE vcap.admins (
    admin_id VARCHAR PRIMARY KEY,
    email VARCHAR,
    name VARCHAR,
    pfp VARCHAR
);

CREATE TABLE vcap.faculty (
    faculty_id VARCHAR PRIMARY KEY,
    name VARCHAR,
    email VARCHAR,
    dept_id UUID REFERENCES vcap.departments(department_id) ON DELETE RESTRICT,
    honorific TEXT,
    designation TEXT,
    status TEXT
);

CREATE TABLE vcap.programmes (
    programme_code TEXT PRIMARY KEY,
    hod_id TEXT REFERENCES vcap.faculty(faculty_id) ON DELETE RESTRICT
);

CREATE TABLE vcap.schools (
    school_id TEXT PRIMARY KEY
);

CREATE TABLE vcap.students (
    reg_no VARCHAR PRIMARY KEY,
    name VARCHAR,
    email VARCHAR,
    programme_code TEXT REFERENCES vcap.programmes(programme_code) ON DELETE RESTRICT
);

CREATE TABLE vcap.student_course_registrations (
    student_reg_no TEXT NOT NULL REFERENCES vcap.students(reg_no) ON DELETE RESTRICT,
    course_id TEXT NOT NULL,
    academic_year TEXT NOT NULL,
    semester TEXT NOT NULL,
    registration_status TEXT,
    PRIMARY KEY (student_reg_no, course_id, academic_year, semester)
);

CREATE TABLE vcap.faculty_guide_capacity (
    capacity_id UUID PRIMARY KEY,
    faculty_id VARCHAR REFERENCES vcap.faculty(faculty_id) ON DELETE RESTRICT,
    academic_year TEXT,
    ug_max_projects INT,
    pg_max_projects INT,
    integrated_pg_max_projects INT,
    status TEXT,
    CONSTRAINT faculty_guide_capacity_nonnegative CHECK (
        (ug_max_projects IS NULL OR ug_max_projects >= 0) AND
        (pg_max_projects IS NULL OR pg_max_projects >= 0) AND
        (integrated_pg_max_projects IS NULL OR integrated_pg_max_projects >= 0)
    )
);

-- dept_id intentionally preserves the supplied VARCHAR contract. The trigger
-- below validates UUID syntax and existence without a cross-type foreign key.
CREATE TABLE vcap.department_roles (
    id SERIAL PRIMARY KEY,
    dept_id VARCHAR NOT NULL,
    role_type VARCHAR,
    faculty_id VARCHAR REFERENCES vcap.faculty(faculty_id) ON DELETE RESTRICT
);

CREATE FUNCTION vcap.validate_department_role_department()
RETURNS trigger
LANGUAGE plpgsql
AS $$
DECLARE
    parsed_department_id UUID;
BEGIN
    BEGIN
        parsed_department_id := NEW.dept_id::UUID;
    EXCEPTION WHEN invalid_text_representation THEN
        RAISE EXCEPTION 'department_roles.dept_id must be a UUID: %', NEW.dept_id
            USING ERRCODE = '22P02';
    END;
    IF NOT EXISTS (
        SELECT 1 FROM vcap.departments
        WHERE department_id = parsed_department_id
    ) THEN
        RAISE EXCEPTION 'department_roles.dept_id does not reference a department: %', NEW.dept_id
            USING ERRCODE = '23503';
    END IF;
    RETURN NEW;
END
$$;

CREATE TRIGGER department_roles_department_guard
BEFORE INSERT OR UPDATE OF dept_id ON vcap.department_roles
FOR EACH ROW EXECUTE FUNCTION vcap.validate_department_role_department();

CREATE TABLE vcap.faculty_roles (
    role_id UUID PRIMARY KEY,
    faculty_id VARCHAR REFERENCES vcap.faculty(faculty_id) ON DELETE RESTRICT,
    role_type TEXT,
    school_id TEXT REFERENCES vcap.schools(school_id) ON DELETE RESTRICT,
    department_id UUID REFERENCES vcap.departments(department_id) ON DELETE RESTRICT,
    programme_code TEXT REFERENCES vcap.programmes(programme_code) ON DELETE RESTRICT,
    status TEXT
);

CREATE TABLE vcap.student_user_links (
    reg_no VARCHAR PRIMARY KEY REFERENCES vcap.students(reg_no) ON DELETE RESTRICT,
    user_id UUID UNIQUE REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    match_method TEXT,
    status TEXT NOT NULL,
    linked_at TIMESTAMPTZ,
    CONSTRAINT student_user_links_status_values CHECK (
        status IN ('LINKED', 'UNLINKED', 'AMBIGUOUS', 'ROLE_INCOMPATIBLE')
    ),
    CONSTRAINT student_user_links_shape CHECK (
        (status = 'LINKED' AND user_id IS NOT NULL AND linked_at IS NOT NULL) OR
        (status <> 'LINKED' AND user_id IS NULL AND linked_at IS NULL)
    )
);

CREATE TABLE vcap.faculty_user_links (
    faculty_id VARCHAR PRIMARY KEY REFERENCES vcap.faculty(faculty_id) ON DELETE RESTRICT,
    user_id UUID UNIQUE REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    match_method TEXT,
    status TEXT NOT NULL,
    linked_at TIMESTAMPTZ,
    CONSTRAINT faculty_user_links_status_values CHECK (
        status IN ('LINKED', 'UNLINKED', 'AMBIGUOUS', 'ROLE_INCOMPATIBLE')
    ),
    CONSTRAINT faculty_user_links_shape CHECK (
        (status = 'LINKED' AND user_id IS NOT NULL AND linked_at IS NOT NULL) OR
        (status <> 'LINKED' AND user_id IS NULL AND linked_at IS NULL)
    )
);

CREATE TABLE vcap.admin_user_links (
    admin_id VARCHAR PRIMARY KEY REFERENCES vcap.admins(admin_id) ON DELETE RESTRICT,
    user_id UUID UNIQUE REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    match_method TEXT,
    status TEXT NOT NULL,
    linked_at TIMESTAMPTZ,
    CONSTRAINT admin_user_links_status_values CHECK (
        status IN ('LINKED', 'UNLINKED', 'AMBIGUOUS', 'ROLE_INCOMPATIBLE')
    ),
    CONSTRAINT admin_user_links_shape CHECK (
        (status = 'LINKED' AND user_id IS NOT NULL AND linked_at IS NOT NULL) OR
        (status <> 'LINKED' AND user_id IS NULL AND linked_at IS NULL)
    )
);

CREATE TABLE latex_core.institution_import_jobs (
    id UUID PRIMARY KEY,
    import_kind TEXT NOT NULL,
    mode TEXT NOT NULL,
    original_filename TEXT NOT NULL,
    content_sha256 TEXT NOT NULL,
    file_type TEXT NOT NULL,
    submitted_by_user_id UUID NOT NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    status TEXT NOT NULL DEFAULT 'UPLOADED',
    total_rows BIGINT NOT NULL DEFAULT 0,
    inserted_rows BIGINT NOT NULL DEFAULT 0,
    updated_rows BIGINT NOT NULL DEFAULT 0,
    skipped_rows BIGINT NOT NULL DEFAULT 0,
    error_rows BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    validated_at TIMESTAMPTZ,
    applied_at TIMESTAMPTZ,
    CONSTRAINT institution_import_jobs_mode_values CHECK (
        mode IN ('VALIDATE_ONLY', 'MERGE', 'ADD_ONLY')
    ),
    CONSTRAINT institution_import_jobs_status_values CHECK (
        status IN ('UPLOADED', 'VALIDATING', 'VALIDATED', 'APPLYING', 'APPLIED', 'PARTIAL', 'FAILED')
    ),
    CONSTRAINT institution_import_jobs_file_type_values CHECK (file_type IN ('CSV', 'XLSX')),
    CONSTRAINT institution_import_jobs_digest CHECK (content_sha256 ~ '^[0-9a-f]{64}$'),
    CONSTRAINT institution_import_jobs_counts_nonnegative CHECK (
        total_rows >= 0 AND inserted_rows >= 0 AND updated_rows >= 0 AND
        skipped_rows >= 0 AND error_rows >= 0
    )
);

CREATE TABLE latex_core.institution_import_rows (
    job_id UUID NOT NULL REFERENCES latex_core.institution_import_jobs(id) ON DELETE RESTRICT,
    source_table_or_sheet TEXT NOT NULL,
    row_number BIGINT NOT NULL,
    natural_key JSONB NOT NULL,
    payload JSONB NOT NULL,
    action TEXT NOT NULL,
    status TEXT NOT NULL,
    error_code TEXT,
    error_message TEXT,
    PRIMARY KEY (job_id, source_table_or_sheet, row_number),
    CONSTRAINT institution_import_rows_row_positive CHECK (row_number >= 2),
    CONSTRAINT institution_import_rows_json_objects CHECK (
        jsonb_typeof(natural_key) = 'object' AND jsonb_typeof(payload) = 'object'
    ),
    CONSTRAINT institution_import_rows_action_values CHECK (
        action IN ('INSERT', 'UPDATE', 'SKIP', 'INVALID', 'MATERIALIZE')
    ),
    CONSTRAINT institution_import_rows_status_values CHECK (
        status IN ('VALID', 'ERROR', 'APPLIED', 'SKIPPED', 'UNRESOLVED')
    )
);

CREATE TABLE vcap.paper_assignment_groups (
    external_team_key TEXT PRIMARY KEY,
    team_name TEXT NOT NULL,
    academic_year TEXT,
    semester TEXT,
    status TEXT
);

CREATE TABLE vcap.paper_assignment_students (
    external_team_key TEXT NOT NULL REFERENCES vcap.paper_assignment_groups(external_team_key) ON DELETE RESTRICT,
    student_reg_no VARCHAR NOT NULL REFERENCES vcap.students(reg_no) ON DELETE RESTRICT,
    writer_order INT NOT NULL,
    is_leader BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (external_team_key, student_reg_no),
    CONSTRAINT paper_assignment_students_writer_order_positive CHECK (writer_order > 0),
    UNIQUE (external_team_key, writer_order)
);
CREATE UNIQUE INDEX paper_assignment_students_one_leader_idx
    ON vcap.paper_assignment_students (external_team_key) WHERE is_leader;

CREATE TABLE vcap.paper_assignment_mentors (
    external_team_key TEXT NOT NULL REFERENCES vcap.paper_assignment_groups(external_team_key) ON DELETE RESTRICT,
    faculty_id VARCHAR NOT NULL REFERENCES vcap.faculty(faculty_id) ON DELETE RESTRICT,
    PRIMARY KEY (external_team_key, faculty_id)
);

CREATE TABLE latex_core.external_paper_team_links (
    external_team_key TEXT PRIMARY KEY REFERENCES vcap.paper_assignment_groups(external_team_key) ON DELETE RESTRICT,
    paper_team_id UUID UNIQUE NOT NULL REFERENCES latex_core.paper_teams(id) ON DELETE RESTRICT,
    source_import_job_id UUID NOT NULL REFERENCES latex_core.institution_import_jobs(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE latex_core.programme_template_defaults (
    programme_code TEXT PRIMARY KEY REFERENCES vcap.programmes(programme_code) ON DELETE RESTRICT,
    template_id UUID NOT NULL REFERENCES latex_core.templates(id) ON DELETE RESTRICT,
    updated_by_user_id UUID NOT NULL REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE latex_core.institution_template_config (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    global_fallback_template_id UUID REFERENCES latex_core.templates(id) ON DELETE RESTRICT,
    updated_by_user_id UUID REFERENCES latex_core.users(id) ON DELETE RESTRICT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
INSERT INTO latex_core.institution_template_config (singleton) VALUES (TRUE);

CREATE TABLE latex_core.paper_template_resolutions (
    paper_team_id UUID PRIMARY KEY REFERENCES latex_core.paper_teams(id) ON DELETE RESTRICT,
    selected_template_id UUID NOT NULL REFERENCES latex_core.templates(id) ON DELETE RESTRICT,
    dominant_programme_code TEXT REFERENCES vcap.programmes(programme_code) ON DELETE RESTRICT,
    resolution_method TEXT NOT NULL,
    manual_override BOOLEAN NOT NULL DEFAULT FALSE,
    resolved_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT paper_template_resolutions_method_values CHECK (
        resolution_method IN ('MODE', 'TIE_FIRST_WRITER', 'GLOBAL_FALLBACK', 'MANUAL_OVERRIDE')
    ),
    CONSTRAINT paper_template_resolutions_manual_shape CHECK (
        manual_override = (resolution_method = 'MANUAL_OVERRIDE')
    )
);

ALTER TABLE latex_core.paper_team_members
    ADD COLUMN writer_order INT,
    ADD CONSTRAINT paper_team_members_writer_order_positive
        CHECK (writer_order IS NULL OR writer_order > 0);

WITH ordered_writers AS (
    SELECT m.paper_team_id, m.user_id,
           row_number() OVER (
               PARTITION BY m.paper_team_id ORDER BY m.created_at, m.user_id
           )::INT AS writer_order
    FROM latex_core.paper_team_members m
    JOIN latex_core.global_user_roles role ON role.user_id = m.user_id
    WHERE role.role = 'writer'
)
UPDATE latex_core.paper_team_members member
SET writer_order = ordered.writer_order
FROM ordered_writers ordered
WHERE member.paper_team_id = ordered.paper_team_id
  AND member.user_id = ordered.user_id;

CREATE UNIQUE INDEX paper_team_members_team_writer_order_idx
    ON latex_core.paper_team_members (paper_team_id, writer_order)
    WHERE writer_order IS NOT NULL;
CREATE INDEX paper_team_members_user_order_idx
    ON latex_core.paper_team_members (user_id, writer_order, paper_team_id);

CREATE INDEX admins_normalized_email_idx ON vcap.admins ((lower(btrim(email)))) WHERE email IS NOT NULL;
CREATE INDEX faculty_normalized_email_idx ON vcap.faculty ((lower(btrim(email)))) WHERE email IS NOT NULL;
CREATE INDEX students_normalized_email_idx ON vcap.students ((lower(btrim(email)))) WHERE email IS NOT NULL;
CREATE INDEX user_credentials_normalized_email_idx ON latex_core.user_credentials ((lower(btrim(email))));
CREATE INDEX students_programme_idx ON vcap.students (programme_code, reg_no);
CREATE INDEX registrations_student_period_status_idx
    ON vcap.student_course_registrations (student_reg_no, academic_year, semester, registration_status);
CREATE INDEX faculty_department_status_idx ON vcap.faculty (dept_id, status, faculty_id);
CREATE INDEX faculty_capacity_faculty_year_status_idx
    ON vcap.faculty_guide_capacity (faculty_id, academic_year, status);
CREATE INDEX department_roles_department_idx ON vcap.department_roles (dept_id, role_type);
CREATE INDEX faculty_roles_faculty_status_idx ON vcap.faculty_roles (faculty_id, status);
CREATE INDEX institution_import_jobs_created_idx
    ON latex_core.institution_import_jobs (created_at DESC, id);
CREATE INDEX institution_import_jobs_status_created_idx
    ON latex_core.institution_import_jobs (status, created_at DESC, id);
CREATE INDEX institution_import_jobs_sha_idx
    ON latex_core.institution_import_jobs (content_sha256, created_at DESC);
CREATE INDEX institution_import_rows_job_status_idx
    ON latex_core.institution_import_rows (job_id, status, source_table_or_sheet, row_number);
CREATE INDEX paper_assignment_students_student_idx
    ON vcap.paper_assignment_students (student_reg_no, external_team_key);
CREATE INDEX paper_assignment_mentors_faculty_idx
    ON vcap.paper_assignment_mentors (faculty_id, external_team_key);
CREATE INDEX external_paper_team_links_job_idx
    ON latex_core.external_paper_team_links (source_import_job_id, external_team_key);
CREATE INDEX programme_template_defaults_template_idx
    ON latex_core.programme_template_defaults (template_id, programme_code);
CREATE INDEX paper_template_resolutions_programme_template_idx
    ON latex_core.paper_template_resolutions (dominant_programme_code, selected_template_id, resolution_method);

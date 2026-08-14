-- M11: immutable server-managed reusable project seeds.
CREATE TABLE latex_core.templates (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    description TEXT NULL,
    main_file TEXT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT templates_name_length CHECK (char_length(name) BETWEEN 1 AND 200),
    CONSTRAINT templates_description_length CHECK (description IS NULL OR char_length(description) <= 2000),
    CONSTRAINT templates_main_file_length CHECK (main_file IS NULL OR char_length(main_file) BETWEEN 1 AND 1024)
);

CREATE TABLE latex_core.template_files (
    template_id UUID NOT NULL REFERENCES latex_core.templates(id) ON DELETE RESTRICT,
    path TEXT NOT NULL,
    blob_hash TEXT NOT NULL,
    size_bytes BIGINT NOT NULL,
    PRIMARY KEY (template_id, path),
    CONSTRAINT template_files_path_length CHECK (char_length(path) BETWEEN 1 AND 1024),
    CONSTRAINT template_files_blob_digest CHECK (blob_hash ~ '^[0-9a-f]{64}$'),
    CONSTRAINT template_files_size_nonnegative CHECK (size_bytes >= 0)
);

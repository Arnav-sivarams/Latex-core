-- Template metadata may be removed without affecting immutable blobs or user projects.
ALTER TABLE latex_core.template_files
    DROP CONSTRAINT template_files_template_id_fkey,
    ADD CONSTRAINT template_files_template_id_fkey
        FOREIGN KEY (template_id) REFERENCES latex_core.templates(id) ON DELETE CASCADE;

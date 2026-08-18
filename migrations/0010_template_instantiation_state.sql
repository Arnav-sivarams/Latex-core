-- Template-created team projects retain their release-critical provenance and
-- default policy independently of any later audience or rule changes.
ALTER TABLE latex_core.templates
    ADD COLUMN policy_default TEXT NOT NULL DEFAULT 'editable',
    ADD CONSTRAINT templates_policy_default_values CHECK (policy_default IN ('editable', 'read_only', 'managed'));

ALTER TABLE latex_core.team_projects
    DROP CONSTRAINT team_projects_template_id_fkey,
    ADD CONSTRAINT team_projects_template_id_fkey
        FOREIGN KEY (template_id) REFERENCES latex_core.templates(id) ON DELETE RESTRICT;

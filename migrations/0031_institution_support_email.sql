-- Reuse the existing mutable institution settings authority. Existing rows remain valid.
ALTER TABLE latex_core.institution_template_config ADD COLUMN support_email TEXT;
ALTER TABLE latex_core.institution_template_config ADD CONSTRAINT institution_support_email_format
    CHECK (support_email IS NULL OR (char_length(support_email) <= 254 AND support_email ~ '^[^[:space:]@]+@[^[:space:]@]+[.][^[:space:]@]+$'));

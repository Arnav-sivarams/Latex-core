ALTER TABLE latex_core.compile_jobs
    DROP CONSTRAINT compile_jobs_engine_values;

ALTER TABLE latex_core.compile_jobs
    ADD CONSTRAINT compile_jobs_engine_values
    CHECK (engine IN ('latex', 'pdflatex', 'lualatex', 'xelatex'));

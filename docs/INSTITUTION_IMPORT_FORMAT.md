# Institution CSV/XLSX import format

The Admin-only API accepts bytes through `multipart/form-data`; server filesystem paths are not accepted. Upload field names are `file`, `mode`, and optional `target_table`. Limits default to 32 MiB, 20 worksheets, 100,000 data rows per sheet, 128 columns, and 16,384 characters per cell.

## Modes

- `VALIDATE_ONLY` validates and stages provenance without canonical mutation. It cannot be applied.
- `MERGE` inserts unseen keys and updates matching keys. Rows absent from a later file are retained.
- `ADD_ONLY` inserts unseen keys and reports existing keys as skipped. It never updates or deletes them.

There is no replace or hard-delete mode. Reimports remain visible as separate SHA-addressed jobs. External Team keys prevent duplicate Paper Teams.

## XLSX worksheets and columns

Worksheet names are exact. Unknown non-empty sheets are rejected. Columns not listed below are rejected; columns shown in **bold** are required.

| Worksheet | Header, in any order |
|---|---|
| `departments` | **`department_id`** |
| `admins` | **`admin_id`**, `email`, `name`, `pfp` |
| `faculty` | **`faculty_id`**, `name`, `email`, `dept_id`, `honorific`, `designation`, `status` |
| `programmes` | **`programme_code`**, `hod_id` |
| `schools` | **`school_id`** |
| `students` | **`reg_no`**, `name`, `email`, `programme_code` |
| `student_course_registrations` | **`student_reg_no`**, **`course_id`**, **`academic_year`**, **`semester`**, `registration_status` |
| `faculty_guide_capacity` | **`capacity_id`**, `faculty_id`, `academic_year`, `ug_max_projects`, `pg_max_projects`, `integrated_pg_max_projects`, `status` |
| `department_roles` | **`id`**, **`dept_id`**, `role_type`, `faculty_id` |
| `faculty_roles` | **`role_id`**, `faculty_id`, `role_type`, `school_id`, `department_id`, `programme_code`, `status` |
| `paper_teams` | **`external_team_key`**, **`team_name`**, `academic_year`, `semester`, `status` |
| `paper_team_writers` | **`external_team_key`**, **`student_reg_no`**, **`writer_order`**, **`is_leader`** |
| `paper_team_mentors` | **`external_team_key`**, **`faculty_id`** |

Example headers:

```csv
reg_no,name,email,programme_code
```

```csv
external_team_key,student_reg_no,writer_order,is_leader
```

Examples intentionally omit personal data.

## CSV

One CSV represents one table. Supply `target_table`, or name the file exactly after a supported table, such as `students.csv`. Unknown names are never guessed. UTF-8 CSV quoting follows RFC-style CSV rules.

## Validation and application

Validation checks column shape, duplicate columns and keys, required key values, UUIDs, nonnegative capacities, positive IDs/order, normalized email shape, foreign keys, department-role UUID existence, Writer-order uniqueness, one Leader at most, and at least one Writer when Team and Writer sheets are supplied together. Formulas in identity/key cells are rejected, not evaluated.

The dependency order is departments, schools, admins, faculty, programmes, students, registrations, capacities, department roles, faculty roles, Team groups, Writers, and Mentors. PostgreSQL constraints remain enabled.

After application, exact normalized-email linking runs without changing global roles. A Team with missing links, an incompatible Writer/Mentor role, or no explicit Leader is marked unresolved and is not partially activated. Other valid Teams may materialize. Error rows remain downloadable from the job's `errors.csv` endpoint.

Manual Team creation remains supported and requires an Admin-selected Leader.

## Admin import wizard

The four steps are select file/mode, validate, review, and apply. CSV requires one of the documented target tables; an exact filename preselects it, while an unknown filename is never guessed. XLSX uses worksheet names and hides the CSV target.

Validation results show job identity, filename, SHA-256, mode/type/status, aggregate actions, and per-sheet/table counts. Only a bounded row preview is rendered. `VALIDATE_ONLY`, failed/error validation, and already-applied jobs cannot be applied. PostgreSQL row locks and external Team keys make repeated Apply safe from duplicate canonical rows or Teams.

History uses page/limit plus optional filename/job, status, mode, and file-type filters. Uploaded bytes are not retained in history and file contents are not written to Audit. Error CSV cells beginning with `=`, `+`, `-`, or `@` are prefixed to prevent spreadsheet formula execution.

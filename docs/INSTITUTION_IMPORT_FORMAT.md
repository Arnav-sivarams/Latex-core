# Institution data management

V2 Admins manage institutional source records from **INSTITUTION DATA** or **DATA IMPORT**. Both paths use the same server-side parser, validation, dependency checks, and apply operations. Writer and Mentor accounts cannot use these endpoints.

## Add, Edit, and Delete

- **Add** inserts keys that are not present. Existing keys are reported as already existing and remain unchanged.
- **Edit** updates only supplied mutable fields on a matching canonical key. An unknown key is `NOT_FOUND`; it is not inserted. Key fields are immutable in this operation.
- **Delete** removes only keys explicitly listed in the input. A record missing from a file is never deleted. Before apply, the server lists dependencies and blocks destructive operations with `DELETE_BLOCKED_DEPENDENCY`.

Delete never cascades into a LaTeX Core user, materialized Paper Team, paper workspace, version, review, or build artifact. Deleting a materialized assignment source requires the separate Paper Team lifecycle workflow. Missing Delete keys are reported safely as `NOT_FOUND`.

## Multi-file import

Drop or browse for as many as 20 CSV/XLSX files (32 MiB per file, 64 MiB combined). Files can be removed before review. A duplicate name-and-content pair is rejected. Each worksheet/file remains a child job with its checksum and error CSV, while the normal history shows one concise batch.

CSV datasets are inferred from a normalized filename such as `students.csv`, `students_2026.csv`, or `VIT_students.csv`, then from a compatible header signature. If more than one dataset matches, the Admin is asked once, “What data is this?” XLSX datasets are inferred from recognized worksheet names. Target selection is not required for an ordinary detected file.

The complete batch is validated together against the canonical database plus staged rows from every selected file. File/drop order does not matter. Apply uses a dependency-safe order with PostgreSQL constraints enabled:

1. Departments, Schools, Admins
2. Faculty, Programmes, Students
3. Registrations, capacity, and role metadata
4. Paper assignment groups, ordered Writers/Leader, and Mentors
5. Identity reconciliation and Paper Team materialization

No compilation is triggered. For fully valid assignment batches, Team materialization is part of the one Apply action; Retry Team Materialization remains recovery-only.

## Supported datasets and canonical keys

| Dataset | Canonical key | Mutable fields |
|---|---|---|
| Departments | `department_id` | — |
| Admins | `admin_id` | `email`, `name`, `pfp` |
| Faculty | `faculty_id` | `name`, `email`, `dept_id`, `honorific`, `designation`, `status` |
| Programmes | `programme_code` | `hod_id` |
| Schools | `school_id` | — |
| Students | `reg_no` | `name`, `email`, `programme_code` |
| Course Registrations | `student_reg_no`, `course_id`, `academic_year`, `semester` | `registration_status` |
| Guide Capacity | `capacity_id` | faculty/year/capacity/status fields |
| Department Roles | `id` | department/role/faculty fields |
| Faculty Roles | `role_id` | faculty/scope/role/status fields |
| Paper Assignments | `external_team_key` | team name/year/semester/status |
| Assignment Writers | `external_team_key`, `student_reg_no` | `writer_order`, `is_leader` |
| Assignment Mentors | `external_team_key`, `faculty_id` | — |

Add files must contain the dataset’s required fields. Edit and Delete files may contain only canonical keys plus fields being changed. Values are bounded and validated for UUID, integer, Boolean, normalized email, duplicate key, relationship, Writer order, and Leader rules. Spreadsheet formulas in identity cells are rejected for XLSX. Downloaded error CSV cells are neutralized against formula injection.

## Manual management

INSTITUTION DATA provides server-side search and pagination for every dataset. **+ Add**, **Edit**, and **Delete** open compact forms generated from the same schema. Delete first creates a dependency preview; blocked references are named before any mutation. Paper Assignments expand to ordered Writers, the explicit Leader, Mentors, and materialization state.

Changing a Student programme can affect resolution for future Teams. Existing Team template pins never change automatically and the Edit preview says so.

## Backward compatibility

Existing standalone jobs and their error downloads remain readable as **Legacy single-file imports**. Internally, `ADD_ONLY`, `UPDATE_ONLY`, and `DELETE_ONLY` implement the three user operations. The older `VALIDATE_ONLY` and `MERGE` endpoints remain available for compatibility; absent rows still never imply deletion.

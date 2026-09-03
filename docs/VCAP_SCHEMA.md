# VCAP institutional schema

V2.2 adds the `vcap` PostgreSQL schema as the institutional People & Roles source of record. It does not replace `latex_core.users`, credentials, sessions, global V2 roles, Paper Teams, or Team memberships.

## People and roles

| Table | Primary key | Relationships and notes |
|---|---|---|
| `vcap.departments` | `department_id UUID` | Institutional department identity. |
| `vcap.admins` | `admin_id VARCHAR` | An imported row never grants the V2 Admin role. |
| `vcap.faculty` | `faculty_id VARCHAR` | Optional UUID department reference. |
| `vcap.programmes` | `programme_code TEXT` | Optional HOD faculty reference permits dependency-safe imports. |
| `vcap.schools` | `school_id TEXT` | Institutional school identity. |
| `vcap.students` | `reg_no VARCHAR` | Optional programme reference. |
| `vcap.student_course_registrations` | student, course, year, semester | No courses table is invented. Registrations are not used to infer a student's programme. |
| `vcap.faculty_guide_capacity` | `capacity_id UUID` | Capacity is imported for reporting, not enforced against Teams in Run 1. |
| `vcap.department_roles` | `id SERIAL` | The external `dept_id VARCHAR` contract is preserved. A trigger requires UUID syntax and a matching department; there is deliberately no cross-type FK. |
| `vcap.faculty_roles` | `role_id UUID` | References faculty and optionally school, department, and programme. Imported role semantics are preserved as supplied. |

Foreign keys use `ON DELETE RESTRICT`. Imports are additive and do not hard-delete institutional records.

## Login identity links

`vcap.student_user_links`, `vcap.faculty_user_links`, and `vcap.admin_user_links` bridge institutional identities to canonical `latex_core.users`. Each external identity and each linked user is unique within its link table. Manual-link application additionally checks all three tables, preventing one V2 account from being reused across Student, Faculty, or Admin identities.

Link statuses are:

- `LINKED`: exactly one normalized-email account match exists and the existing V2 role is compatible.
- `UNLINKED`: no account match exists.
- `AMBIGUOUS`: multiple normalized account or institutional identity matches exist.
- `ROLE_INCOMPATIBLE`: the unique account has the wrong existing V2 global role.

An Add operation provisions a missing enabled V2 Writer for each imported Student with a valid unique email. Faculty are provisioned as V2 Mentors only when referenced by `paper_assignment_mentors`. Existing compatible accounts are reused without changing password hashes; incompatible roles are `ROLE_INCOMPATIBLE`. `vcap.admins` never provisions or grants V2 Admin, and unassigned Faculty do not gain Mentor accounts. Automatic reconciliation does not overwrite `MANUAL` links, and unlink is blocked while the identity is used by a non-archived imported Team.

`latex_core.user_credentials.must_change_password` is an application-owned account state. New automatic credentials store an Argon2 hash and set this flag; the temporary plaintext exists only in the immediate apply response. It is not stored in VCAP, import rows, audit metadata, or PostgreSQL credential columns.

## Paper assignments

`vcap.paper_assignment_groups` stores an external Team key and metadata. Ordered Writers live in `vcap.paper_assignment_students`; `writer_order` is positive and unique per Team, and a partial unique index permits at most one imported Leader. `vcap.paper_assignment_mentors` stores faculty assignments.

`latex_core.external_paper_team_links` gives an external key exactly one existing V2 Paper Team. Runtime access continues to come only from `latex_core.paper_team_members`. Imported assignments never create a second Team architecture.

Admin edits of a linked Team update the three `vcap.paper_assignment_*` relations and `latex_core.paper_team_members` in one transaction. The external key, Paper Team ID, workspace, immutable versions, comments, and builds remain unchanged.

## Import and template provenance

`latex_core.institution_import_jobs` and `latex_core.institution_import_rows` retain job counters, source row numbers, natural keys, normalized payloads, actions, errors, and status. They do not retain uploaded source files or secrets.

`latex_core.programme_template_defaults` maps `students.programme_code` to an immutable template. `latex_core.institution_template_config` contains the global fallback. `latex_core.paper_template_resolutions` records the selected template, dominant programme, method, and whether selection was a manual override. Existing `paper_template_pins` remains authoritative for the immutable Team template pin.

Directory endpoints query these tables with bounded page/limit, search, programme/department, link-status, and identity-type predicates. Programme student counts and template mappings are aggregated in PostgreSQL rather than by loading the institutional directory into the browser.

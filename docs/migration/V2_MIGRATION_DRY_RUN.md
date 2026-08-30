# V2 Migration Dry Run

## Purpose and boundary

The C2 planner answers what later V1-to-V2 migration checkpoints must handle before any live schema or data change is authorized. Its only source is an explicitly supplied PostgreSQL custom dump. There is no default database mode and no environment-based connection discovery.

The planner proves more than dump syntax. After restore it requires the migration ledger, the `latex_core` schema, key users/projects/teams/private-draft/Research Group/template tables, and audit and compile metadata. It then independently computes and reconciles the complete frozen C0 inventory. Any mismatch stops planning with a nonzero exit before substantive proposal files are generated; `reconciliation.tsv` records the mismatch.

## Role decisions

V2 roles are global and exclusive: `writer`, `mentor`, or `admin`. C2 emits suggestions only, and `resolved_v2_role` is always blank.

The decision classes are:

- `AUTO_CANDIDATE`: actual ownership or project history is internally consistent enough to suggest a role, or a legacy Admin has no client-work conflict.
- `MANUAL_REQUIRED`: Writer/Mentor evidence conflicts, Project Manager history needs interpretation, an Admin has client-work history, or available history is otherwise semantically insufficient.
- `BLOCKED_MISSING_ACCOUNT`: no usable credential/account-type row exists. Identity and role are not inferred.
- `BLOCKED_PRIVATE_WORK`: unpublished private work must receive an explicit disposition before role conversion.

An actual Mentor assignment is required for a Mentor suggestion; legacy Professor status alone is insufficient. Project Manager has no V2 equivalent. Admins with personal-paper ownership or Writer/Mentor activity require review because V2 Admin cannot act as a client.

## Papers, teams, and assignments

A personal paper can be preserved under its current workspace identifier only when its owner is a compatible Writer candidate. C2 never selects a transfer target. Incompatible or unresolved ownership is reported for future transfer, Paper Team conversion, archive, or role resolution.

Every legacy team project becomes one Paper Team candidate. A single-project Team keeps the legacy Team name as its preferred proposed name. Each project in a multi-project Team becomes a separate candidate named conceptually `legacy team — legacy project`. These are proposals only; no source names change.

Zero-project Teams are retained in `legacy-teams.tsv` and `unresolved.tsv` for an `ARCHIVE_EMPTY_TEAM` or manual-review decision. C2 does not pick one.

Paper Team membership carries access, not a local role. `team-memberships.tsv` unions broad legacy Team membership with project-role membership for each proposed paper. Because C2 does not finalize roles, assignments remain blocked pending a resolved global role even when a role suggestion is automatic.

## Research Group and private work

Every Research Group is reported with owner, members, workspace, observed file count, and latest database activity. Its action remains `DECISION_REQUIRED`. A future reviewed decision must choose exactly one of:

- `CONVERT_TO_WRITER_PERSONAL`
- `CONVERT_TO_PAPER_TEAM`
- `ARCHIVE_EXPORT`

Every project-user private change set is a hard blocker. The report includes canonical and base revisions, draft revisions, counts, operations, and paths but no draft content or blob hashes. Each remains `UNRESOLVED`; C2 never merges, discards, exports, archives, or rejects it.

## Templates and file policies

Structurally valid legacy template sources are proposed for preservation. A later checkpoint will create immutable `template_versions`; C2 neither versions nor modifies templates.

Policy mapping is based on verified legacy enforcement and provenance:

| Legacy state | Suggested V2 state | C2 classification |
| --- | --- | --- |
| `editable` | `EDITABLE` | Deterministic |
| `read_only` | `CONTENT_READ_ONLY` | Deterministic |
| `managed` with template provenance | `TEMPLATE_MANAGED` | Deterministic |
| `managed` without template provenance | `TEMPLATE_MANAGED` | Manual suggestion only |
| unknown | blank | Manual required |

Legacy non-template `managed` protection relied on the removed Project Manager capability. C2 therefore does not silently convert it to the V2 Admin-only template workflow. `STRUCTURE_LOCKED` and `HIDDEN_SYSTEM` are not suggested without direct legacy semantic evidence.

## Determinism and readiness

All substantive rows are sorted by stable identifiers and paths. Output has no generation timestamp, random container name, runtime path, or other execution-specific value. The dump SHA-256 and frozen commit identifiers are stable inputs. Two runs against the same dump must compare byte-for-byte with `diff -ru`.

`ready_for_destructive_migration` remains `false` while private work, manual role decisions, missing accounts, Research Group decisions, or policy decisions remain. Correctly identifying those blockers is successful C2 behavior, not planner failure. C2 never authorizes destructive migration.

# V2 Domain Model

Status: conceptual contract. This document defines no SQL and authorizes no migration.

## Core invariant

**One Paper Team equals one research-paper workspace.** A Paper Team cannot contain multiple papers or workspaces. Admin creates the Paper Team and its single workspace as one domain operation.

## Entities

| Entity | Responsibility |
| --- | --- |
| `users` | Identity, credentials linkage, enabled state, and one global exclusive role. |
| `personal_papers` | A paper and workspace owned by exactly one Writer. |
| `paper_teams` | Admin-created collaborative paper boundary with exactly one workspace. |
| `paper_team_members` | Writer or Mentor assignment to a Paper Team; assignment grants access, not capabilities outside the global role. |
| `paper_files` | Stable file identity, mutable path, current content reference, and lifecycle state. |
| `file_policies` | Versioned Writer mutation rules for files and structure. |
| `template_versions` | Immutable template releases that may be pinned to papers. |
| `collaboration_updates` | Ordered durable CRDT update tail for a text file and document epoch. |
| `collaboration_snapshots` | Compressed CRDT state used to bound replay. |
| `paper_versions` | Human-visible, immutable whole-paper checkpoints. |
| `review_rounds` | Bounded review cycles over a paper version or state. |
| `review_threads` | Anchored feedback lifecycle and ownership. |
| `review_messages` | Append-preserving conversation within a thread. |
| `review_suggestions` | Mentor-proposed replacement text and Writer decision. |
| `restore_requests` | Governed requests to create a new head from a historical team version. |
| `compile_jobs` | Durable, bounded, idempotent compilation work. |
| `compile_artifacts` | Immutable PDF, log, and SyncTeX output tied to exact source state. |
| `audit_events` | Append-only security and administrative record. |

All public API representations of these concepts must be versionable and serializable.

## Ownership and access

A personal paper's owner must be a Writer. Only its owner may access it through the Writer shell.

A Paper Team is created by an Admin, owns exactly one paper workspace, and has zero or more assigned Writers and zero or more assigned Mentors during setup. Operational policy may require populated assignments before activation.

Membership answers, “Can this person access this paper?” The user's global role answers, “What can this person do there?” Assignment never creates a mixed role. Admin inspection uses the control plane and does not create membership.

## Paper Team status

- `active`: normal Writer collaboration and Mentor review are available.
- `frozen`: source and structural mutation are disabled; authorized reading and administration remain available.
- `submitted`: a submission checkpoint is designated; subsequent mutation follows future submission policy rather than being implied here.
- `archived`: excluded from normal client work lists and read-only except for controlled administration.

State transitions are authorized Admin control-plane operations and are audited.

## Stable file identity

Every paper file has an immutable UUID `file_id`; `path` is mutable. Rename and move change only the path, never `file_id`.

Stable identity is required because:

- annotations must continue to identify a file after rename or move;
- history must distinguish identity-preserving rename from delete-and-create;
- collaboration rooms and update logs must not fork merely because a path changed; and
- file policies must remain attached to the intended file rather than accidentally transfer through path reuse.

Deletion first creates a tombstone or other reversible state under the future implementation policy; it does not immediately destroy referenced blobs.

## Immutable state

Blob content remains addressed by SHA-256. Human-visible versions are immutable canonical manifests referencing file identities and blob or CRDT state. Restore and template update create new heads and preserve previous versions.

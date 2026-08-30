# C3 V2 Domain Foundation

Checkpoint C3 adds migration `0012_v2_domain_foundation.sql` and a persistence-only V2 domain boundary. It does not route HTTP traffic to V2 behavior or migrate V1 data.

## Additive schema

- `global_user_roles` is a one-to-one assignment from canonical user identity to exactly one of `writer`, `mentor`, or `admin`. A missing row is transitional migration state only; every active V2 user must have exactly one row before cutover.
- `personal_papers` gives one Writer ownership of one uniquely linked workspace and records `active`, `frozen`, `submitted`, or `archived` status.
- `paper_teams` gives one Admin-created Paper Team exactly one unique workspace and the same closed status set.
- `paper_team_members` grants paper access to globally assigned Writers and Mentors. It contains no local role or capability field, and Admin inspection is not membership.
- `paper_files` records an immutable UUID `file_id`, mutable `LogicalPath`, revision, and tombstone state. A partial unique index permits only one live instance of a path per workspace; tombstoning releases that path for a new identity without deleting the historical row or BlobStore content.

The repository serializes cross-table role, ownership, membership, and workspace checks with PostgreSQL row locks. Writer ownership blocks transitions to Mentor or Admin. Writer/Mentor membership blocks an Admin transition and is never silently deleted by low-level role assignment.

## Migration boundary

Migration 0012 contains only new tables, indexes, constraints, and comments. It performs no role backfill, paper/team conversion, account-type rewrite, legacy update, delete, truncate, drop, or migration-history rewrite. The 147 legacy users may therefore have no V2 role assignment until the C2 human decisions are resolved. The 73 personal projects, 43 Teams, 41 Team projects, Research Group, and private working state remain V1 data.

## Compatibility and deployment proof

The frozen C0 dump was restored into a disposable PostgreSQL 18.4 container using no network, tmpfs database storage, no product volumes, and read-only dump/migration mounts. Applying only migration 0012 succeeded. All 20 frozen inventory checks matched before and after, including 147 users, 73 personal projects, 43 Teams, 41 Team projects, one Research Group, 13 private change sets, 16 draft files, and 17 structural operations. All 32 pre-existing `latex_core` tables remained; all five C3 tables existed and contained zero rows.

The final application image was deployed only to the API and worker services. SQLx ledger version 12 reports `v2 domain foundation` successful. Every C3 table contains zero live rows; representative V1 counts remain 147 users, 73 projects, 43 Teams, 41 Team projects, one Research Group, and 13 private change sets. Product status and doctor are healthy, the login page returns HTTP 200, and the unauthenticated auth boundary returns HTTP 401 as before.

## Deferred work

- C4: exclusive V2 login routing and separate Writer, Mentor, and Admin shells.
- C5: Admin user/team workflows, intentional multi-step role changes, and template versions.
- C6: durable collaboration persistence and protocol.
- C7: Writer collaboration client integration.

The C2 role, ownership, Research Group, policy, and private-work decisions remain intentionally unresolved.

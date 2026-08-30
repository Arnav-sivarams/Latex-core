# S2 Admin and Writer Workspace

S2 ships V2-only provisioning and a manual-save Writer workspace without changing legacy V1 data, the compiler, or the durable queue.

## Implemented

- Admin APIs and the existing `/admin` shell can create V2 Writer, Mentor, and Admin identities, change their exclusive global role, create one-workspace Paper Teams, and add or remove Writer/Mentor assignments.
- V2 account creation commits credentials and the requested `global_user_roles` row together. The compatibility `account_type` is `student` and never authorizes V2 access. Role changes reuse invariant checks and revoke sessions.
- Writer HTTP APIs list owned personal papers and assigned Paper Teams only. Writers can create initialized personal papers and create, open, save, rename, tombstone, and select Main for stable-ID files.
- New papers contain a valid minimal `main.tex`. Workspace events, `paper_files` metadata, paper rows, and team assignments commit atomically after immutable content is stored through `BlobStore`.
- Active papers are editable. Frozen, submitted, and archived papers are read-only. Mentors and Admins cannot call Writer source mutation APIs.
- `/write` uses a locally built CodeMirror 6 bundle with the standard setup, Stex highlighting through the supported stream-language bridge, nested path display, explicit save states, and Ctrl/Cmd+S.

## Save conflicts

Every HTTP mutation carries the current durable workspace version. PostgreSQL locks and compares that version before appending an event. A stale save returns `409 Conflict`, preserves the newer durable content, and leaves the Writer's local editor text visible with a reload/resolve-required state. There is no last-write-wins behavior.

## S3

S3 will replace HTTP text synchronization with Yjs/Yrs collaboration, awareness, collaboration snapshots, and Writer-scoped undo/recovery. S2 intentionally adds no WebSockets, CRDT updates, live PDF rebuild, Mentor comments, or PDF annotations.

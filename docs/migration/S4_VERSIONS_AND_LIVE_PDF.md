# S4 Versions and Live PDF

S4 adds append-only `paper_versions` records for named `manual_checkpoint` states and submitted `compile_checkpoint` states. Each version points at the existing immutable workspace snapshot and records the paper/workspace, collaboration epoch and durable cutoffs, workspace sequence, main file, ordered file/blob manifest, requesting Writer, and deterministic state hash. The hash is the canonical workspace snapshot identity; no template or policy ID is invented when that provenance does not yet exist.

Every checkpoint and build crosses the same collaboration barrier. The API asks the S3 hub to flush every loaded room in the workspace, waits for its durable update batch and canonical workspace materialization, then calls the existing workspace snapshot service. Unloaded rooms already exist in canonical state. Compilation always consumes that captured snapshot, never mutable in-memory Yjs/Yrs state.

The browser requests an automatic build after roughly two idle seconds following a local or remote durable update. PostgreSQL is authoritative: `v2_paper_build_state` serializes requests per paper, `v2_paper_builds` relates V2 metadata to the existing `compile_jobs` queue, identical successful/active inputs are reused, and an active build permits only one replaceable pending immutable state. Completion enqueues only the newest pending state.

Queue completion updates V2 metadata transactionally. A successful build is promoted only when its exact `state_hash` still equals the paper's desired source hash. Stale successful PDF/log/SyncTeX artifacts remain stored and version-visible but never become current. A failure retains the last promoted PDF and exposes the compiler error while later durable edits may schedule another build.

All V2 jobs enable SyncTeX through the existing frozen M7 `latexmk` runtime. Successful current builds expose authenticated same-paper PDF, log, and non-empty `paper.synctex.gz` concepts through V2 artifact routes. Legacy jobs and artifacts are unchanged.

The Writer shell now provides the current browser-native PDF preview, source/PDF sequence relationship, settling/building/rebuilding/current/failure states, manual Compile with Ctrl/Cmd+Enter, and Sync now with Ctrl/Cmd+S. History lists immutable versions, creates named checkpoints, and compares added/removed/changed files with bounded text diffs. It intentionally provides no Team restore control.

S5 can build Mentor review and PDF.js source/PDF linked annotations on the exact compile checkpoints and persisted SyncTeX artifacts introduced here.

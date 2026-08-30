# Writer guide

Writers use `/write` for personal papers and assigned Team Papers.

## Papers and files

- Create and own personal papers from the Writer paper list.
- Open only Team Papers to which an Admin assigned you.
- Create, rename, delete, and select the main file when the Team lifecycle and current file policy permit it.
- File identity is stable across renames and version history.

The five policies are `EDITABLE`, `CONTENT_READ_ONLY`, `STRUCTURE_LOCKED`, `TEMPLATE_MANAGED`, and `HIDDEN_SYSTEM`. The server enforces them on HTTP and collaboration connections. Hidden system files are omitted from the normal tree.

## Realtime, offline, and saving

Source changes synchronize through durable collaboration rooms. The sync indicator distinguishes pending and durable work. `Ctrl/Cmd+S` requests durable CRDT synchronization; it does not compile.

Offline state is stored per paper epoch. After a governed restoration, changes from the previous epoch are preserved locally and are not merged into the restored paper. Use **Copy recovery text** to retrieve that buffer.

## Build and PDF

After a durable local or remote source change becomes idle for about two seconds, the client requests an automatic exact-state build. The server remains authoritative for state barriers, hash deduplication, and queue coalescing. `Ctrl/Cmd+Enter` runs a manual compile. The last good PDF remains available when a later build fails.

## History and restoration

History contains immutable versions. A Writer may directly restore an owned personal paper after confirmation; the old head is first retained as a safety version and restoration creates a new head.

For a Team Paper, choose **Request restoration**, select a version, optionally enter a reason, and submit. A Team restoration requires the assigned Mentor’s endorsement and an Admin’s decision. Writers cannot directly restore Team Papers.

## Reviews and productivity

Use linked review threads and suggestions from the paper/review panels. Productivity tools include builders, shortcuts, text undo/redo for your own edits, and structural undo/redo subject to the current policy.

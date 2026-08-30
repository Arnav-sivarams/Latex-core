# S3 realtime collaboration

S3 moves normal text editing from HTTP `PUT` to an authenticated binary WebSocket at
`/api/v2/collab/{paper_id}/files/{file_id}`. The opaque same-origin session cookie is
validated during upgrade and again before every source update. Rooms are keyed by
`(workspace_id, file_id)`; rename therefore retains room identity.

The browser uses Yjs, `y-codemirror.next`, and a Writer-local `Y.UndoManager`. The
server uses Yrs with the shared text name `source`. Remote updates have an untracked
origin, so Ctrl/Cmd+Z and redo affect only that Writer's local text transactions.

## Wire contract

Client binary frames use `0x01 || client_seq:u64-be || Yjs-v1-update` for
`SOURCE_UPDATE` and `0x02` for `FLUSH`. Server binary frames use
`0x10 || Yjs-v1-update` for `INITIAL_STATE` and `0x11 || Yjs-v1-update` for
`REMOTE_SOURCE_UPDATE`. Text JSON controls are `JOIN_ACCEPTED`, `DURABLE_ACK`,
`REMOTE_DURABLE`, `FLUSHED`, `ERROR`, and `RELOAD_REQUIRED`.

`DURABLE_ACK` means the attributed update batch and one canonical materialization of
its final Y.Text value committed together in PostgreSQL. The room broadcasts edits
promptly, batches for about 50 ms or 64 KiB, and serializes materialization with the
workspace head/file locks so it cannot silently overwrite a structural change.

Each workspace lazily receives one `document_epoch`. First collaboration persists a
compressed sequence-zero Yrs snapshot of canonical BlobStore content. Recovery loads
the latest compressed snapshot and then ordered update rows. Further snapshots occur
after about 500 updates, 1 MiB, or the last client disconnect; S3 does not delete the
update history.

`y-indexeddb` stores browser CRDT state under a key containing user, workspace, stable
file UUID, and epoch. Offline edits remain editable and locally durable. Reconnect
applies server state with a remote origin, merges it with IndexedDB state, and submits
the merged Yjs state for durable acknowledgement. The UI reports Local, Syncing,
Synced, Offline, and Reconnecting without an HTTP overwrite fallback.

Assigned Writers have read/write access to active Team papers; owners have read/write
access to active personal papers. Assigned Mentors may subscribe read-only to Team
papers. Admins, unassigned users, and Mentors on personal papers are denied. Frozen,
submitted, or archived papers are read-only. Membership, session, role, file liveness,
paper status, workspace identity, and epoch are revalidated before every mutation.

New File, Rename, Delete, and Set Main remain structural HTTP operations. The Writer
flushes the current room before Rename, Delete, or Set Main. Delete broadcasts
`RELOAD_REQUIRED`. Structural undo is deliberately deferred. S4 versions/checkpoints
and its exact-state compile barrier will consume the durable sequence and canonical
materialization foundation established here.

# ADR-001: Real-Time Collaboration

## Status

Accepted for V2.

## Decision

The Writer editor is CodeMirror 6. Browser collaborative state uses Yjs; Rust collaborative state uses Yrs. An authenticated binary WebSocket carries collaboration traffic. PostgreSQL stores an ordered update log and compressed CRDT snapshots.

Each text file has one room: `paper:{paper_id}:file:{file_id}`. Stable file identity prevents rename or move from changing the room.

For a source update, the gateway must:

1. receive the binary update;
2. validate the authenticated session;
3. validate the global Writer role;
4. validate paper membership;
5. validate Paper Team status and file policy;
6. apply the update to the in-memory Yrs document;
7. broadcast the accepted update;
8. batch durable PostgreSQL persistence; and
9. acknowledge “Synced” only after the batch is durable.

The system may separately acknowledge receipt for protocol flow control, but the UI must not confuse receipt with durability. A persistence failure leaves the update unacknowledged as durable and triggers bounded recovery/retry behavior.

A Mentor receives live document updates after authorization but has no permitted source-update message. An Admin never joins ordinary collaboration rooms.

## Rejected alternatives

- Timestamp last-write-wins silently loses concurrent work and cannot preserve author intent.
- Polling adds latency and does not provide an appropriate ordered collaborative document model.
- Private drafts plus Publish split the canonical paper and prevent natural live collaboration.
- A whole-paper monolithic room couples unrelated files, enlarges replay, and makes file-scoped authorization and recovery harder.
- Compile per keystroke couples the hot source path to expensive work and creates an unbounded build stream.

## Consequences

Collaboration persistence becomes a first-class durable layer. CRDT state does not replace immutable paper versions, audit events, or blob-backed compile manifests. C1 selects these technologies but does not add them to product code.

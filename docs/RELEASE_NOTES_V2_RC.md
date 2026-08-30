# Professor V2 release candidate

LaTeX Core V2 is ready for final manual browser acceptance after automated qualification.

## Included

- Exclusive Writer, Mentor, and Admin products with server-controlled login
- Durable realtime collaboration, epoch-scoped offline recovery, and stable file UUIDs
- Restored two-second idle auto-build plus manual compile and live/last-good PDF
- Immutable history, review rounds, PDF.js, SyncTeX-linked review, and productivity builders
- Governed Team restoration with Mentor endorsement, Admin application, safety checkpoint, new-head semantics, and client epoch reload
- Direct safe restoration for Writer-owned personal papers
- Server-authoritative file policies across HTTP, WebSocket, and structural mutations
- Admin Paper Team lifecycle, template pinning, versions, restoration, reviews, queue, audit, and read-only system health
- Additive migration 0017 and retained deterministic C2 legacy migration planner

## Known limitations

- This RC has no formal 1,000-user qualification. Its recorded concurrency result is only a bounded 12-client smoke.
- Unresolved legacy-data migration decisions require human resolution and are not automatically applied.
- Host-level operational actions remain CLI-only in this release candidate.
- Templates can be immutably selected and pinned when a Team is created. Conflict-safe updates to existing Teams are explicitly unavailable in this RC.

## Next

Run the short Writer → Mentor → Admin browser acceptance on the pristine live cluster. If it passes, create the final tag and package seal without moving an existing tag.

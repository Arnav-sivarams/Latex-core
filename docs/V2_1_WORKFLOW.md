# V2.1 Team workflow

V2.1 keeps the global roles `WRITER`, `MENTOR`, and `ADMIN`. A Paper Team additionally designates exactly one assigned Writer as its Team Leader; this is a Team-scoped capability, not a fourth role. Admin creates the Team, assigns members, and transactionally selects or changes the Leader.

## Writer workflow

The Writer workspace is a viewport-locked three-pane surface: Papers/Files, source, and PDF scroll independently below a compact application bar and action toolbar. **Save** (`Ctrl/Cmd+S`) durably flushes the current CRDT state. It does not compile or create a version. **Compile** (`Ctrl/Cmd+Enter`) remains an explicit build action.

All Team Writers can edit, Save, Compile, view PDF/history, and request a revert. The Team Leader additionally creates named checkpoints, handles revert requests, performs confirmed append-only safe reverts, and opens or closes review. Team reverts retain `PRE_RESTORE_SAFETY`, advance the document epoch, and create a new head; Mentor and Admin are not ordinary revert approvers.

## Review workflow

**Send for Review** is visible only to the Team Leader and requires a successful PDF whose state hash exactly matches durable current source. It records an immutable source/version/build baseline and opens the review gate. Closing review disables new annotations without deleting previous rounds or feedback.

Mentors can always read assigned live source and PDF. While the gate is open, a non-empty source selection or PDF region can be right-clicked to open one compact Comment/Suggest popover. New feedback defaults legacy metadata to `NOTE` and `WRITING` without asking for severity, category, assignee, or due date. Source anchors remain Yjs RelativePositions; PDF anchors remain normalized rectangles with truthful SyncTeX mapping.

Active Mentor source comments appear as CodeMirror highlights for Writers. Hovering shows Mentor identity and feedback without moving the workspace; **Done** resolves and hides the active highlight while preserving history. Safe suggestions also expose **Apply**. Comments, resolved history, versions, revert controls, and problems live in temporary toolbar drawers rather than permanent panels.

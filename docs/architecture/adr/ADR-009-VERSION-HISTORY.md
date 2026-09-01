# ADR-009: Version History

## Status

Accepted for V2.

## Four layers

1. The CRDT update log provides ordered durable collaborative operations within a document epoch.
2. Periodic compressed snapshots bound CRDT recovery and replay.
3. Human-visible immutable paper versions capture named or system-significant whole-paper state.
4. The audit log records security-relevant, administrative, and governance actions and their authenticated actors.

These layers reference one another where useful but are not interchangeable. CRDT compaction must not erase human-visible versions or audit history.

## Version types

- `MANUAL_CHECKPOINT`
- `COMPILE_CHECKPOINT`
- `REVIEW_ROUND`
- `PRE_RESTORE_SAFETY`
- `TEAM_REVERT` (new V2.1 heads; historical `ADMIN_RESTORATION` rows remain readable)
- `TEMPLATE_UPDATE`
- `SUBMISSION`

Every paper version identifies the exact epoch, collaboration cutoff, manifest revision, stable file identities and content state, main file, template version, and policy version required to materialize it.

History is append-preserving. Restore always creates a new head; it never rewinds by deleting or reclassifying intervening history.

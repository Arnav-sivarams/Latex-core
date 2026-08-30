# ADR-003: Undo Versus Historical Restore

## Status

Accepted for V2.

## Undo

Undo and redo are normal Writer operations for the Writer's own recent work. They are collaborative inverse edits, immediately visible to every participant, and power Ctrl+Z/Ctrl+Y. They are not historical restoration and require neither Mentor nor Admin approval.

Every collaborative transaction records an origin containing `writer_user_id`, `session_id`, `file_id`, and `transaction_group`. The Writer-scoped undo manager operates on that origin and current CRDT structure. Writer A's undo must not erase Writer B's edits. If an exact inverse is no longer safe, the operation fails visibly or produces the CRDT-defined scoped inverse; it never falls back to whole-document replacement.

## Structural undo

Writer operations `create`, `rename`, `move`, `delete`, and `Set Main` are represented conceptually with operation ID, actor, forward payload, inverse payload, manifest revision, reversible state, and timestamps. A Writer may reverse only their own recent operation when current manifest state and policy still permit the inverse.

Delete initially creates a tombstone or reversible state according to the future implementation policy rather than causing immediate destructive blob loss. Structural undo creates an audited forward state transition; it does not erase the original event.

## Restore

Restore is a historical, whole-paper state transition. A Writer cannot directly restore a team paper. A team restore follows the governance in [ADR-004](ADR-004-TEAM-RESTORATION.md), creates a new head, and preserves all history. The personal-paper owner exception is defined in the role model.

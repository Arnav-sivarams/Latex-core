# V2 Compile State Machine

## States

```text
IDLE -- explicit Compile/durability barrier --> QUEUED --> RUNNING
  ^                                                     | success, head matches
  |                                                     +--> SUCCEEDED_CURRENT --> IDLE
  |                                                     | success, head differs
  +-----------------------------------------------------+--> SUCCEEDED_STALE -- retain
                                                        | failure
                                                        +--> FAILED --> IDLE
```

`SUCCEEDED_CURRENT` and `SUCCEEDED_STALE` are artifact classification outcomes, not mutable source states.

## Manual admission and duplicate protection

Only an authorized, explicit Compile request crosses the durability barrier and admits work. Editing, autosave, collaboration, metadata resolution, file operations, restore, opening, and reconnecting never enqueue a build. Queue caps, immutable identities, idempotency keys, and duplicate-click coalescing still apply.

Existing queued and running work is retained. A legacy pending automatic candidate is discarded when the active job completes; it is not converted into a new manual request.

## Exact-state identity

Each job carries a state hash over its compile manifest. A duplicate state hash may reuse a verified artifact or coalesce with existing work, subject to authorization and compiler-environment identity. It must not create redundant intermediate work.

A compile barrier first flushes collaboration updates durably and then captures paper ID, epoch, cutoff sequence, manifest revision, file state vectors or hashes, main file ID, template version, file-policy version, state hash, requester, and trigger.

## Source changes and undo

An authorized manual compile flushes accepted collaboration updates, then captures the exact durable state. Manual requests remain bounded and idempotent; they do not cause unbounded parallel work.

Undo or any later mutation does not rewrite or replace a running job. Its result may be retained as historical/stale. The user must explicitly Compile again for the new state.

## Artifact promotion

If H1 completes while H2 is current, retain H1 historically but never present it as current. Show the last-good PDF with a stale notice until an explicit build of H2 succeeds. Failure never replaces the latest valid PDF. Outputs are immutable PDF, log, and SyncTeX artifacts tied to the exact manifest.

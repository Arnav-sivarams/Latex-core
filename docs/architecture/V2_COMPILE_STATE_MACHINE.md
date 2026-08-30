# V2 Compile State Machine

## States

```text
IDLE -- edit --> DEBOUNCING -- deadline/barrier --> QUEUED --> RUNNING
  ^                  | edit: restart debounce          |       | success, head matches
  |                  | undo: recompute/cancel          |       +--> SUCCEEDED_CURRENT --> IDLE
  |                  v                                 |       | success, head differs
  +-------------- obsolete                             |       +--> SUCCEEDED_STALE -- retain --> IDLE/debounce
                                                     failure
                                                       +-----> FAILED --> IDLE/debounce
```

`SUCCEEDED_CURRENT` and `SUCCEEDED_STALE` are artifact classification outcomes, not mutable source states.

## Coalescing

Automatic compilation begins at approximately two seconds of source inactivity. This is a starting parameter, not a product guarantee. Per Paper Team there is at most one automatic build running and one newer automatic build pending. Edits during `DEBOUNCING` restart the debounce and replace the candidate. Edits while `QUEUED` replace or supersede the queued candidate when safe. Edits while `RUNNING` replace the single pending candidate with the latest exact state.

For `H1 -> H2 -> H3 -> H4`, if H1 is running, H1 remains running and only H4 is pending. Intermediate automatic builds are not queued.

## Exact-state identity

Each job carries a state hash over its compile manifest. A duplicate state hash may reuse a verified artifact or coalesce with existing work, subject to authorization and compiler-environment identity. It must not create redundant intermediate work.

A compile barrier first flushes collaboration updates durably and then captures paper ID, epoch, cutoff sequence, manifest revision, file state vectors or hashes, main file ID, template version, file-policy version, state hash, requester, and trigger.

## Manual compile and undo

An authorized manual compile bypasses the idle wait but still uses the durability barrier and exact-state manifest. Manual requests remain bounded and idempotent; they do not cause unbounded parallel work.

Undo during debounce invalidates the obsolete candidate and recomputes from current state. Undo while a build runs does not rewrite that job; its result may be retained as historical/stale, while the latest state becomes the pending candidate.

## Artifact promotion

If H1 completes while H2 is current, retain H1 historically but never promote it as current. Schedule or use H2, and continue showing the latest valid current PDF until H2 succeeds. Failure never replaces the latest valid PDF. Outputs are immutable PDF, log, and SyncTeX artifacts tied to the exact manifest.

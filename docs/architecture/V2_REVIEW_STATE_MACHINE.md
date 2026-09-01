# V2 Review State Machine

## Aggregate

A review round scopes review threads to an immutable paper baseline. It records the exact version, successful build/PDF, source state hash, submitting Team Leader, and submission time. A thread contains append-preserving messages and may reference one source anchor, one PDF anchor, or both. A suggestion records the Mentor proposal plus the Writer's decision and resulting Writer-attributed transaction when accepted.

The primary V2.1 review types are `COMMENT` and `SUGGESTION`. New rows use compatibility defaults `NOTE`, `WRITING`, no assigned Writer, and no due date. Historical structured review rows remain readable.

## Review-round lifecycle

```text
no open review -- Team Leader sends exact current build --> OPEN_FOR_REVIEW
OPEN_FOR_REVIEW -- Team Leader ends review -------------> CLOSED
```

Sending for review requires durable source and a successful PDF whose state hash exactly matches current desired source. Opening binds that immutable baseline and enables Mentor annotation. Closing disables new Mentor annotations and preserves all prior rounds, threads, anchors, and messages. A later submission creates a new baseline.

## Thread lifecycle

```text
OPEN -- Writer chooses Done --> RESOLVED
```

Done hides the active highlight but retains the review row historically. Legacy lifecycle states and messages remain readable.

## Suggested replacement

```text
PROPOSED -- Writer accepts ----------> ACCEPTED
    |                                    |
    | Writer edits, then accepts          +-- source transaction attributed to Writer
    +----------------------------------> ACCEPTED_EDITED
    |
    +-- Writer rejects with reason ----> REJECTED
```

The Mentor proposal never mutates source. Acceptance is validated as a current Writer action and becomes a Writer-authored CRDT transaction.

## Anchors

A source anchor uses stable `file_id`, CRDT-relative start/end, quoted text, context hash, and creation sequence. A PDF anchor uses immutable artifact, page, normalized geometry, and mapping metadata. Mapping can be exact, approximate, PDF-only, source changed, or source deleted/re-anchor required.

## Approvals

Section approval belongs to a review round and a stable section/source reference. A Mentor may approve or revoke/reopen according to audited review policy. Paper approval records the reviewed version/state and does not freeze or submit the Paper Team by itself. Review-round approval summarizes the round; paper approval is an explicit separate review decision.

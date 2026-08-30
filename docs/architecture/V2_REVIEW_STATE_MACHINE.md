# V2 Review State Machine

## Aggregate

A review round scopes review threads to a paper state and records round status, reviewers, version comparisons, section approvals, and paper approval. A thread contains append-preserving messages and may reference one source anchor, one PDF anchor, or both. A suggested replacement is attached to a thread and records the Mentor proposal plus the Writer's decision and resulting Writer-attributed transaction when accepted.

Review types are Comment, Question, Change Request, Suggested Replacement, Section Approval, and Paper Approval. Severity is `note`, `minor`, `major`, or `blocking`. Categories are `writing`, `methodology`, `evidence`, `citation`, `formatting`, `figure`, `table`, `equation`, and `submission_requirement`.

## Review-round lifecycle

```text
DRAFT -- Mentor opens --> OPEN -- Mentor approves round --> APPROVED
                            |
                            `-- superseded/administratively closed --> CLOSED
```

Opening a round binds its comparison baseline and creates the scope in which threads, messages, suggestions, section approvals, and paper approval are recorded. `APPROVED` is a review conclusion, not permission to mutate source or an automatic Paper Team status transition. Closing or superseding a round preserves its contents.

## Thread lifecycle

```text
OPEN -- Writer marks addressed --> ADDRESSED -- Mentor resolves --> RESOLVED
                                     |                              |
                                     | Mentor reopens               | Mentor reopens
                                     v                              |
                                  REOPENED <-------------------------+
                                     |
                                     `-- Writer marks addressed --> ADDRESSED
```

`REOPENED` behaves as actionable Mentor feedback: the Writer may mark it `ADDRESSED`, after which the Mentor may resolve or reopen it. Only a Mentor may resolve Mentor feedback. Messages remain available across lifecycle transitions.

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

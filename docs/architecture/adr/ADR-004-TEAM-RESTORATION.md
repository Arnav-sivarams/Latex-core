# ADR-004: Team Restoration Governance

## Status

Accepted for V2.

## State machine

```text
DRAFT
  |
  | Writer submits
  v
AWAITING_MENTOR_REVIEW
  |-- Mentor rejects -----------------> MENTOR_REJECTED
  `-- Mentor endorses ----------------> AWAITING_ADMIN_REVIEW
                                          |-- Admin rejects --> ADMIN_REJECTED
                                          `-- Admin applies --> APPLIED
```

Terminal rejection records remain auditable. Resubmission, if supported, creates a new request or explicit append-preserving transition rather than rewriting the decision.

The Writer selects a target version, provides a reason, submits the request, and views its diff and status. The Mentor compares current and target state, comments, rejects, or endorses to Admin. The Admin cannot apply before Mentor endorsement; after endorsement the Admin independently compares and either rejects or applies.

## Apply algorithm

Applying an endorsed request performs one controlled transition:

1. acquire a short restoration lock;
2. notify active clients;
3. flush pending durable collaboration updates;
4. create a `PRE_RESTORE_SAFETY` checkpoint;
5. materialize the target version;
6. increment `document_epoch`;
7. create a new head from the historical content;
8. retain all prior history;
9. release the lock;
10. broadcast `PAPER_EPOCH_CHANGED`; and
11. require active clients to reload the new epoch.

The lock prevents source or structural mutations from crossing the epoch boundary. Authorization and governance state are revalidated inside the apply transaction.

If current state is V42 and target state is V36, the result is V43 containing V36's historical state. V36 never becomes current by deletion or concealment of V37-V42.

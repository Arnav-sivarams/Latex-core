# ADR-004: Team Restoration Governance

## Status

Superseded by the V2.1 Team Leader workflow.

## State machine

```text
REQUESTED
  |-- Team Leader rejects ------------> LEADER_REJECTED
  `-- Team Leader safely applies -----> APPLIED
```

Terminal rejection records remain auditable. Resubmission, if supported, creates a new request or explicit append-preserving transition rather than rewriting the decision.

A regular Team Writer selects a target version, optionally provides a reason, and creates a request. The Team Leader compares current and target state and rejects or applies it. A Leader may also apply a direct revert after explicit confirmation. Mentor and Admin ordinary endpoints are deprecated and cannot authorize or apply Team reverts. Historical requests and their legacy statuses remain auditable.

## Apply algorithm

Applying an authorized request performs one controlled transition:

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

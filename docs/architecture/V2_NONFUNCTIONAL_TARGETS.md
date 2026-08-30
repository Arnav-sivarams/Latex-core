# V2 Nonfunctional Targets

Status: targets to test, not current performance claims or contractual service levels. Measurements use representative files, the intended local/campus network, and the initial capacity target of at most 1,000 concurrent users.

## Latency targets

| Interaction | Initial target |
| --- | --- |
| Local keystroke response | Within one animation frame |
| Remote source propagation | p95 under 250 ms |
| Durable sync acknowledgement | p95 under 500 ms |
| Review event propagation | p95 under 500 ms |
| Structural operation propagation | p95 under 300 ms |
| Warm room open | Under 300 ms |
| Snapshot plus update-tail load | Under 1 second for ordinary files |
| Automatic build debounce | Approximately 2 seconds |

Durable acknowledgement is measured through committed PostgreSQL persistence, not merely server receipt. Warm-room and recovery measurements exclude TeX compilation.

## Reliability and security properties

Tests must demonstrate:

- no lost acknowledged update;
- no silent last-write-wins overwrite;
- no stale PDF promotion;
- no cross-team source, review, presence, or artifact leakage;
- no Mentor source mutation;
- no Admin source mutation;
- bounded recovery after collaboration-node restart;
- deterministic snapshot-plus-tail reconstruction;
- bounded memory and outbound queues under slow-client pressure; and
- session/capability revocation after role, membership, status, or policy change.

“Bounded recovery” means a measured, documented bound for representative room sizes and update tails. Exact thresholds and load profiles are set in the implementation checkpoint and must be validated before making an operational claim.

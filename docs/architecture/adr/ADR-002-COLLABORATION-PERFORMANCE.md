# ADR-002: Collaboration Performance

## Status

Accepted for V2.

## Decision

The hot source-update path must not wait on compilation, history rendering, Admin reporting, or PDF annotation reprojection.

Each active text file is owned by an in-memory room actor. The actor applies Yrs updates serially, uses bounded outbound queues, and sends accepted updates to batched database persistence. Room recovery loads a compressed snapshot plus its ordered update tail. Compilation runs on a separate durable background path.

Initial persistence batch triggers are approximately:

- 50 milliseconds;
- 32-64 KB of updates;
- a compile barrier; or
- the last Writer disconnect.

Whichever trigger occurs first flushes the batch. These values are starting engineering parameters, not hard product promises. Disconnect flush has a bounded server-side completion path; a client sees “Synced” only after durability.

Snapshot creation is triggered conceptually by a bounded update count, bounded update bytes, inactivity, or a successful version checkpoint. Exact thresholds belong to performance testing.

## Priority and backpressure

Priority is:

1. source updates;
2. structural operations;
3. review messages;
4. compile notifications;
5. presence.

Presence may be coalesced or dropped under pressure. Source updates may not be dropped. A slow client with a full bounded queue is disconnected and must resynchronize; the server does not permit unbounded memory growth.

## Scaling boundary

The V2 MVP needs no Redis or NATS. A future multi-node design may use consistent ownership or sharding by `file_id`, with one authoritative room actor at a time. Cross-node routing and coordination are deferred until measured need exists.

# LaTeX Core V2 RC test report

Date: 2026-08-30

## Qualification scope

This report covers the final governance, policy, restoration, auto-build, security, migration, and bounded resilience qualification. It does not claim a 1,000-user load certification and does not rerun the historical compiler corpus.

## Automated gates

- Rust formatting, workspace check, tests, and Clippy with warnings denied
- Frontend clean install, production build, and Node tests
- Non-bundled JavaScript syntax checks and `git diff --check`
- Disposable PostgreSQL integration tests for restoration, all five policies, immutable template pinning, lifecycle enforcement, and role boundaries
- Existing scheduler tests for state-hash deduplication, one-active/newest-pending coalescing, and stale-artifact protection

## Focused results

- Auto-build: 2,000 ms idle debounce after durable local or remote updates; newer durable activity resets the timer; manual compile remains independent.
- Governed restoration: Writer request and Mentor/Admin decisions; `PRE_RESTORE_SAFETY`; append-only restored head; epoch increment; stale-epoch rejection and reload notification; direct personal restoration limited to the owner.
- Policies: `EDITABLE`, `CONTENT_READ_ONLY`, `STRUCTURE_LOCKED`, `TEMPLATE_MANAGED`, and `HIDDEN_SYSTEM` covered through HTTP, WebSocket, direct fetch/listing, and structural undo paths.
- Bounded concurrency smoke: passed at 12 simulated WebSocket clients across a Team Paper file in 2.19 seconds, with convergence and exact durable canonical state. This result is intentionally not extrapolated.
- Collaboration restart recovery and last-good PDF behavior remain covered by the existing workspace/collaboration and compile scheduler suites.

## Migration evidence

- Fresh disposable database: migrations 001 through 017 applied successfully from empty state; the migration ledger reports 17 successful migrations and the governance tables are present.
- C2 planner: the frozen C0 backup was processed twice in isolated, network-disabled disposable PostgreSQL runs. Outputs were byte-for-byte deterministic, the active database was not accessed, and unresolved human decisions remained explicit.

## Compiler environment

M7 remained unchanged and verified. Compiler digest:

`sha256:8db804f76b8e80e5be9fb28ba14b0938df5989b7a8250ca6b0e9f3c200c4ee38`

## Known limitations

- No formal 1,000-user qualification.
- Unresolved legacy-data migration decisions are not automated.
- Host-level operational actions remain CLI-only.
- Existing-Team template update is unavailable in this RC; immutable initial pinning is supported.

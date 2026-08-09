# Repository Engineering Rules

## Scope

This repository implements a production-grade server-side LaTeX core platform in Rust. The current target is at most 1,000 concurrent users with manual compilation only. Do not design for one million users or introduce unnecessary distributed infrastructure. No frontend UI is in scope.

## Architecture

- Rust is the primary application language.
- Use Tokio and Axum for asynchronous server functionality when that milestone begins.
- PostgreSQL is the source of truth for structured persistent state and the durable compilation queue.
- Store project file content through a `BlobStore` abstraction addressed by SHA-256. Development initially uses a filesystem backend; add an S3-compatible backend later.
- Project snapshots are immutable canonical manifests referencing blob hashes.
- Workspace edits must be durable before the server reports them as durably saved.
- Use Tree-sitter LaTeX as the parser syntax layer and a project-owned Rust semantic layer above it.
- Compile using TeX Live and `latexmk`; never implement a TeX engine.
- Compilation is manual only. Saving and parsing must never trigger compilation.
- Compilation jobs must be durable, bounded, and idempotent.
- Workers claim PostgreSQL jobs transactionally with row locking and `SKIP LOCKED`.
- Never execute user LaTeX or extensions inside the API or parser process.
- Put compiler execution behind a `Sandbox` abstraction.
- Marketplace extensions will eventually execute through Wasmtime with explicit capabilities.
- Do not introduce Phoenix, Redis, Kafka, NATS, MLIR, Kubernetes, or a JavaScript server unless a later requirement specifically needs one.
- Every public API type must be versionable and serializable.
- Avoid unsafe Rust. Any unsafe block requires written justification.

## Engineering discipline

- Inspect existing files before changing anything.
- Never rewrite unrelated working code.
- Keep modules small and responsibilities explicit.
- Use deterministic tests.
- Never silently ignore errors; use structured error enums.
- Add structured tracing around persistence, compilation, and job transitions when those features are implemented.
- Every milestone must include unit tests and relevant integration tests.
- Run `cargo fmt --check`, Clippy with warnings denied where practical, `cargo test --workspace`, and `git diff --check` before declaring a milestone complete.
- Do not commit automatically.
- At the end of each task report exactly: files changed, architecture decisions, tests run, tests passed/failed, remaining issues.


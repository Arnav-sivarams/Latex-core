# Purpose

LaTeX Core is a portable, production-grade, server-side platform core for durable LaTeX projects and manual compilation. This document freezes the direction that current and future milestones must preserve.

# Scope

Milestone 1 implements only synchronous domain types, validated identifiers and logical paths, immutable canonical manifests, deterministic SHA-256 identities and compile keys, artifact contracts, tests, and this document. Persistence, parsing, compilation, networking, and extension execution described below are future work.

# Non-Goals

- No frontend.
- No automatic compilation.
- No custom TeX engine.
- No LuaLaTeX modification.
- No unnecessary microservices.
- No million-user architecture in V1.

# Server-First Principle

Persistence, snapshotting, hashing, parsing, semantic analysis, dependency and language intelligence, scheduling, compilation, caching, artifacts, extensions, and security belong on the server. Clients remain thin.

# Manual Compilation Principle

Only an explicit manual request may schedule compilation. Saving, typing, persistence, parsing, and analysis must never trigger compilation.

# Persistence Model

PostgreSQL will be the source of truth for structured state and the durable compilation queue. Workspace changes will use append-only durable events with periodic immutable snapshots, and durable-save acknowledgements will follow durable persistence.

# Content-Addressed Storage

Project content will use a SHA-256-addressed `BlobStore` abstraction. Development will use a filesystem backend and production will later support S3-compatible storage. Immutable canonical JSON manifests reference blobs and contain no machine- or user-specific identity.

# LaTeX Parsing Strategy

Future parsing will combine Tree-sitter LaTeX as the syntax layer with a project-owned Rust semantic layer. Parsing will not execute user content or start compilation.

# TeX Compatibility Strategy

The system will eventually use full TeX Live plus latexmk and support pdfLaTeX, LuaLaTeX, XeLaTeX, standard packages/templates/fonts and project-local classes/styles/assets. It will use existing TeX engines rather than implementing or modifying one.

# Compilation Queue Strategy

Manual compile requests will enter a durable, bounded, idempotent PostgreSQL queue. Workers will claim jobs transactionally using row locking and `SELECT ... FOR UPDATE SKIP LOCKED`, enabling retry and crash recovery.

# Worker Model

A bounded, long-lived worker pool will execute jobs independently of API and parser processes. Cost classes, fairness, cache reuse, and future horizontal scaling will prevent request bursts from becoming unbounded compiler processes.

# Security Model

Compiler execution will sit behind a `Sandbox` abstraction. Production will use strong container/gVisor-class isolation. User LaTeX and tools will never execute in API or parser processes; stronger compatibility remains explicitly approved and strongly sandboxed.

# Extension Model

Marketplace extensions will eventually run through Wasmtime/WebAssembly and explicit permissions rather than arbitrary native server execution. Milestone 1 implements no extension runtime.

# Integration Model

Future websites/apps should integrate through a versioned REST API and optional thin SDK, without embedding compiler internals into the host application.

# Deployment Model

The modular Rust application will initially be portable through Docker Compose. Kubernetes is future-only and appropriate only when operationally required; the architecture is not decomposed into network microservices.

# 1,000-User Scalability Target

1,000 concurrent users does not mean 1,000 simultaneous compiler processes. The system will use a durable queue and bounded worker pool. Durable burst handling, exact retry, crash recovery, content deduplication, user fairness, cost classification, cache reuse, and later horizontal scaling remain required.

# Architecture Decisions That Are Currently Frozen

Rust, future Tokio/Axum, PostgreSQL structured state and queue, SHA-256 `BlobStore`, immutable canonical snapshots, Tree-sitter plus Rust semantics, full TeX Live and latexmk, manual-only compilation, bounded workers, sandboxed compiler execution, Wasmtime extensions with capabilities, versioned REST and thin SDKs, Docker Compose initially, and Kubernetes only when needed are frozen. Phoenix, Elixir/Erlang, NATS, Kafka, required Redis, MLIR, custom TeX engines, modified LuaLaTeX, non-Rust backends, automatic compilation, frontend frameworks, and premature microservices are excluded unless a future versioned decision explicitly changes this contract.

# LaTeX Core

Server-side infrastructure for durable LaTeX project storage, parsing, indexing, and manual compilation.

The repository is currently at Milestone 0: a compileable Rust workspace foundation. Its crates define architectural boundaries only; application features have not been implemented.

## Requirements

- Stable Rust with `rustfmt` and Clippy
- Git (used by the validation script)

The pinned stable toolchain and required components are described in `rust-toolchain.toml`.

## Workspace

Reusable components live under `crates/`. Executables live under `services/`.

Run all repository checks with:

```sh
./scripts/validate.sh
```

The script resolves the repository root itself and may be invoked from any working directory.


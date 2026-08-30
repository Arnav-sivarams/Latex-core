# V2 Migration Planning

Checkpoint C2 provides a deterministic, read-only planner for the frozen C0 PostgreSQL custom dump. It restores only into a disposable PostgreSQL 18.4 container with no network, host port, or persistent volume, validates the restored prototype, reconciles every frozen baseline count, and writes private migration metadata to an external directory.

C2 does not migrate data. It does not connect to or modify the running product, choose roles or transfer targets, resolve private drafts, dispose of Research Groups, change policies, or create a V2 schema.

## Safe execution

Use a new or empty output directory outside the repository:

```bash
./scripts/migration/v2-plan.sh \
  --dump "$HOME/Projects/Backups/latex-core/c0-prototype-.../database.dump" \
  --output /tmp/v2-plan
```

The dump path and output path must be absolute. The dump is mounted read-only. The output directory must be writable and empty, and the repository root is explicitly refused as output. Docker must be available; the script uses `postgres:18.4`, matching the frozen product database major version.

The planner prints its safety mode at startup. A shell trap removes the uniquely named temporary container on success, failure, or interruption. PostgreSQL data lives in a container tmpfs and the container has no network. The output contains IDs and emails as private planning metadata, but never passwords, credential hashes, sessions, cookies, blob hashes, or source-file contents. Do not commit planner output.

`--strict-ready` is an optional CI-style gate. It still writes the complete plan, then exits nonzero while required human decisions remain. That non-readiness is expected at C2.

## Output

The output directory contains:

- `migration-plan.json`: machine-readable counts and readiness.
- `summary.txt` and `README.txt`: concise operator guidance.
- `reconciliation.tsv`: all 20 frozen C0 expectations and independently calculated values.
- `users.tsv` and `user-role-decisions.tsv`: safe identity metadata, signals, suggestions, and blank final-role fields.
- `personal-papers.tsv`: owner compatibility and future action candidates.
- `paper-team-plan.tsv`, `legacy-teams.tsv`, and `team-memberships.tsv`: one-paper-per-team proposals and access relationships.
- `research-groups.tsv`: the required unresolved disposition.
- `private-work.tsv`: paths and revision metadata only; every change set remains unresolved.
- `templates.tsv` and `file-policy-plan.tsv`: preservation and policy mapping proposals.
- `unresolved.tsv`: every decision or blocker that future checkpoints must address.

See [V2 Migration Dry Run](V2_MIGRATION_DRY_RUN.md) for planner behavior and [V2 Decision Manifest](V2_DECISION_MANIFEST.md) for the future reviewed-decision contract.

# Backup and restore

LaTeX Core durable state has two inseparable parts: PostgreSQL contains identities, manifests, queue state, reviews, and metadata; the BlobStore volume contains immutable content addressed by SHA-256. Back up both at the same operational point.

## Backup

With the stack running:

```sh
./latex-core backup /absolute/backup/directory
```

The supported wrapper writes `database.sql` with `pg_dump` and `blobs.tar.gz` from the BlobStore volume. Store the directory outside the Git checkout. Verify both files exist, are non-zero, and are protected from unauthorized access.

## Restore

Restore into a matching release only after separately backing up the target. The repository helper expects a running stack and both backup files:

```sh
./scripts/restore-release.sh /absolute/backup/directory
./latex-core restart
./latex-core doctor
```

Restoring replaces or conflicts with target application state and must be scheduled as a maintenance operation. Never restore untrusted archives and never commit either output.

## Restore testing

At regular intervals, create an isolated Compose project with unique ports and empty volumes, restore a backup, start it, run `./latex-core doctor`, and verify representative accounts, papers, versions, and blobs. Stop only that isolated project after the test. A backup is not qualified until a restore has been exercised.

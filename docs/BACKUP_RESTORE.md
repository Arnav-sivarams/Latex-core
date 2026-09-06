# Backup and restore

LaTeX Core recovery has two inseparable durable parts: PostgreSQL and the
content-addressed BlobStore volume. A database dump alone is not a complete
backup. The supported command briefly stops API and worker writers, verifies
PostgreSQL durability settings, dumps PostgreSQL, archives the immutable blob
volume, verifies both artifacts, and only then atomically publishes a restore
point with a `COMPLETE` marker.

## Recovery points and secrets

The manifest's `recovery_point_utc` is the latest point promised by that
backup. Changes acknowledged after that time are not in it. Process restarts
with intact volumes recover later acknowledged changes; volume loss does not.

Keep these values separately in protected operator storage:

- `POSTGRES_PASSWORD` (and the matching `DATABASE_URL`);
- `LATEX_CORE_MAIL_SECRET_KEY`, which is required to decrypt unexpired
  temporary-credential outbox payloads in the original installation;
- SMTP credentials when SMTP is configured.

The backup deliberately contains no `.env` or plaintext keys. Restored test
clones disable SMTP and expire queued credential payloads rather than replaying
historical mail. Permanent account password hashes remain in PostgreSQL.

## Create and verify a backup

Choose an absolute destination outside the Git checkout, preferably a mounted
off-host filesystem:

```sh
./latex-core backup /mnt/off-host/latex-core-backups
./scripts/verify-backup.sh \
  /mnt/off-host/latex-core-backups/latex-core-YYYYMMDDTHHMMSSZ-COMMIT
```

The destination root receives one new timestamped directory containing:

- `database.dump`: PostgreSQL custom-format dump, including the SQLx ledger;
- `blobs.tar.gz`: the complete BlobStore volume;
- `checksums.sha256` and `manifest.json`;
- `COMPLETE`, written only after validation succeeds.

An interrupted `.partial.*` directory is not a restore point. The backup
command resumes API/worker services through its exit trap. Confirm service
health after any host-level interruption.

## Restore into a new isolated installation

Restoring over an existing Compose project is intentionally unsupported. Make
a new mode-0600 environment file with a unique Compose project, HTTP/PostgreSQL
ports, and worker staging directory. Set `LATEX_CORE_MAIL_ENABLED=false`.
The database credentials must match those in the environment file; recovery
encryption keys remain in protected operator storage.

```sh
./scripts/restore-release.sh \
  /mnt/off-host/latex-core-backups/latex-core-YYYYMMDDTHHMMSSZ-COMMIT \
  --target-project latex-core-restore-drill-20260906 \
  --target-env /run/operator/latex-core-restore-drill.env
```

The command verifies backup hashes, manifest format, exact release commit,
schema level, absence of target containers/volumes, and SMTP disablement. It
then creates only the named target, restores PostgreSQL and blobs, hashes every
database-referenced blob, removes restored sessions, expires unsent credential
messages, and starts the target only after cross-store verification succeeds.

A failed restore stops the target PostgreSQL container but retains its isolated
volumes for diagnosis. It never alters the source installation or backup.
Delete an isolated drill only after separately confirming its Compose project
labels and volume names.

## Opt-in scheduling and retention

Copy `deploy/compose/backup-schedule.example` to a protected path outside Git,
set its destination and retention count, and optionally set an rclone remote.
Invoke the same backup command through the host's existing cron or systemd
timer, for example a daily cron entry:

```cron
17 2 * * * /opt/latex-core/scripts/scheduled-backup.sh /run/operator/latex-core-backup.conf
```

No schedule is installed automatically. Retention runs only after the new
backup is complete and any configured off-host `rclone copy` plus `rclone
check` succeeds. At least the newest verified backup is retained. Set
`LATEX_CORE_BACKUP_SCHEDULE_CONFIGURED=true` and
`LATEX_CORE_BACKUP_OFFHOST_CONFIGURED=true` in the application environment
only when those operator statements are true; Admin → System displays them.

Run a restore drill after release changes and periodically thereafter. A
process restart drill is not evidence of physical-disk or power-loss safety.

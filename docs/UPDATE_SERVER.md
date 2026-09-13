# Server updates

`./install.sh` is for initial installation and repair of the same deployment. Ordinary code updates use the incremental Compose workflow and preserve the existing `.env`, Compose project, PostgreSQL container, PostgreSQL volume, and BlobStore volume.

## Matching schema

After switching to an approved commit and confirming the working tree is clean, update both application services:

```sh
./scripts/update-deployment.sh api worker
```

The helper validates configuration, checks the exact SQLx migration ledger, builds before replacement, replaces only the selected services, waits for readiness, and verifies persistence identities. A failed build leaves the healthy running application containers in place.

Use a narrower form when the release affects only one executable:

```sh
./scripts/update-deployment.sh api
./scripts/update-deployment.sh worker
```

## Release with pending migrations

Schema changes require an explicit operator sequence:

1. Create a verified database and BlobStore backup in approved off-host storage.
2. Build the candidate API and Worker images while the current services remain available.
3. Run the candidate image's embedded SQLx migrator.
4. Verify the migration ledger against the checked-out source.
5. Replace API and Worker.
6. Verify status, Worker dependencies, installation, login, and one manual compile.

```sh
./latex-core backup /operator-approved/off-host/latex-core-backups
./scripts/verify-backup.sh /exact/timestamped/backup/path

source scripts/install-common.sh
latex_core_init "$PWD"
"${LATEX_CORE_COMPOSE[@]}" build api worker
"${LATEX_CORE_COMPOSE[@]}" run --rm --no-deps api \
  /usr/local/bin/latex-core-admin database migrate
./scripts/check-deployment-migrations.sh
./scripts/update-deployment.sh api worker

./latex-core status
./latex-core doctor
./install.sh --verify-only
```

Use the exact backup path printed by `./latex-core backup`; do not proceed unless independent verification passes. An older application image does not reverse a schema migration. Any image rollback requires a separate schema-compatibility review, and data rollback requires a deliberate restore procedure.

Do not regenerate `.env`, change `COMPOSE_PROJECT_NAME`, or reinstall the product for a normal code update. See [Backup and restore](BACKUP_RESTORE.md) and [Troubleshooting](TROUBLESHOOTING.md).

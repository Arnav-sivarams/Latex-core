# Install and operate LaTeX Core

## Requirements

- A Linux host with a current Docker Engine and Docker Compose plugin
- Enough disk for PostgreSQL, source blobs, build artifacts, and backups
- The repository checkout and permission to run Docker

The compiler image is digest-pinned. Do not replace it with an unreviewed TeX image.

## Install and start

From the repository root:

```sh
./install.sh
./latex-core start
./latex-core status
./latex-core doctor
./latex-core url
```

`status` reports service state. `doctor` checks PostgreSQL, blob storage, Docker access from the worker, and the pinned compiler environment.

## Accounts and templates

Use the documented operator commands, for example `./latex-core user --help` and `./latex-core template --help`. Do not place passwords in documentation, scripts, shell history, or source control.

## Backup and stop

Create a backup in an operator-controlled destination:

```sh
./latex-core backup /absolute/path/to/backup-directory
```

The backup contains a PostgreSQL dump and a blob archive. Test restoration separately from the active deployment.

Stop containers without deleting volumes:

```sh
./latex-core stop
```

Never use volume-deleting Compose options against a production installation.

Host-level operational actions remain CLI-only in this release candidate. The Admin SYSTEM page is read-only and does not expose the Docker socket or arbitrary command execution.

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

## Temporary-credential mail

Set `LATEX_CORE_MAIL_ENABLED=true` and configure `LATEX_CORE_SMTP_HOST`,
`LATEX_CORE_SMTP_PORT`, `LATEX_CORE_SMTP_FROM_EMAIL`, `LATEX_CORE_SMTP_FROM_NAME`,
`LATEX_CORE_SMTP_SECURITY` (`starttls` or `tls`), and `LATEX_CORE_PUBLIC_BASE_URL`.
Set `LATEX_CORE_SMTP_USERNAME` and `LATEX_CORE_SMTP_PASSWORD` together when the SMTP relay
requires authentication. These settings work with institutional SMTP and standard SMTP gateways
such as SES, SendGrid, and Mailgun; no provider-specific API is used.

Generate the payload-encryption key with `openssl rand -base64 32` and provide it as
`LATEX_CORE_MAIL_SECRET_KEY`. The default encrypted-payload lifetime is 72 hours and can be set
with `LATEX_CORE_MAIL_SECRET_LIFETIME_HOURS`. `LATEX_CORE_MAIL_BATCH_SIZE` bounds each claim and
`LATEX_CORE_MAIL_MAX_ATTEMPTS` caps automatic retries. `./latex-core doctor` validates the
configuration without sending a message.

Keep SMTP credentials and the mail encryption key outside Git. Never reuse the database password
as the mail encryption key. When mail is intentionally unavailable, leave delivery disabled; the
stack remains healthy and administrators can use the one-time credentials CSV.

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

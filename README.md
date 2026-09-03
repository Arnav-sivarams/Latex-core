# LaTeX Core

LaTeX Core is a self-hosted, server-side LaTeX paper platform. It provides durable real-time editing, immutable version history, manual and idle-triggered builds through a pinned TeX Live environment, PDF review, and governed administration. PostgreSQL is the source of truth; user LaTeX is compiled only through the worker sandbox boundary.

This is the Professor V2 release candidate. The retired V1 workspace and Research Group product surfaces remain closed.

## Roles

- **Writer** — authors personal papers and assigned Team Papers at `/write`.
- **Mentor** — reads assigned Team Papers, reviews PDFs, and decides restoration endorsements at `/review`.
- **Admin** — governs users, Paper Teams, templates, policies, versions, restoration, and system inspection at `/admin`.

## Quick start

```sh
./install.sh
./latex-core start
./latex-core status
./latex-core doctor
```

Temporary credentials can be delivered through provider-neutral SMTP using a durable encrypted
outbox. Mail is disabled by default; see [installation](docs/INSTALL.md) for configuration. The
administrator credentials CSV remains a one-time fallback.

Open the URL printed by `./latex-core url`. Accounts and roles are created by an operator; the browser login is server controlled.

## Guides

- [Installation and operations](docs/INSTALL.md)
- [Writer guide](docs/WRITER_GUIDE.md)
- [Mentor guide](docs/MENTOR_GUIDE.md)
- [Admin guide](docs/ADMIN_GUIDE.md)
- [V2 RC test report](docs/TEST_REPORT_V2_RC.md)
- [V2 RC release notes](docs/RELEASE_NOTES_V2_RC.md)
- [CLI reference](docs/CLI.md)

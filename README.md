# LaTeX Core

LaTeX Core is a self-hosted, server-side platform for institutional LaTeX papers. It provides durable collaborative editing, immutable version history, manual builds through a frozen TeX Live environment, PDF review, and governed administration. PostgreSQL is the source of truth; compilation runs only through the worker sandbox boundary.

This is the Professor V2 release candidate. The retired V1 workspace and Research Group product surfaces remain closed.

## Roles

- **Writer** — authors personal papers and assigned Team Papers at `/write`.
- **Mentor** — reads assigned Team Papers, reviews PDFs, and decides restoration endorsements at `/review`.
- **Admin** — governs users, Paper Teams, templates, policies, versions, restoration, and system inspection at `/admin`.

## Quick Start

The current installation-hardening candidate is published on the named candidate branch; no release tag points at it yet. Do not use `v2.3.0-rc2` when qualifying this candidate.

```sh
git clone https://github.com/Arnav-sivarams/latex-core.git
cd latex-core
git switch --track origin/professor-feedback-publication-workspace-ux
./install.sh
```

Open the URL printed by the installer and sign in with the Admin account created during installation. Follow the [Professor Test Guide](docs/PROFESSOR_TEST_GUIDE.md) for the complete walkthrough.

SMTP is optional during basic installation. To configure password-email delivery during installation, run:

```sh
./install.sh --configure-mail
```

## Guides

- [Local installation](docs/INSTALL_LOCAL.md)
- [Server installation](docs/INSTALL_SERVER.md)
- [SMTP setup](docs/SMTP_SETUP.md)
- [Backup and restore](docs/BACKUP_RESTORE.md)
- [Writer guide](docs/WRITER_GUIDE.md)
- [Mentor guide](docs/MENTOR_GUIDE.md)
- [Admin guide](docs/ADMIN_GUIDE.md)
- [V2 RC test report](docs/TEST_REPORT_V2_RC.md)
- [V2 RC release notes](docs/RELEASE_NOTES_V2_RC.md)
- [CLI reference](docs/CLI.md)

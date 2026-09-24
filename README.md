# LaTeX Core

LaTeX Core is a self-hosted platform for institutional LaTeX papers. It provides durable collaborative editing, immutable version history, manual builds through a frozen TeX Live environment, PDF review, and governed administration. PostgreSQL is the source of truth, and compilation runs only through the Worker sandbox boundary.

## Requirements

Ubuntu 22.04 or 24.04 LTS x86_64 and a normal sudo-capable account are supported. The installer provisions missing host utilities, Docker Engine 24+, Docker Compose 2.20+, and Buildx 0.12+ from Docker's signed Ubuntu repository, and starts a stopped local Docker service after showing the proposed host changes and requesting authorization. Existing compatible Docker installations are reused. No permanent Docker-group membership and no host PostgreSQL, Rust, Cargo, Node, npm, or TeX installation is required. See the [supported host contract](docs/INSTALL_SERVER.md).

## Quick Start

On the current feature branch:

```sh
git clone --branch feature/manual-compile-review-lock-institutional-api --single-branch https://github.com/Arnav-sivarams/latex-core.git
cd latex-core
./latex-core install
```

The installer handles sudo authorization inside that command, checks the host, generates a private `.env` and secrets once, creates checkout-owned worker staging, starts PostgreSQL, applies migrations, builds and starts the API/worker/proxy, checks health, and asks to create the first administrator. Existing configuration, accounts, blobs, and database volumes are preserved on subsequent installs. `sudo ./latex-core install` is also accepted: the launcher validates the real sudo identity and returns ordinary file work to that account instead of creating root-owned checkout state.

The default URL is `http://localhost:9000`, bound to loopback. For a remote host, run `ssh -L 9000:127.0.0.1:9000 USER@SERVER` on your laptop, then visit that URL locally.

## First Administrator

Answer the installer's prompt with an email and a password of 12–256 characters. Input is not echoed. If you deferred bootstrap with `./latex-core install --skip-admin`, run `./latex-core admin create` on the server afterward. For unattended bootstrap, use `./latex-core install --admin-email EMAIL --admin-password-stdin` and supply exactly one password line on standard input; never pass a password as a command-line argument. An existing administrator is never duplicated.

## Starting and Stopping

```sh
./latex-core start
./latex-core stop
./latex-core restart
```

Stopping retains persistent volumes. Restart performs an actual service restart followed by health checks.

## Checking Health and Logs

```sh
./latex-core status
./latex-core doctor
./latex-core url
./latex-core logs
```

`status` reports PostgreSQL/API health and service state; `doctor` verifies database, blob storage, writable staging, Docker access, and the frozen compiler. Use `./latex-core logs api` or `./latex-core logs worker` for a specific service. `./latex-core install --verify-only` checks the complete installation without starting or migrating services.

## Updating

For an existing deployment, follow the [reviewed update workflow](docs/UPDATE_SERVER.md), including backup and migration compatibility checks. Do not reset `.env` or run volume-deleting Compose commands as an update shortcut.

## Troubleshooting

Installation failures identify the stage and preserve any build/startup log privately under `.install-diagnostics/`. Fix the named port, configuration, Docker-access, or staging-path problem, then rerun `./latex-core install`. Existing data is retained. See [troubleshooting](docs/TROUBLESHOOTING.md) and `./latex-core diagnose` for bounded private diagnostics.

## Advanced/Developer Deployment

Custom `.env` settings and operational constraints are in the [server installation guide](docs/INSTALL_SERVER.md). The [local installation guide](docs/INSTALL_LOCAL.md) covers developer machines. The application remains containerized; `scripts/install-common.sh` is an internal library and is never a prerequisite for CLI commands.

## Roles

- **Writer** authors personal papers and assigned Team reports at `/write`. A Team Leader can manage Team-scoped details and send a report for review.
- **Mentor** reviews assigned Team reports and explicitly publishes private draft feedback at `/review`.
- **Admin** manages institutional records, people, programmes, Paper Teams, templates, branding, policies, versions, and recovery status at `/admin`.

## Operations

- [Server installation](docs/INSTALL_SERVER.md)
- [Normal server updates](docs/UPDATE_SERVER.md)
- [Troubleshooting](docs/TROUBLESHOOTING.md)
- [Backup and restore](docs/BACKUP_RESTORE.md)
- [SMTP setup](docs/SMTP_SETUP.md)
- [CLI reference](docs/CLI.md)
- [Writer guide](docs/WRITER_GUIDE.md)
- [Mentor guide](docs/MENTOR_GUIDE.md)
- [Admin guide](docs/ADMIN_GUIDE.md)
- [Front Matter packs](docs/FRONT_MATTER_PACKS.md)

# Server installation

## Fresh install

Run these commands on a new Ubuntu Server 22.04 or 24.04 LTS x86_64 host:

```sh
git clone --branch feature/manual-compile-review-lock-institutional-api --single-branch \
  https://github.com/Arnav-sivarams/latex-core.git
cd latex-core
./latex-core install
```

This is the only setup command. You may authorize sudo, skip SMTP, and create the first Admin when prompted inside it. The installer generates `.env` once with restrictive permissions and preserves it on later runs. Do not run `apt`, `usermod`, `newgrp`, `systemctl`, Docker diagnostics, or internal scripts as installation steps.

Verify the result:

```sh
./latex-core status
./latex-core doctor
./latex-core install --verify-only
```

The local server URL is `http://127.0.0.1:9000`. For access from a laptop:

```sh
ssh -L 9000:127.0.0.1:9000 USER@SERVER
```

Open `http://localhost:9000` on the laptop. A normal fresh install requires no manual Compose edits.

## Supported host contract

The supported host is Ubuntu Server 22.04 LTS or Ubuntu Server 24.04 LTS on x86_64 with a normal sudo-capable account. The supported runtime is a local, rootful daemon at `unix:///var/run/docker.sock`, Docker Engine 24 or newer, Docker Compose 2.20 or newer, and Buildx 0.12 or newer. Rootless Docker, remote contexts, alternate sockets, and CPU emulation do not satisfy the Worker compiler-mount contract.

When prerequisites are missing, the installer lists the changes and requests sudo authorization through the controlling terminal. It installs basic utilities from Ubuntu and installs exact APT candidate versions of `docker-ce`, `docker-ce-cli`, `containerd.io`, `docker-buildx-plugin`, and `docker-compose-plugin` from Docker's official HTTPS repository after verifying its signing-key fingerprint. It never runs a downloaded root shell. Package-lock waits are bounded. Existing compatible Docker is reused; a running daemon is not restarted. Conflicting administrator-managed container packages and a masked Docker service produce explicit failures instead of automatic removal or unmasking.

Docker is used directly when the account already has access. Otherwise every Docker and Compose operation goes through the same sudo-assisted executor, including image inspection/pulls, builds, migrations, bootstrap, verification, diagnostics, and later lifecycle commands. Authorization is read from `/dev/tty`, never from administrator-password stdin, and is renewed there if it expires. No group change, logout, new shell, socket chmod, sudoers rule, or stored sudo password is used. `sudo ./latex-core install` is supported by validating `SUDO_UID`, `SUDO_GID`, `SUDO_USER`, the account home, and checkout ownership, then returning configuration and file creation to that account.

The host does not need PostgreSQL Server, `psql`, Rust, Cargo, Node, npm, TeX, generated frontend files, or prior LaTeX Core state.

PostgreSQL comes from the Compose stack. Rust builds run in the Docker build stage. The application image contains the embedded SQLx migrator, and installation runs it against the Compose database. The frozen M7 compiler image is pulled and checked by immutable image ID; the installer does not rebuild it.

The preflight warns below 10 GiB free source or Docker storage and below 4 GiB available memory. Operators must still size CPU, memory, storage, backup retention, and concurrent compilation capacity for their use.

## Created resources and ports

The installer creates a mode-600 `.env`, a private dedicated Worker staging directory, and Compose-managed PostgreSQL and BlobStore volumes. A new checkout gets a stable project name derived from its canonical path, avoiding collisions with other checkouts; existing `.env` project names and valid staging permissions are preserved. Verification checks the staging directory from the deployment account and confirms that the running Worker uses the exact configured host bind. Rerunning preserves resources, credentials, accounts, and data. Invalid existing settings produce a named error; the installer does not silently regenerate `.env`, recursively change ownership, or reset volumes.

Fresh defaults resolve to:

| Host binding | Destination | Purpose |
| --- | --- | --- |
| `127.0.0.1:9000` | Caddy `8080`, then API `8080` | application access |
| `127.0.0.1:9001` | PostgreSQL `5432` | loopback diagnostics |
| none | API `8080` | Compose network only |
| none | Worker | Compose network only |

The application `DATABASE_URL` remains `postgres:5432`; host port 9001 is never used for service-to-service access. PostgreSQL is not publicly exposed.

## HTTPS and SMTP

DNS, TLS certificates, firewall policy, and an institutional reverse proxy remain operator inputs. Forward the existing proxy to loopback port 9000, then set the deployment `.env` consistently:

```text
SESSION_COOKIE_SECURE=true
LATEX_CORE_PUBLIC_BASE_URL=https://latex.example.edu
```

Use `./latex-core install --configure-mail` when SMTP and the public URL are ready. See [SMTP setup](SMTP_SETUP.md). Mail-disabled startup requires no SMTP credentials and retains the generated mail encryption key for later configuration.

## Failure handling

An installation failure prints the phase, keeps existing data intact, and writes bounded private diagnostics under `.install-diagnostics/`. It does not print raw service logs or secret environment values. Run:

```sh
./latex-core diagnose
./latex-core status
./latex-core logs api
./latex-core logs worker
./latex-core logs database
./latex-core logs caddy
```

Review diagnostic files for sensitive content before sharing them. Continue with [troubleshooting](TROUBLESHOOTING.md). For later code releases, use the [normal update workflow](UPDATE_SERVER.md) instead of reinstalling.

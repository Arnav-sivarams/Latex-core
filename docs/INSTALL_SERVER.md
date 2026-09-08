# Server installation

## Supported host contract

This candidate supports a native Ubuntu Server 24.04 LTS x86_64 host with Docker Engine 24 or newer and Docker Compose 2.20 or newer. The Docker CLI must address the local Linux daemon through `unix:///var/run/docker.sock`; remote contexts, rootless Docker, alternate socket paths, and CPU emulation are rejected because the Worker passes that socket and absolute staging paths to compiler containers. The frozen M7 compiler is `linux/amd64` and its image identity remains fixed.

Run installation as the normal login account that already has Docker access. Do not run it through `sudo`: changing users can select another Docker context and create root-owned `.env` and staging files. The API image runs without the Docker socket. The Worker container runs as root so it can use the local daemon socket, while every compiler container retains the M7 sandbox's UID 10001, disabled network, read-only root, dropped capabilities, resource limits, and dedicated per-job bind mount.

Required host commands are Git, Docker, the Docker Compose plugin, awk, sed, grep, mktemp, chmod, mv, Python 3, curl, `ss` from iproute2, and `df`. Secret generation additionally uses OpenSSL when available (or Python 3). The preflight warns below 10 GiB free source-filesystem space or 4 GiB currently available memory; these are warnings, not hardware-capacity guarantees. Operators must size CPU, memory, Docker storage, PostgreSQL/blob volumes, and retention for their own workload.

## Basic staging installation

The current hardening candidate is on a branch and is not represented by `v2.3.0-rc2`. Obtain that exact published branch without moving any tag:

```sh
git clone https://github.com/Arnav-sivarams/latex-core.git
cd latex-core
git switch --track origin/professor-feedback-publication-workspace-ux
./install.sh
```

The default binds Caddy to `127.0.0.1:8080`, disables registration and SMTP, and is suitable for local access or an SSH tunnel:

```sh
ssh -L 8080:127.0.0.1:8080 operator@server
```

The installer generates `.env` once with mode 600, a random PostgreSQL password, a matching URL-encoded-safe database URL, and a standard-base64 32-byte mail encryption key. Mail-disabled startup does not require SMTP host, username, or password. The mail key is generated separately so later mail enablement does not require replacing other secrets; disabling mail never deletes queued credentials or rewrites that key.

Rerunning the installer preserves `.env`, the Compose project name, named PostgreSQL/BlobStore volumes, accounts, and the dedicated Worker staging location. Invalid or missing existing settings produce a named correction; the installer does not regenerate passwords or reset volumes. Configuration values may use Compose-style single or double quotes, and the files are parsed as data—not sourced or evaluated as shell code.

## Institution-facing HTTPS

DNS, certificate issuance, firewall policy, and an existing institution reverse proxy remain operator inputs. The installer does not modify them or take over ports 80/443. Terminate TLS in the operator-managed proxy and forward it to the loopback HTTP port. Set the deployment `.env` values consistently before rerunning:

```text
SESSION_COOKIE_SECURE=true
LATEX_CORE_PUBLIC_BASE_URL=https://your-real-domain.example
```

Use `./install.sh --configure-mail` only when SMTP and the public URL are ready. If intentionally exposing the bundled Caddy port beyond loopback, set `HTTP_BIND_ADDRESS=0.0.0.0` and enforce the intended host network policy externally; the installer never changes the firewall.

## Startup and failure evidence

Startup displays separate image acquisition/build, PostgreSQL readiness and application-credential access, embedded SQLx migration, API database readiness, Worker dependency readiness, Caddy readiness, and Admin bootstrap phases. Checks are bounded and stop early on repeated container failure. SQLx remains the only migration mechanism: startup runs the embedded migrator explicitly and the API/Worker retain its locked, checksum-validating idempotent startup behavior.

On failure, private bounded evidence is saved under `.install-diagnostics/` without dumping container configuration or complete environments. It includes stopped/restarting containers, exit/OOM/restart state, image IDs, mount identities, and recent API/Worker/PostgreSQL/Caddy logs. Raw logs are not printed and must be reviewed for secrets or personal data before sharing.

```sh
./scripts/diagnose-install.sh
./latex-core status
./latex-core logs worker
./install.sh --verify-only
```

The diagnostic command never installs, migrates, restarts, or executes commands inside application containers. Existing-volume or credential mismatches are reported; no reset, prune, or volume deletion is performed.

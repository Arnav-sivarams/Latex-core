# Server installation

## Fresh install

Run these commands on a new Ubuntu Server 24.04 LTS x86_64 host:

```sh
git clone --branch release/complete-candidate-20260913 --single-branch \
  https://github.com/Arnav-sivarams/latex-core.git
cd latex-core
./install.sh
```

You may skip SMTP when prompted. Create the first Admin when prompted. The installer generates `.env` once with restrictive permissions and preserves it on later runs.

Verify the result:

```sh
./latex-core status
./latex-core doctor
./install.sh --verify-only
```

The local server URL is `http://127.0.0.1:9000`. For access from a laptop:

```sh
ssh -L 9000:127.0.0.1:9000 USER@SERVER
```

Open `http://localhost:9000` on the laptop. A normal fresh install requires no manual Compose edits.

## Supported host contract

The supported host is Ubuntu Server 24.04 LTS on x86_64 with Docker Engine 24 or newer and Docker Compose 2.20 or newer. Run installation as the normal login account with access to the local, rootful Docker daemon at `unix:///var/run/docker.sock`; do not run it through `sudo`. Rootless Docker, remote Docker contexts, alternate sockets, and CPU emulation do not satisfy the Worker compiler-mount contract.

The host commands checked by the installer are Git, Docker, the Docker Compose plugin, `awk`, `sed`, `grep`, `mktemp`, `chmod`, `mv`, Python 3, `curl`, `ss`, `df`, and `sha384sum`. OpenSSL is optional because Python 3 can generate secrets. The host does not need PostgreSQL Server, `psql`, Rust, Cargo, Node, npm, generated frontend files, or prior LaTeX Core state.

PostgreSQL comes from the Compose stack. Rust builds run in the Docker build stage. The application image contains the embedded SQLx migrator, and installation runs it against the Compose database. The frozen M7 compiler image is pulled and checked by immutable image ID; the installer does not rebuild it.

The preflight warns below 10 GiB free source or Docker storage and below 4 GiB available memory. Operators must still size CPU, memory, storage, backup retention, and concurrent compilation capacity for their use.

## Created resources and ports

The installer creates a mode-600 `.env`, a dedicated Worker staging directory, and Compose-managed PostgreSQL and BlobStore volumes. Rerunning it preserves those resources, credentials, accounts, and data. Invalid existing settings produce a named error; the installer does not silently regenerate `.env` or reset volumes.

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

Use `./install.sh --configure-mail` when SMTP and the public URL are ready. See [SMTP setup](SMTP_SETUP.md). Mail-disabled startup requires no SMTP credentials and retains the generated mail encryption key for later configuration.

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

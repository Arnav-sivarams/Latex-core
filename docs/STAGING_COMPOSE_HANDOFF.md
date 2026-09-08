# Staging Compose handoff

This is a controlled candidate for the existing Compose deployment. It does not
replace `.env`, bootstrap accounts, delete volumes, or update Git automatically.
The candidate commit is the exact output of `git rev-parse HEAD` on branch
`staging-compose-handoff-candidate`; compare it with the SHA in the accompanying
handoff message before running an update.

## Prerequisites and files

Supported host: Ubuntu 24.04 LTS on x86_64, Docker Engine 24 or newer, Docker
Compose v2.20 or newer, local rootful Docker at `/var/run/docker.sock`, Python 3,
`curl`, `ss`, and at least 4 GiB memory. The frozen M7 compiler must remain
`sha256:8db804f76b8e80e5be9fb28ba14b0938df5989b7a8250ca6b0e9f3c200c4ee38`.

- Application image: [`deploy/Dockerfile`](../deploy/Dockerfile)
- Compose model: [`deploy/compose/docker-compose.yml`](../deploy/compose/docker-compose.yml)
- Build exclusions: [`.dockerignore`](../.dockerignore)
- Nonsecret proxy configuration: [`deploy/compose/Caddyfile`](../deploy/compose/Caddyfile)
- Configuration schema/defaults: [`.env.example`](../.env.example)

The Docker build context is the repository root. The Dockerfile alone is not a
buildable distribution: Cargo manifests, Rust sources, migrations, locked
dependencies, and bundled static assets from the approved commit are required.

| Host publication | Container/network destination | Exposure |
|---|---|---|
| `${HTTP_BIND_ADDRESS}:9000` | Caddy `8080`, then `api:8080` | fresh default `127.0.0.1` |
| `127.0.0.1:9001` | PostgreSQL `5432` | diagnostics only |
| none | API `8080` | Compose network only |
| none | Worker | Compose network only |

Internal `DATABASE_URL` remains `postgres:5432`. Do not substitute host port
9001. No other service is published.

## First candidate update

On the server, stop if `git status --short` shows local work that has not been
reviewed. Fetching and switching must use the explicitly approved candidate:

```sh
git status --short
git fetch origin staging-compose-handoff-candidate
git switch staging-compose-handoff-candidate
git pull --ff-only origin staging-compose-handoff-candidate
git rev-parse HEAD
```

Keep the existing `.env`. To opt in to the new ports, narrowly edit/add these
three settings; choose `0.0.0.0` only if the existing institutional proxy/firewall
requires that exposure:

```dotenv
HTTP_PORT=9000
HTTP_BIND_ADDRESS=127.0.0.1
LATEX_CORE_POSTGRES_PORT=9001
```

Do not change an existing valid HTTPS `LATEX_CORE_PUBLIC_BASE_URL` when changing
the backend port. For localhost HTTP, set it to `http://localhost:9000` only if
the key already exists or mail will be enabled. Validate without executing `.env`:

```sh
python3 scripts/validate-install-config.py .env
source scripts/install-common.sh
latex_core_init "$PWD"
"${LATEX_CORE_COMPOSE[@]}" config --quiet
```

Candidate migrations 0025 and 0026 are additive, but the staging database must
still be backed up and compatibility-reviewed. Take and verify the backup first,
then build while old services run and migrate with the built candidate image.
Running an older image does not reverse a schema change.

```sh
./latex-core backup /operator-approved/off-host/latex-core-pre-candidate
./scripts/verify-backup.sh /exact/verified/backup/path/printed/by/the/previous/command
"${LATEX_CORE_COMPOSE[@]}" build api worker
"${LATEX_CORE_COMPOSE[@]}" run --rm --no-deps api /usr/local/bin/latex-core-admin database migrate
./scripts/check-deployment-migrations.sh
./scripts/update-deployment.sh api worker
```

The backup destination is operator policy, not a repository default. Enter a
real approved root, use the exact timestamped backup path printed by the backup
command for the independent verification, and do not proceed unless it passes. The helper
rebuilds (normally from cache), replaces only API/Worker, performs bounded
readiness checks, and confirms the PostgreSQL container and blob volume
identities did not change. A brief API/Worker interruption is expected.

## Ordinary application updates

The first command builds an image without replacing a running container. The
second command recreates only the named service after configuration and schema
checks pass:

```sh
# API only
./scripts/update-deployment.sh api

# Worker only
./scripts/update-deployment.sh worker

# Both
./scripts/update-deployment.sh api worker
```

Equivalent explicit commands, after the validation/initialization block above:

```sh
"${LATEX_CORE_COMPOSE[@]}" build api
./scripts/check-deployment-migrations.sh
"${LATEX_CORE_COMPOSE[@]}" up -d --no-deps api

"${LATEX_CORE_COMPOSE[@]}" build worker
./scripts/check-deployment-migrations.sh
"${LATEX_CORE_COMPOSE[@]}" up -d --no-deps worker

"${LATEX_CORE_COMPOSE[@]}" build api worker
./scripts/check-deployment-migrations.sh
"${LATEX_CORE_COMPOSE[@]}" up -d --no-deps api worker
```

A failed build occurs before `up` and leaves running application containers in
place. A readiness failure after replacement is reported, with private bounded
logs under `.install-diagnostics/`; log collection does not repair the failure.

## Port-only update

No application build is needed. After changing only `.env`, resolve the final
ports and recreate only services whose published mappings changed:

```sh
source scripts/install-common.sh
latex_core_init "$PWD"
python3 scripts/validate-install-config.py "$LATEX_CORE_ENV_FILE"
"${LATEX_CORE_COMPOSE[@]}" config --format json | python3 -c 'import json,sys; data=json.load(sys.stdin); print("\n".join("{} {}:{} -> {}/{}".format(name,port.get("host_ip","*"),port.get("published"),port.get("target"),port.get("protocol","tcp")) for name,service in data["services"].items() for port in service.get("ports", [])))'
"${LATEX_CORE_COMPOSE[@]}" up -d --no-deps caddy
```

If the diagnostic PostgreSQL port also changed, plan a brief database connection
interruption, take the required backup, and run:

```sh
"${LATEX_CORE_COMPOSE[@]}" up -d --no-deps postgres
"${LATEX_CORE_COMPOSE[@]}" up -d --no-deps caddy
./scripts/verify-install.sh
```

The Compose project name must remain the existing value (legacy default
`latex-core`), which preserves the exact named PostgreSQL/blob volumes. Never use
`down -v`, regenerate `.env`, or run the installer for an ordinary update.

## Operate and diagnose

```sh
./latex-core status
./latex-core doctor
./latex-core logs api
./latex-core logs worker
./latex-core logs database
./latex-core logs caddy
./latex-core diagnose
./scripts/verify-install.sh
```

Fresh loopback access is `http://127.0.0.1:9000`. From an operator workstation:

```sh
ssh -L 9000:127.0.0.1:9000 operator@server
```

Then browse to `http://localhost:9000`. An institutional HTTPS proxy may retain
its public domain and forward to loopback port 9000.

## Limitations and rollback

- The original native staging exception was not available for reproduction;
  local source evidence only established the legacy `.env` preflight defect.
- Lease recovery runs when a Worker starts. An expired lease created by another
  failed Worker is not periodically reaped by an already-running Worker.
- There is no zero-downtime promise. API/Worker replacement is brief; PostgreSQL
  port remapping recreates that container and interrupts connections.
- Image rollback requires an explicitly approved older commit, rebuild, and
  service replacement. It is allowed only after schema compatibility review;
  it never reverses migrations or restores deleted/changed data.

# M10 release operation

For a fresh Docker server checkout, run `./scripts/install-server.sh`. It verifies the exact frozen M7 image (or loads `M7_IMAGE_TAR`), creates `.env` only when missing, prepares staging, starts Compose, and runs doctor. It never rebuilds M7 or overwrites an existing `.env`.

Normal browser registration is disabled by `ALLOW_REGISTRATION=false`. Create users only from the server owner path:

```sh
./scripts/create-user.sh alice@example.com
./scripts/server.sh {start|stop|status|doctor|logs|backup DIRECTORY}
```

Copy `.env.example` to `.env`, replace the password (and matching `DATABASE_URL`) plus the immutable M7 compiler image digest and its exact `TEX_ENVIRONMENT_ID`, then run:

```sh
docker compose -f deploy/compose/docker-compose.yml up -d --build
scripts/release-smoke.sh
```

The API and browser only receive the PostgreSQL and blob volumes. The worker alone mounts `/var/run/docker.sock`; it creates unique temporary staging directories in a dedicated worker-only host directory, materializes canonical manifests there, and deletes them when each compiler execution ends. Set `WORKER_STAGING_HOST_ROOT` to a root-owned dedicated directory (the local default is `/tmp/latex-core-worker-staging`); it is mounted at the same absolute path so Docker can bind only the per-job child directory. Compiler containers are labelled `latex-core.application=latex-core` and `latex-core.job-id=<job UUID>`; worker startup reaps only containers with the application label.

Use `docker compose ... run --rm worker /usr/local/bin/latex-core-doctor` to check PostgreSQL migrations, blob storage, Docker, and the configured immutable compiler image. Back up with `scripts/backup-release.sh backups/DATE`; restore with `scripts/restore-release.sh backups/DATE` while services are stopped.

Set `SESSION_COOKIE_SECURE=true` when deployed behind real HTTPS. Caddy provides a single same-origin HTTP entry point; use an external TLS-capable Caddy configuration before internet exposure.

Explicitly deferred: ZIP import, collaboration/CRDT, OAuth, password reset/email verification, teams, billing, WebSockets, polished/mobile UI, advanced administration, and load qualification.

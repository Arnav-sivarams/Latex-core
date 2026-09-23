# Local installation

## One-command installation

The qualified server contract is in [Server installation](INSTALL_SERVER.md). Local development is accepted on Ubuntu 22.04 or 24.04 LTS x86_64, including WSL2 Ubuntu when Docker Desktop exposes the local `/var/run/docker.sock` endpoint. Native macOS and non-amd64 images are not supported by this installer candidate.

```sh
git clone https://github.com/Arnav-sivarams/latex-core.git
cd latex-core
git switch --track origin/feature/manual-compile-review-lock-institutional-api
./latex-core install
```

The installer verifies Docker, anonymously obtains the frozen M7 compiler image when necessary, creates a private `.env`, starts PostgreSQL/API/worker/Caddy, applies migrations, runs the doctor and verifier, and prompts for the first Admin email and permanent password. SMTP is optional. Open the printed URL (normally `http://localhost:9000`).

Useful commands:

```sh
./latex-core status
./latex-core doctor
./latex-core logs
./latex-core logs api
./latex-core logs worker
./latex-core diagnose
./latex-core restart
./latex-core stop
./latex-core install --verify-only
```

Running `./latex-core install` again preserves `.env`, PostgreSQL data, BlobStore data, and existing accounts. To enable SMTP later, run `./latex-core install --configure-mail`.

## Updating

Back up first and follow the [reviewed update procedure](UPDATE_SERVER.md) for an existing deployment. Never copy another installation's `.env` or database into a fresh installation.

```sh
./latex-core backup /absolute/path/to/backup
git fetch origin --tags
git checkout <new-release-tag>
./scripts/update-deployment.sh api worker
```

## Manual fallback

Use this only to diagnose the one-command flow:

```sh
./scripts/generate-local-env.sh
./latex-core start
./latex-core doctor
./latex-core admin create
./latex-core install --verify-only
```

The installer never installs operating-system packages with `sudo`, rebuilds M7, resets PostgreSQL, or removes Docker volumes.

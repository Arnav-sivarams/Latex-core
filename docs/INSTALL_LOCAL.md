# Local installation

## One-command installation

The qualified server contract is in [Server installation](INSTALL_SERVER.md). Local development is accepted on Ubuntu 24.04 x86_64, including WSL2 Ubuntu when Docker Desktop exposes the local `/var/run/docker.sock` endpoint. Native macOS and non-amd64 images are not supported by this installer candidate.

```sh
git clone https://github.com/Arnav-sivarams/latex-core.git
cd latex-core
git switch --track origin/professor-feedback-publication-workspace-ux
./install.sh
```

The installer verifies Docker, anonymously obtains the frozen M7 compiler image when necessary, creates a private `.env`, starts PostgreSQL/API/worker/Caddy, applies migrations, runs the doctor and verifier, and prompts for the first Admin email and permanent password. SMTP is optional. Open the printed URL (normally `http://localhost:8080`).

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
./install.sh --verify-only
```

Running `./install.sh` again preserves `.env`, PostgreSQL data, BlobStore data, and existing accounts. To enable SMTP later, run `./install.sh --configure-mail`.

## Updating

Back up first, fetch the intended release, check it out, and rerun the installer. Never copy another installation's `.env` or database into a fresh installation.

```sh
./latex-core backup /absolute/path/to/backup
git fetch origin --tags
git checkout <new-release-tag>
./install.sh
```

## Manual fallback

Use this only to diagnose the one-command flow:

```sh
./scripts/generate-local-env.sh
./latex-core start
./latex-core doctor
./scripts/bootstrap-admin.sh
./scripts/verify-install.sh
```

The installer never installs operating-system packages with `sudo`, rebuilds M7, resets PostgreSQL, or removes Docker volumes.

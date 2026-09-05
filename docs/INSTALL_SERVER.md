# Server installation

This guide targets a maintained Ubuntu 24.04 server. Install Docker Engine from Docker's official repository and install the Docker Compose v2 plugin. Allow only SSH (22), HTTP (80), and HTTPS (443) through the host firewall. PostgreSQL must not be exposed publicly.

## Install

```sh
git clone https://github.com/Arnav-sivarams/latex-core.git
cd latex-core
git checkout v2.3.0-rc2
./install.sh
```

The installer pulls the frozen compiler image from GHCR, creates `.env`, starts the current Compose stack, migrates PostgreSQL, and bootstraps the first Admin. Preserve `.env` with mode 600 and back it up separately from source control.

## Domain and TLS

Point the domain's DNS records at the server. The repository Caddy service currently reverse-proxies the API on its internal port; for an Internet deployment, terminate TLS with the site's Caddy configuration and publish 80/443 instead of the local 8080 mapping. Set:

```text
SESSION_COOKIE_SECURE=true
LATEX_CORE_PUBLIC_BASE_URL=https://latex.example.edu
```

Do not publish the Compose PostgreSQL port beyond loopback. Configure SMTP with `./install.sh --configure-mail` after DNS/TLS are correct.

## Persistent state and operations

The named PostgreSQL and BlobStore volumes are both required state. The worker staging directory is temporary; compiler jobs remain durable in PostgreSQL. Operate the stack with:

```sh
./latex-core status
./latex-core doctor
./latex-core logs
./latex-core restart
./latex-core stop
```

Back up PostgreSQL and BlobStore together with `./latex-core backup /srv/backups/latex-core/<date>`. Test restores on an isolated host or isolated Compose project. See [Backup and Restore](BACKUP_RESTORE.md).

## Updates

Create and verify a backup, fetch the intended signed/annotated release tag, inspect release notes, check out that tag, and rerun `./install.sh`. The installer preserves existing secrets and volumes. Never use `docker compose down -v` during an update.

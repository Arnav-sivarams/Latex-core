# Server setup

From a checkout on the server, run `./install.sh`. If the frozen compiler image is not already loaded, provide its approved tarball:

```sh
M7_IMAGE_TAR=/path/to/latex-core-texlive-2026-m7.tar ./install.sh
```

The installer creates a private `.env`, verifies the frozen compiler image, starts the service, and installs `/usr/local/bin/latex-core` when that directory is writable. The repository-local `./latex-core` always remains available.

Normal administration uses the product CLI:

```sh
latex-core start
latex-core status
latex-core doctor
latex-core user create alice@example.com
latex-core backup /srv/backups/latex-core
```

Use `latex-core stop` and `latex-core restart` for service lifecycle operations. `latex-core url` prints the local service URL. Registration is disabled by default; accounts are provisioned by the server owner.

## Advanced troubleshooting

`latex-core logs`, `latex-core logs api`, `latex-core logs worker`, and `latex-core logs database` expose service logs without requiring container identifiers. Docker Compose is an implementation detail; use it directly only for advanced diagnosis.

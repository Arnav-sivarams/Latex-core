# Server setup

From a checkout on the server, run the supported root installer:

```sh
./install.sh
```

The installer anonymously pulls the public frozen compiler image from GHCR when it is absent, verifies its exact image ID, creates a private `.env`, and starts the service. The repository-local `./latex-core` provides all operator commands.

Normal administration uses the product CLI:

```sh
./latex-core start
./latex-core status
./latex-core doctor
./latex-core user create alice@example.com
./latex-core backup /srv/backups/latex-core
```

Use `./latex-core stop` and `./latex-core restart` for service lifecycle operations. `./latex-core url` prints the local service URL. Registration is disabled by default; accounts are provisioned by the server owner.

## Advanced troubleshooting

`./latex-core logs`, `./latex-core logs api`, `./latex-core logs worker`, and `./latex-core logs database` expose service logs without requiring container identifiers. Docker Compose is an implementation detail; use it directly only for advanced diagnosis.

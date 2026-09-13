# Troubleshooting

Start with the non-mutating checks:

```sh
./latex-core status
./latex-core doctor
./latex-core diagnose
```

Read the relevant bounded service log:

```sh
./latex-core logs api
./latex-core logs worker
./latex-core logs database
./latex-core logs caddy
```

| Symptom | First check |
| --- | --- |
| API or Worker is restarting | Read that service's log. |
| Database is down or unhealthy | Read `./latex-core logs database`. PostgreSQL is the Compose service; no host PostgreSQL or `psql` is required. |
| UI is unavailable while services are healthy | Check Caddy logs, loopback binding, firewall/reverse-proxy routing, and the SSH tunnel. |
| A manual compile fails | Check Worker logs and `./latex-core doctor` for the frozen M7 image, Docker socket, staging path, database, and BlobStore contract. |
| Installation fails | Run `./latex-core diagnose` first and use the named failed phase. |

The installer automatically retains useful evidence under `.install-diagnostics/` when it fails. Diagnostics do not install, migrate, restart, reset, or delete anything. Raw logs are stored with restrictive permissions and are not printed; review them for secrets or personal data before sharing.

For a healthy installation, this final check must pass:

```sh
./install.sh --verify-only
```

Do not delete volumes or regenerate `.env` to address a diagnostic error. Correct the named configuration, port, permission, image, database, or service failure and rerun the relevant check.

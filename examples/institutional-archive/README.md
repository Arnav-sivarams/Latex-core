# Institutional archive example

This standard-library Python client demonstrates the read-only HTTP API; it never connects to PostgreSQL. Set `LATEX_CORE_INTEGRATION_TOKEN` from private input, then run:

```sh
python3 archive.py --base-url http://127.0.0.1:9000 --output ./archive-output
```

Use HTTPS outside trusted local testing. Reruns atomically replace JSON and matching downloaded objects, verify SHA-256 values, preserve stable report/version/build IDs in directory names, and do not print or persist the token.

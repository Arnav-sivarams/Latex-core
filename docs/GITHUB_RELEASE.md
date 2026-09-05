# GitHub release procedure

Canonical source: `https://github.com/Arnav-sivarams/latex-core`

Frozen compiler: `ghcr.io/arnav-sivarams/latex-core-texlive:2026-m7`

Professor releases are immutable annotated Git tags. The V2.3 professor candidate is `v2.3.0-rc2`; source documentation and installer behavior must match that tag. Never move or force-update a published release tag.

The compiler container is a separately published frozen artifact. `install.sh` pulls it anonymously when the matching local image is absent and verifies image ID `sha256:8db804f76b8e80e5be9fb28ba14b0938df5989b7a8250ca6b0e9f3c200c4ee38`. A source release must not rebuild M7.

For an update, qualify source gates, perform a clean `git archive` installation with isolated volumes/ports, verify anonymous GHCR pull, create the annotated tag, and push main and the tag without force. Keep release notes tied to the exact commit.

Never commit `.env`, credentials CSVs, SMTP credentials, mail encryption keys, database dumps, BlobStore data, private institutional records, browser/UAT output, backups, or container archives. Scan both the proposed tree and Git history before publication.

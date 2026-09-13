# LaTeX Core

LaTeX Core is a self-hosted platform for institutional LaTeX papers. It provides durable collaborative editing, immutable version history, manual builds through a frozen TeX Live environment, PDF review, and governed administration. PostgreSQL is the source of truth, and compilation runs only through the Worker sandbox boundary.

This branch is a release candidate. It does not replace `main` or a release tag.

## Fresh server install

On a new supported Ubuntu server:

```sh
git clone --branch release/complete-candidate-20260913 --single-branch \
  https://github.com/Arnav-sivarams/latex-core.git
cd latex-core
./install.sh
```

SMTP may be skipped initially. Create the first Admin when the installer prompts. Then verify the installation:

```sh
./latex-core status
./latex-core doctor
./install.sh --verify-only
```

On the server, open `http://127.0.0.1:9000`. From a remote laptop, create an SSH tunnel:

```sh
ssh -L 9000:127.0.0.1:9000 USER@SERVER
```

Then open `http://localhost:9000` on the laptop.

The supported host needs Git, Docker Engine, Docker Compose, and the ordinary Ubuntu utilities listed in the [server installation guide](docs/INSTALL_SERVER.md). It does not need host PostgreSQL, `psql`, Rust, Cargo, Node, or npm. PostgreSQL, application builds, and migrations run through containers. A fresh install creates its own `.env`, PostgreSQL volume, BlobStore volume, and Worker staging directory.

## Roles

- **Writer** authors personal papers and assigned Team reports at `/write`. A Team Leader can manage Team-scoped details and send a report for review.
- **Mentor** reviews assigned Team reports and explicitly publishes private draft feedback at `/review`.
- **Admin** manages institutional records, people, programmes, Paper Teams, templates, branding, policies, versions, and recovery status at `/admin`.

## Operations

- [Server installation](docs/INSTALL_SERVER.md)
- [Normal server updates](docs/UPDATE_SERVER.md)
- [Troubleshooting](docs/TROUBLESHOOTING.md)
- [Backup and restore](docs/BACKUP_RESTORE.md)
- [SMTP setup](docs/SMTP_SETUP.md)
- [CLI reference](docs/CLI.md)
- [Writer guide](docs/WRITER_GUIDE.md)
- [Mentor guide](docs/MENTOR_GUIDE.md)
- [Admin guide](docs/ADMIN_GUIDE.md)
- [Front Matter packs](docs/FRONT_MATTER_PACKS.md)

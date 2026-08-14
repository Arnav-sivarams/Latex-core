# LaTeX Core

LaTeX Core is a self-hosted, browser-based workspace for durable LaTeX projects and manual server-side compilation.

## Quick start

On the server:

```sh
./install.sh
latex-core user create alice@example.com
latex-core status
```

Open the URL printed by `latex-core url`, then sign in with the credentials displayed when the account was created.

The compiler is a pinned, server-side TeX Live environment. Browser clients never need Docker or a local TeX installation.

- [Server setup](docs/SERVER_SETUP.md)
- [Client guide](docs/CLIENT_GUIDE.md)
- [Template library](docs/TEMPLATES.md)
- [CLI reference](docs/CLI.md)

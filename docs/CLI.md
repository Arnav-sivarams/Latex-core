# CLI reference

`latex-core --help` prints the short command summary. Normal operations do not require Docker commands.

| Command | Purpose |
| --- | --- |
| `start`, `stop`, `restart` | Manage the LaTeX Core service. |
| `status`, `doctor`, `url` | Show product health or the service URL. |
| `logs [api\|worker\|database]` | Read recent service logs. |
| `backup DIRECTORY` | Create database and blob backups. |
| `user create EMAIL [--password PASSWORD]` | Provision an account; a generated password is printed once when omitted. |
| `user list` | List accounts and enabled state. |
| `user disable EMAIL`, `user enable EMAIL` | Change account access. |
| `user reset-password EMAIL [--password PASSWORD]` | Set or generate a replacement password. |
| `template add ZIP --name NAME [--description TEXT] [--main PATH]` | Add a safe reusable template. |
| `template list`, `template remove NAME` | View or remove templates. |
| `config` | Print non-secret runtime configuration. |
| `version` | Print the CLI version. |

The `config` command deliberately omits database passwords, session secrets, and other credentials.

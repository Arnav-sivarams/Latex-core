# Future operator service

The web API intentionally does not execute host commands, mount a Docker socket,
or accept shell strings. Host lifecycle work remains an operator responsibility.

A future local operator daemon will accept authenticated, typed requests from the
API over a narrowly configured local channel. Its allow-list will contain only
explicit operations such as `status`, `restart`, and `backup`; it will never
provide arbitrary command execution. The daemon will own host credentials and
redact secrets from every response.

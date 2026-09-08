#!/usr/bin/env python3
"""Validate the supported deployment .env without executing it as shell code."""

from __future__ import annotations

import base64
import os
import re
import stat
import sys
from pathlib import Path
from urllib.parse import unquote, urlsplit

EXPECTED_IMAGE = "sha256:8db804f76b8e80e5be9fb28ba14b0938df5989b7a8250ca6b0e9f3c200c4ee38"
EXPECTED_TEX_ENV = "texlive-2026-sha256-364cea85dc8ba5e2a5131f1d8088142d08c9733d24d2644880d6eae86f751d5a"
KEY = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
PROJECT = re.compile(r"^[a-z0-9][a-z0-9_-]{1,62}$")
DB_IDENTIFIER = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
PLACEHOLDER = re.compile(r"replace[-_ ]with|example\.edu|change[-_ ]me", re.IGNORECASE)


class ConfigError(ValueError):
    pass


def dotenv(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    for number, original in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = original.strip()
        if not line or line.startswith("#"):
            continue
        if "=" not in line:
            raise ConfigError(f"line {number} is not KEY=VALUE syntax")
        key, raw = line.split("=", 1)
        key = key.strip()
        if not KEY.fullmatch(key):
            raise ConfigError(f"line {number} has invalid variable name")
        if key in values:
            raise ConfigError(f"{key} is defined more than once")
        raw = raw.strip()
        if raw.startswith("'"):
            if len(raw) < 2 or not raw.endswith("'"):
                raise ConfigError(f"{key} has an unterminated single-quoted value")
            value = raw[1:-1].replace("\\'", "'")
        elif raw.startswith('"'):
            if len(raw) < 2 or not raw.endswith('"'):
                raise ConfigError(f"{key} has an unterminated double-quoted value")
            try:
                value = bytes(raw[1:-1], "utf-8").decode("unicode_escape")
            except UnicodeDecodeError as error:
                raise ConfigError(f"{key} has an invalid quoted escape") from error
        else:
            value = re.split(r"\s+#", raw, maxsplit=1)[0].rstrip()
        values[key] = value
    return values


def require(values: dict[str, str], name: str) -> str:
    value = values.get(name, "")
    if not value:
        raise ConfigError(f"required variable {name} is missing or empty")
    if PLACEHOLDER.search(value):
        raise ConfigError(f"required variable {name} still contains a placeholder")
    return value


def boolean(values: dict[str, str], name: str) -> bool:
    value = require(values, name)
    if value not in {"true", "false"}:
        raise ConfigError(f"{name} must be true or false (the application parser is case-sensitive)")
    return value == "true"


def integer(values: dict[str, str], name: str, low: int = 1, high: int | None = None) -> int:
    value = require(values, name)
    if not value.isascii() or not value.isdecimal():
        raise ConfigError(f"{name} must be a base-10 integer")
    parsed = int(value)
    if parsed < low or (high is not None and parsed > high):
        suffix = f" through {high}" if high is not None else " or greater"
        raise ConfigError(f"{name} must be {low}{suffix}")
    return parsed


def validate_database(values: dict[str, str]) -> None:
    user = require(values, "POSTGRES_USER")
    database = require(values, "POSTGRES_DB")
    password = require(values, "POSTGRES_PASSWORD")
    if not DB_IDENTIFIER.fullmatch(user):
        raise ConfigError("POSTGRES_USER must be a simple PostgreSQL identifier")
    if not DB_IDENTIFIER.fullmatch(database):
        raise ConfigError("POSTGRES_DB must be a simple PostgreSQL identifier")
    parsed = urlsplit(require(values, "DATABASE_URL"))
    try:
        url_port = parsed.port
    except ValueError as error:
        raise ConfigError("DATABASE_URL has an invalid port") from error
    if parsed.scheme not in {"postgres", "postgresql"}:
        raise ConfigError("DATABASE_URL must use postgresql://")
    if parsed.hostname != "postgres" or url_port != 5432:
        raise ConfigError("DATABASE_URL must target postgres:5432 on the Compose network")
    if unquote(parsed.username or "") != user or unquote(parsed.password or "") != password:
        raise ConfigError("DATABASE_URL credentials do not match POSTGRES_USER/POSTGRES_PASSWORD; URL-encode reserved characters")
    if unquote(parsed.path.lstrip("/")) != database or parsed.query or parsed.fragment:
        raise ConfigError("DATABASE_URL database does not match POSTGRES_DB")


def validate_mail(values: dict[str, str]) -> None:
    enabled = boolean(values, "LATEX_CORE_MAIL_ENABLED")
    if not enabled:
        return
    host = require(values, "LATEX_CORE_SMTP_HOST")
    if any(character.isspace() for character in host):
        raise ConfigError("LATEX_CORE_SMTP_HOST must not contain whitespace")
    integer(values, "LATEX_CORE_SMTP_PORT", 1, 65535)
    if require(values, "LATEX_CORE_SMTP_SECURITY").lower() not in {"starttls", "tls"}:
        raise ConfigError("LATEX_CORE_SMTP_SECURITY must be starttls or tls")
    username = values.get("LATEX_CORE_SMTP_USERNAME", "")
    password = values.get("LATEX_CORE_SMTP_PASSWORD", "")
    if bool(username) != bool(password):
        raise ConfigError("LATEX_CORE_SMTP_USERNAME and LATEX_CORE_SMTP_PASSWORD must be set together")
    if "@" not in require(values, "LATEX_CORE_SMTP_FROM_EMAIL"):
        raise ConfigError("LATEX_CORE_SMTP_FROM_EMAIL is invalid")
    key = require(values, "LATEX_CORE_MAIL_SECRET_KEY")
    try:
        decoded = base64.b64decode(key, validate=True)
    except ValueError as error:
        raise ConfigError("LATEX_CORE_MAIL_SECRET_KEY must be standard base64") from error
    if len(decoded) != 32:
        raise ConfigError("LATEX_CORE_MAIL_SECRET_KEY must encode exactly 32 bytes")
    integer(values, "LATEX_CORE_MAIL_SECRET_LIFETIME_HOURS", 1, 168)
    integer(values, "LATEX_CORE_MAIL_BATCH_SIZE", 1, 100)
    integer(values, "LATEX_CORE_MAIL_MAX_ATTEMPTS", 1, 20)


def validate(path: Path) -> None:
    if not path.is_file():
        raise ConfigError(f"environment file does not exist: {path}")
    mode = stat.S_IMODE(path.stat().st_mode)
    if mode & 0o077:
        raise ConfigError(f"environment file permissions are {mode:04o}; run chmod 600 {path}")
    values = dotenv(path)
    managed = {
        key for key in values if key.startswith(("LATEX_CORE_", "QUEUE_", "WORKER_", "POSTGRES_"))
    } | {"DATABASE_URL", "HTTP_PORT", "HTTP_BIND_ADDRESS", "COMPOSE_PROJECT_NAME", "SESSION_COOKIE_SECURE", "SESSION_TTL_SECONDS", "ALLOW_REGISTRATION", "COMPILER_IMAGE", "TEX_ENVIRONMENT_ID"}
    conflicts = sorted(key for key in managed if key in os.environ and os.environ[key] != values[key])
    if conflicts:
        raise ConfigError("exported configuration conflicts with .env: " + ", ".join(conflicts) + "; unset those variables and rerun")
    if not PROJECT.fullmatch(require(values, "COMPOSE_PROJECT_NAME")):
        raise ConfigError("COMPOSE_PROJECT_NAME must use 2-63 lowercase letters, numbers, dashes, or underscores")
    integer(values, "HTTP_PORT", 1, 65535)
    integer(values, "LATEX_CORE_POSTGRES_PORT", 1, 65535)
    bind = require(values, "HTTP_BIND_ADDRESS")
    if bind not in {"127.0.0.1", "0.0.0.0"}:
        raise ConfigError("HTTP_BIND_ADDRESS must be 127.0.0.1 or 0.0.0.0 for this release")
    validate_database(values)
    boolean(values, "SESSION_COOKIE_SECURE")
    integer(values, "SESSION_TTL_SECONDS")
    boolean(values, "ALLOW_REGISTRATION")
    if require(values, "COMPILER_IMAGE") != EXPECTED_IMAGE:
        raise ConfigError("COMPILER_IMAGE must retain the frozen M7 sha256 identity")
    if require(values, "TEX_ENVIRONMENT_ID") != EXPECTED_TEX_ENV:
        raise ConfigError("TEX_ENVIRONMENT_ID does not identify the frozen M7 environment")
    staging = Path(require(values, "WORKER_STAGING_HOST_ROOT"))
    if not staging.is_absolute() or staging == Path("/") or ".." in staging.parts or "," in str(staging):
        raise ConfigError("WORKER_STAGING_HOST_ROOT must be a dedicated absolute path without '..' or commas")
    if "latex-core" not in str(staging):
        raise ConfigError("WORKER_STAGING_HOST_ROOT must identify a dedicated latex-core path")
    for name in ("WORKER_CONCURRENCY", "QUEUE_GLOBAL_RUNNING", "QUEUE_PER_USER_RUNNING", "QUEUE_PER_USER_OUTSTANDING", "QUEUE_LEASE_SECONDS", "QUEUE_MAX_ATTEMPTS"):
        integer(values, name)
    public_url = urlsplit(require(values, "LATEX_CORE_PUBLIC_BASE_URL"))
    if public_url.scheme not in {"http", "https"} or not public_url.hostname or public_url.username or public_url.password:
        raise ConfigError("LATEX_CORE_PUBLIC_BASE_URL must be an http(s) origin without credentials")
    secure = values["SESSION_COOKIE_SECURE"] == "true"
    if (public_url.scheme == "https") != secure:
        raise ConfigError("SESSION_COOKIE_SECURE must be true for https and false for http access")
    if public_url.hostname in {"localhost", "127.0.0.1"} and public_url.scheme == "http":
        public_port = public_url.port or 80
        if public_port != int(values["HTTP_PORT"]):
            raise ConfigError("LATEX_CORE_PUBLIC_BASE_URL port must match HTTP_PORT for local access")
    validate_mail(values)


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: validate-install-config.py ENV_FILE", file=sys.stderr)
        return 2
    try:
        validate(Path(sys.argv[1]).resolve())
    except (ConfigError, OSError, UnicodeError, ValueError) as error:
        print(f"Configuration error: {error}", file=sys.stderr)
        return 1
    print("Configuration is valid for the supported Compose deployment.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

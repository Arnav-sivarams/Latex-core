#!/usr/bin/env bash
set -euo pipefail

mode="${LATEX_CORE_DOCKER_MODE:-}"
docker_bin="${LATEX_CORE_DOCKER_BIN:-}"

[[ "$mode" == direct || "$mode" == sudo ]] || {
  echo 'Internal error: Docker execution mode was not initialized.' >&2
  exit 70
}
[[ "$docker_bin" == /* && -x "$docker_bin" ]] || {
  echo 'Internal error: Docker executable is missing or untrusted.' >&2
  exit 70
}

if [[ "$mode" == direct ]]; then
  exec "$docker_bin" "$@"
fi

sudo_bin=/usr/bin/sudo
if [[ "${LATEX_CORE_INSTALL_TESTING:-}" == 1 ]]; then
  sudo_bin="${LATEX_CORE_TEST_SUDO_BIN:-$sudo_bin}"
fi
[[ -x "$sudo_bin" ]] || { echo 'sudo is required for Docker access.' >&2; exit 1; }

if ! "$sudo_bin" -n -- "$docker_bin" info >/dev/null 2>&1; then
  tty_path=/dev/tty
  if [[ "${LATEX_CORE_INSTALL_TESTING:-}" == 1 ]]; then
    tty_path="${LATEX_CORE_TEST_TTY_PATH:-$tty_path}"
  fi
  if ! exec 3<>"$tty_path"; then
    echo 'Docker requires sudo authorization, but no controlling terminal is available. Run this command from an interactive terminal.' >&2
    exit 1
  fi
  printf 'LaTeX Core needs sudo authorization to access the local Docker daemon.\n' >&3
  "$sudo_bin" -v <&3 >&3
  exec 3>&-
fi
exec "$sudo_bin" -n -- "$docker_bin" "$@"

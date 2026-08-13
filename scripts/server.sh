#!/usr/bin/env bash
set -euo pipefail
compose=(docker compose -f deploy/compose/docker-compose.yml)
case "${1:-}" in
  start) "${compose[@]}" up -d --build ;;
  stop) "${compose[@]}" down ;;
  status) "${compose[@]}" ps ;;
  doctor) "${compose[@]}" run --rm worker /usr/local/bin/latex-core-doctor ;;
  logs) "${compose[@]}" logs --tail="${LOG_TAIL:-200}" "${@:2}" ;;
  backup) "$(dirname "$0")/backup-release.sh" "${2:?usage: server.sh backup DIRECTORY}" ;;
  *) echo 'usage: server.sh {start|stop|status|doctor|logs|backup DIRECTORY}' >&2; exit 2 ;;
esac

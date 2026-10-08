#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd -P)"
# shellcheck source=scripts/install-common.sh
source "$root/scripts/install-common.sh"
latex_core_init "$root"
staging="$(latex_core_env_value WORKER_STAGING_HOST_ROOT "$LATEX_CORE_ENV_FILE")"
worker_id="$(latex_core_container_id worker)"
[[ -n "$worker_id" && -d "$staging" && -w "$staging" ]] || {
  echo 'Deployment account cannot access the configured worker staging path.' >&2
  exit 1
}
mounts="$(latex_core_docker inspect -f '{{json .Mounts}}' "$worker_id")"
mount_kind="$(python3 -c '
import json, sys
mounts = [mount for mount in json.loads(sys.argv[1]) if mount.get("Destination") == sys.argv[2]]
if len(mounts) != 1 or mounts[0].get("Type") != "bind" or mounts[0].get("RW") is not True:
    sys.exit("Worker staging must be a writable bind at the configured destination.")
source = mounts[0].get("Source", "")
if source == sys.argv[2]:
    print("exact")
elif source.startswith("/run/desktop/mnt/host/wsl/docker-desktop-bind-mounts/"):
    print("desktop")
else:
    sys.exit("Worker staging bind source differs from the configured host path.")
' "$mounts" "$staging")"
if [[ "$mount_kind" == desktop ]]; then
  [[ "$(latex_core_docker info --format '{{.OperatingSystem}}')" == 'Docker Desktop' ]] || {
    echo 'Unexpected Docker Desktop staging path on a different daemon.' >&2
    exit 1
  }
  # Docker Desktop translates WSL bind sources. Prove this is the configured
  # directory rather than accepting an arbitrary rewritten mount path.
  probe="$(mktemp "$staging/.latex-core-mount-check.XXXXXX")"
  trap 'rm -f -- "$probe"' EXIT
  nonce="${probe##*/}-$RANDOM-$RANDOM"
  printf '%s' "$nonce" > "$probe"
  # The positional parameters expand inside the worker, not on the host.
  # shellcheck disable=SC2016
  "${LATEX_CORE_COMPOSE[@]}" exec -T worker sh -ceu \
    'test "$(cat -- "$1")" = "$2"' sh "$probe" "$nonce"
fi

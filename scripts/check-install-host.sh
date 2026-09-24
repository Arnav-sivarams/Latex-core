#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=scripts/install-common.sh
source "$root/scripts/install-common.sh"
for command in git awk sed grep mktemp chmod mv python3 curl ss df sha256sum sha384sum; do
  command -v "$command" >/dev/null 2>&1 || { echo "Unsupported host: required utility '$command' is missing." >&2; exit 1; }
done
[[ "$(uname -s)" == Linux ]] || { echo 'Unsupported host: this server release supports Linux only.' >&2; exit 1; }
[[ "$(uname -m)" == x86_64 ]] || { echo "Unsupported host architecture: $(uname -m); frozen M7 requires x86_64/amd64." >&2; exit 1; }
os_release_file="${LATEX_CORE_OS_RELEASE_FILE:-/etc/os-release}"
if [[ -r "$os_release_file" ]]; then
  # shellcheck disable=SC1090
  source "$os_release_file"
  [[ "${ID:-}" == ubuntu && "${VERSION_ID:-}" =~ ^(22\.04|24\.04)$ ]] || {
    echo "Unsupported server distribution: ${ID:-unknown} ${VERSION_ID:-unknown}; supported releases are Ubuntu 22.04 LTS and Ubuntu 24.04 LTS." >&2
    exit 1
  }
fi
latex_core_select_docker "$root"
latex_core_docker info >/dev/null
latex_core_docker compose version >/dev/null 2>&1 || { echo 'Docker Compose v2 is required.' >&2; exit 1; }
buildx_output="$(latex_core_docker buildx version 2>/dev/null || true)"
buildx_version="$(sed -nE 's/.* v?([0-9]+)\.([0-9]+).*/\1 \2/p' <<<"$buildx_output" | head -n1)"
buildx_major="${buildx_version%% *}"; buildx_minor="${buildx_version##* }"
if [[ ! "$buildx_major" =~ ^[0-9]+$ || ! "$buildx_minor" =~ ^[0-9]+$ ]] ||
   ((buildx_major < 1 && buildx_minor < 12)); then
  echo 'Docker Buildx 0.12 or newer is required.' >&2
  exit 1
fi
engine_version="$(latex_core_docker version --format '{{.Server.Version}}')"
engine_major="${engine_version%%.*}"
[[ "$engine_major" =~ ^[0-9]+$ && "$engine_major" -ge 24 ]] || { echo "Unsupported Docker Engine $engine_version; version 24 or newer is required." >&2; exit 1; }
compose_version="$(latex_core_docker compose version --short)"; compose_version="${compose_version#v}"
compose_major="${compose_version%%.*}"; compose_rest="${compose_version#*.}"; compose_minor="${compose_rest%%.*}"
[[ "$compose_major" =~ ^[0-9]+$ && "$compose_minor" =~ ^[0-9]+$ ]] || { echo "Unable to parse Docker Compose version $compose_version." >&2; exit 1; }
if ((compose_major < 2 || (compose_major == 2 && compose_minor < 20))); then
  echo "Unsupported Docker Compose $compose_version; version 2.20 or newer is required." >&2
  exit 1
fi
context="$(latex_core_docker context show)"
endpoint="$(latex_core_docker context inspect "$context" --format '{{.Endpoints.docker.Host}}')"
[[ "$endpoint" == unix:///var/run/docker.sock ]] || {
  echo "Unsupported Docker context '$context': Worker requires the local unix:///var/run/docker.sock daemon." >&2
  exit 1
}
server_os="$(latex_core_docker info --format '{{.OSType}}')"
server_arch="$(latex_core_docker info --format '{{.Architecture}}')"
[[ "$server_os" == linux && "$server_arch" =~ ^(x86_64|amd64)$ ]] || {
  echo "Unsupported Docker daemon platform: $server_os/$server_arch; linux/amd64 is required." >&2
  exit 1
}
if latex_core_docker info --format '{{json .SecurityOptions}}' | grep -qi rootless; then
  echo 'Unsupported Docker layout: rootless Docker does not provide the /var/run/docker.sock and host-bind contract required by this Worker.' >&2
  exit 1
fi
available_kb="$(df -Pk "$root" | awk 'NR==2 {print $4}')"
available_mem_kb="$(awk '/MemAvailable:/ {print $2}' /proc/meminfo)"
((available_kb >= 10 * 1024 * 1024)) || echo 'WARNING: less than 10 GiB is free on the source filesystem; image builds or application data may exhaust it.' >&2
((available_mem_kb >= 4 * 1024 * 1024)) || echo 'WARNING: less than 4 GiB memory is currently available; builds or TeX jobs may fail or be OOM-killed.' >&2
docker_root="$(latex_core_docker info --format '{{.DockerRootDir}}')"
if [[ -d "$docker_root" ]]; then
  docker_available_kb="$(df -Pk "$docker_root" | awk 'NR==2 {print $4}')"
  ((docker_available_kb >= 10 * 1024 * 1024)) || echo 'WARNING: less than 10 GiB is free on the Docker storage filesystem; image builds or volumes may exhaust it.' >&2
else
  echo "WARNING: Docker storage path $docker_root is not visible from this login environment; verify daemon disk capacity separately." >&2
fi
daemon_memory_bytes="$(latex_core_docker info --format '{{.MemTotal}}')"
((daemon_memory_bytes >= 4 * 1024 * 1024 * 1024)) || echo 'WARNING: Docker exposes less than 4 GiB memory; builds or TeX jobs may be OOM-killed.' >&2
printf 'Host contract: Ubuntu 22.04 or 24.04 x86_64, local Docker context %s, Engine %s, Compose %s.\n' "$context" "$engine_version" "$compose_version"

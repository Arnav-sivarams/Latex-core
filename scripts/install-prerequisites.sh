#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
# shellcheck source=scripts/install-common.sh
source "$root/scripts/install-common.sh"

os_release_file="${LATEX_CORE_OS_RELEASE_FILE:-/etc/os-release}"
os_value() {
  local wanted="$1" key value
  while IFS='=' read -r key value; do
    [[ "$key" == "$wanted" ]] || continue
    value="${value#\"}"; value="${value%\"}"
    value="${value#\'}"; value="${value%\'}"
    printf '%s\n' "$value"
    return
  done <"$os_release_file"
}
os_id="$(os_value ID)"
os_version="$(os_value VERSION_ID)"
codename="$(os_value VERSION_CODENAME)"
[[ "$os_id" == ubuntu && "$os_version" =~ ^(22\.04|24\.04)$ && "$codename" =~ ^(jammy|noble)$ ]] || {
  echo "Unsupported server distribution: ${os_id:-unknown} ${os_version:-unknown}; supported releases are Ubuntu 22.04 LTS and Ubuntu 24.04 LTS." >&2
  exit 1
}
[[ "$(uname -m)" == x86_64 ]] || { echo 'Only Ubuntu x86_64/amd64 is supported.' >&2; exit 1; }
if ((EUID == 0)) && [[ "${LATEX_CORE_INSTALL_TESTING:-}" != 1 ]]; then
  echo 'Run ./latex-core install from a normal sudo-capable account. sudo ./latex-core install is supported only when sudo provides a validated invoking identity.' >&2
  exit 1
fi

sudo_bin=/usr/bin/sudo
if [[ "${LATEX_CORE_INSTALL_TESTING:-}" == 1 ]]; then sudo_bin="${LATEX_CORE_TEST_SUDO_BIN:-$sudo_bin}"; fi
authorize_sudo() {
  local tty_path=/dev/tty
  [[ -x "$sudo_bin" ]] || { echo 'sudo is required to install or start host prerequisites.' >&2; return 1; }
  if "$sudo_bin" -n true 2>/dev/null; then return 0; fi
  if [[ "${LATEX_CORE_INSTALL_TESTING:-}" == 1 ]]; then
    tty_path="${LATEX_CORE_TEST_TTY_PATH:-$tty_path}"
  fi
  if ! exec 3<>"$tty_path"; then
    echo 'Host changes require sudo authorization, but no controlling terminal is available.' >&2
    return 1
  fi
  printf 'LaTeX Core needs sudo authorization for the listed host changes.\n' >&3
  local status=0
  "$sudo_bin" -v <&3 >&3 || status=$?
  exec 3>&-
  return "$status"
}
sudo_run() {
  authorize_sudo
  "$sudo_bin" -n -- "$@"
}
apt_install_exact() {
  sudo_run /usr/bin/apt-get -o DPkg::Lock::Timeout=120 install -y --no-install-recommends "$@"
}

declare -a missing_utilities=()
declare -A utility_packages=(
  [git]=git [awk]=mawk [sed]=sed [grep]=grep [mktemp]=coreutils [chmod]=coreutils
  [mv]=coreutils [python3]=python3 [curl]=curl [ss]=iproute2 [df]=coreutils
  [sha256sum]=coreutils [sha384sum]=coreutils [gpg]=gnupg [update-ca-certificates]=ca-certificates
)
for utility in "${!utility_packages[@]}"; do
  command -v "$utility" >/dev/null 2>&1 || missing_utilities+=("${utility_packages[$utility]}")
done
if ((${#missing_utilities[@]})); then
  mapfile -t missing_utilities < <(printf '%s\n' "${missing_utilities[@]}" | sort -u)
  printf 'LaTeX Core will install required Ubuntu utilities: %s\n' "${missing_utilities[*]}"
  authorize_sudo || { echo 'sudo authorization was denied or cancelled; no prerequisite installation was attempted.' >&2; exit 1; }
  sudo_run /usr/bin/apt-get -o DPkg::Lock::Timeout=120 update
  apt_install_exact "${missing_utilities[@]}"
fi

docker_bin="$(latex_core_resolve_executable docker || true)"
if [[ "${LATEX_CORE_INSTALL_TESTING:-}" == 1 && "${LATEX_CORE_FORCE_DOCKER_MISSING:-}" == 1 ]]; then docker_bin=''; fi
need_docker=false
if [[ -z "$docker_bin" ]]; then
  need_docker=true
else
  client_version="$($docker_bin version --format '{{.Client.Version}}' 2>/dev/null || true)"
  client_major="${client_version%%.*}"
  compose_version="$($docker_bin compose version --short 2>/dev/null || true)"
  compose_version="${compose_version#v}"
  compose_major="${compose_version%%.*}"
  compose_rest="${compose_version#*.}"
  compose_minor="${compose_rest%%.*}"
  buildx_version="$($docker_bin buildx version 2>/dev/null | sed -nE 's/.* v?([0-9]+)\.([0-9]+).*/\1 \2/p' | head -n1 || true)"
  buildx_major="${buildx_version%% *}"
  buildx_minor="${buildx_version##* }"
  if [[ ! "$client_major" =~ ^[0-9]+$ || "$client_major" -lt 24 || ! "$compose_major" =~ ^[0-9]+$ || ! "$compose_minor" =~ ^[0-9]+$ ]] ||
     ((compose_major < 2 || (compose_major == 2 && compose_minor < 20))) ||
     [[ ! "$buildx_major" =~ ^[0-9]+$ || ! "$buildx_minor" =~ ^[0-9]+$ ]] ||
     ((buildx_major < 1 && buildx_minor < 12)); then
    need_docker=true
  fi
fi

if [[ "$need_docker" == true ]]; then
  if [[ "${LATEX_CORE_PREREQUISITE_DRY_RUN:-}" == 1 ]]; then
    echo 'DRY RUN: Docker Engine/Compose/Buildx provisioning is required.'
    exit 0
  fi
  declare -a conflicts=()
  for package in docker.io docker-compose docker-compose-v2 podman-docker containerd runc; do
    if dpkg-query -W -f='${db:Status-Abbrev}' "$package" 2>/dev/null | grep -q '^ii '; then conflicts+=("$package"); fi
  done
  if ((${#conflicts[@]})); then
    echo "Docker prerequisites are missing or incompatible, but conflicting packages are installed: ${conflicts[*]}." >&2
    echo 'The installer will not remove or replace administrator-managed container packages automatically.' >&2
    exit 1
  fi
  printf 'LaTeX Core will add Docker\047s official signed Ubuntu repository and install Docker Engine, CLI, containerd, Buildx, and Compose v2.\n'
  authorize_sudo || { echo 'sudo authorization was denied or cancelled; Docker was not installed.' >&2; exit 1; }
  sudo_run /usr/bin/install -m 0755 -d /etc/apt/keyrings
  key_file="$(mktemp /tmp/latex-core-docker-key.XXXXXX)"
  trap 'rm -f -- "$key_file"' EXIT
  curl --fail --show-error --silent --location --max-time 60 https://download.docker.com/linux/ubuntu/gpg --output "$key_file"
  fingerprint="$(gpg --show-keys --with-colons "$key_file" | awk -F: '$1=="fpr" {print $10; exit}')"
  [[ "$fingerprint" == 9DC858229FC7DD38854AE2D88D81803C0EBFCD88 ]] || {
    echo "Docker repository signing-key fingerprint mismatch: ${fingerprint:-missing}." >&2
    exit 1
  }
  sudo_run /usr/bin/install -m 0644 "$key_file" /etc/apt/keyrings/docker.asc
  source_line="deb [arch=amd64 signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/ubuntu $codename stable"
  source_file="$(mktemp /tmp/latex-core-docker-source.XXXXXX)"
  printf '%s\n' "$source_line" >"$source_file"
  sudo_run /usr/bin/install -m 0644 "$source_file" /etc/apt/sources.list.d/docker.list
  rm -f -- "$source_file" "$key_file"
  trap - EXIT
  sudo_run /usr/bin/apt-get -o DPkg::Lock::Timeout=120 update
  declare -a docker_packages=(docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin)
  declare -a pinned_packages=()
  for package in "${docker_packages[@]}"; do
    candidate="$(apt-cache policy "$package" | awk '$1 == "Candidate:" { candidate=$2 } END { print candidate }')"
    [[ -n "$candidate" && "$candidate" != '(none)' ]] || { echo "No installable Docker package candidate for $package." >&2; exit 1; }
    pinned_packages+=("$package=$candidate")
  done
  printf 'Installing exact repository candidates: %s\n' "${pinned_packages[*]}"
  apt_install_exact "${pinned_packages[@]}"
  docker_bin="$(latex_core_resolve_executable docker)"
fi

if [[ -n "${DOCKER_HOST:-}" && "$DOCKER_HOST" != unix:///var/run/docker.sock ]]; then
  echo "Unsupported DOCKER_HOST '$DOCKER_HOST'. The installer will not switch to a different daemon." >&2
  exit 1
fi
context="$($docker_bin context show 2>/dev/null || true)"
endpoint="$($docker_bin context inspect "$context" --format '{{.Endpoints.docker.Host}}' 2>/dev/null || true)"
[[ "$endpoint" == unix:///var/run/docker.sock ]] || {
  echo "Unsupported Docker context '${context:-unknown}' (${endpoint:-unresolved}). The installer will not switch to a different daemon." >&2
  exit 1
}

daemon_ready=false
if "$docker_bin" info >/dev/null 2>&1; then
  daemon_ready=true
else
  authorize_sudo || { echo 'sudo authorization was denied or cancelled; Docker access was not changed.' >&2; exit 1; }
  if sudo_run "$docker_bin" info >/dev/null 2>&1; then daemon_ready=true; fi
fi
if [[ "$daemon_ready" != true ]]; then
  systemctl_bin=/usr/bin/systemctl
  if [[ "${LATEX_CORE_INSTALL_TESTING:-}" == 1 ]]; then systemctl_bin="${LATEX_CORE_TEST_SYSTEMCTL_BIN:-$systemctl_bin}"; fi
  [[ -x "$systemctl_bin" ]] || { echo 'The local Docker daemon is unreachable and systemctl is unavailable.' >&2; exit 1; }
  if [[ "$($systemctl_bin is-enabled docker 2>/dev/null || true)" == masked ]]; then
    echo 'Docker service is administratively masked; the installer will not unmask it.' >&2
    exit 1
  fi
  if "$systemctl_bin" is-active --quiet docker; then
    echo 'docker.service is running, but the local daemon is still unreachable with sudo. Check daemon logs and configuration.' >&2
    exit 1
  fi
  echo 'Docker is installed but stopped; starting docker.service.'
  sudo_run "$systemctl_bin" start docker
  for _ in {1..30}; do
    if sudo_run "$docker_bin" info >/dev/null 2>&1; then daemon_ready=true; break; fi
    sleep 1
  done
  [[ "$daemon_ready" == true ]] || { echo 'docker.service did not become ready within 30 seconds.' >&2; exit 1; }
fi

latex_core_select_docker "$root"
echo "Docker execution mode: $LATEX_CORE_DOCKER_MODE (permanent docker-group membership is not required)."

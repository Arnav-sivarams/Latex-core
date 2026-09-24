#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
test_root="$(mktemp -d /tmp/latex-core-privilege-test.XXXXXX)"
cleanup() { rm -rf -- "$test_root"; }
trap cleanup EXIT
mkdir -p "$test_root/bin" "$test_root/repo/scripts"
cp "$root/scripts/install-common.sh" "$root/scripts/docker-exec.sh" "$root/scripts/compose.sh" "$test_root/repo/scripts/"
chmod 755 "$test_root/repo/scripts/docker-exec.sh" "$test_root/repo/scripts/compose.sh"
printf 'COMPOSE_PROJECT_NAME=latex-core-privilege-test\n' >"$test_root/repo/.env"
printf 'ID=ubuntu\nVERSION_ID="24.04"\nVERSION_CODENAME=noble\n' >"$test_root/os-release"

cat >"$test_root/bin/docker" <<'EOF'
#!/usr/bin/env bash
printf '%s privileged=%s\n' "$*" "${FAKE_PRIVILEGED:-0}" >>"$FAKE_DOCKER_LOG"
case "$*" in
  'context show') echo default ;;
  'context inspect default --format {{.Endpoints.docker.Host}}') echo unix:///var/run/docker.sock ;;
  'version --format {{.Client.Version}}') echo 24.0.0 ;;
  'compose version --short') echo 2.20.0 ;;
  'buildx version') echo 'github.com/docker/buildx v0.12.0' ;;
  'info')
    if [[ "${FAKE_REQUIRE_DAEMON:-0}" == 1 && ! -f "$FAKE_DAEMON_MARKER" ]]; then exit 1; fi
    [[ "${FAKE_DIRECT:-0}" == 1 || "${FAKE_PRIVILEGED:-0}" == 1 ]]
    ;;
  'compose version') echo 'Docker Compose version v2.20.0' ;;
  *) [[ "${FAKE_PRIVILEGED:-0}" == 1 || "${FAKE_DIRECT:-0}" == 1 ]] ;;
esac
EOF
cat >"$test_root/bin/sudo" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$FAKE_SUDO_LOG"
if [[ "${1:-} ${2:-}" == '-n true' ]]; then [[ "${FAKE_SUDO_POLICY:-allow}" == allow ]]; exit; fi
if [[ "${1:-}" == -v ]]; then echo 'fake sudo auth diagnostic' >&2; exit "${FAKE_SUDO_AUTH_EXIT:-1}"; fi
[[ "${1:-}" == -n && "${2:-}" == -- ]] || exit 2
[[ "${FAKE_SUDO_POLICY:-allow}" == allow ]] || exit 1
shift 2
FAKE_PRIVILEGED=1 exec "$@"
EOF
cat >"$test_root/bin/systemctl" <<'EOF'
#!/usr/bin/env bash
case "$1" in
  is-enabled) echo enabled ;;
  is-active) [[ -f "$FAKE_DAEMON_MARKER" ]] ;;
  start) : >"$FAKE_DAEMON_MARKER" ;;
  *) exit 2 ;;
esac
EOF
chmod 755 "$test_root/bin/"*

export PATH="$test_root/bin:$PATH"
export LATEX_CORE_INSTALL_TESTING=1
export LATEX_CORE_TEST_SUDO_BIN="$test_root/bin/sudo"
export LATEX_CORE_TEST_SYSTEMCTL_BIN="$test_root/bin/systemctl"
export LATEX_CORE_OS_RELEASE_FILE="$test_root/os-release"
export FAKE_DOCKER_LOG="$test_root/docker.log"
export FAKE_SUDO_LOG="$test_root/sudo.log"
export FAKE_DAEMON_MARKER="$test_root/daemon-ready"

# Invoking-user recovery accepts the checkout owner and rejects inconsistent
# sudo identity data without changing ownership.
source "$root/scripts/install-common.sh"
current_identity="$(latex_core_validate_invoking_identity "$root" "$(id -u)" "$(id -g)" "$(id -un)")"
[[ "$current_identity" == "$(id -un):"* ]]
if latex_core_validate_invoking_identity "$root" "$(id -u)" "$(id -g)" definitely-not-this-user >/dev/null; then
  echo 'inconsistent invoking identity unexpectedly passed' >&2
  exit 1
fi

# Direct access never invokes sudo.
: >"$FAKE_DOCKER_LOG"; : >"$FAKE_SUDO_LOG"
FAKE_DIRECT=1 LATEX_CORE_DOCKER_MODE='' LATEX_CORE_DOCKER_BIN='' bash -ceu '
  source "$1/scripts/install-common.sh"
  latex_core_select_docker "$1"
  [[ "$LATEX_CORE_DOCKER_MODE" == direct ]]
  latex_core_docker ps >/dev/null
' _ "$test_root/repo"
[[ ! -s "$FAKE_SUDO_LOG" ]]

# Denied direct socket access selects one shared sudo executor, including in a
# separately executed Compose helper.
: >"$FAKE_DOCKER_LOG"; : >"$FAKE_SUDO_LOG"
FAKE_DIRECT=0 FAKE_SUDO_POLICY=allow LATEX_CORE_DOCKER_MODE='' LATEX_CORE_DOCKER_BIN='' \
  "$test_root/repo/scripts/compose.sh" ps >/dev/null
grep -Fq -- '-n --' "$FAKE_SUDO_LOG"
grep -Fq 'compose --project-name latex-core-privilege-test' "$FAKE_DOCKER_LOG"

# A sudo-assisted Docker call does not consume application password stdin.
preserved="$(printf 'admin-password-line\n' | FAKE_DIRECT=0 FAKE_SUDO_POLICY=allow \
  LATEX_CORE_DOCKER_MODE=sudo LATEX_CORE_DOCKER_BIN="$test_root/bin/docker" \
  bash -ceu '"$1/scripts/docker-exec.sh" info >/dev/null; IFS= read -r line; printf "%s" "$line"' _ "$test_root/repo")"
[[ "$preserved" == admin-password-line ]]

# Missing Engine or missing Compose/Buildx is recognized as provisioning work.
LATEX_CORE_PREREQUISITE_DRY_RUN=1 LATEX_CORE_FORCE_DOCKER_MISSING=1 LATEX_CORE_OS_RELEASE_FILE="$test_root/os-release" \
  "$root/scripts/install-prerequisites.sh" | grep -Fq 'provisioning is required'
cat >"$test_root/bin/docker-missing-compose" <<'EOF'
#!/usr/bin/env bash
case "$*" in
  'version --format {{.Client.Version}}') echo 24.0.0 ;;
  'compose version --short'|'buildx version') exit 1 ;;
  *) exit 1 ;;
esac
EOF
chmod 755 "$test_root/bin/docker-missing-compose"
mkdir "$test_root/missing-compose-bin"
ln -s "$test_root/bin/docker-missing-compose" "$test_root/missing-compose-bin/docker"
LATEX_CORE_PREREQUISITE_DRY_RUN=1 PATH="$test_root/missing-compose-bin:$PATH" \
  LATEX_CORE_OS_RELEASE_FILE="$test_root/os-release" "$root/scripts/install-prerequisites.sh" | grep -Fq 'provisioning is required'

# A stopped daemon is started and used during the same prerequisite invocation.
rm -f "$FAKE_DAEMON_MARKER"
FAKE_DIRECT=0 FAKE_SUDO_POLICY=allow FAKE_REQUIRE_DAEMON=1 LATEX_CORE_DOCKER_MODE='' LATEX_CORE_DOCKER_BIN='' \
  "$root/scripts/install-prerequisites.sh" >/dev/null
[[ -f "$FAKE_DAEMON_MARKER" ]]

# Without an authorized sudo policy, access fails honestly.
if FAKE_DIRECT=0 FAKE_SUDO_POLICY=deny LATEX_CORE_DOCKER_MODE=sudo LATEX_CORE_DOCKER_BIN="$test_root/bin/docker" \
    "$test_root/repo/scripts/docker-exec.sh" info </dev/null >"$test_root/out" 2>"$test_root/err"; then
  echo 'denied sudo policy unexpectedly succeeded' >&2
  exit 1
fi
grep -Eq 'sudo authorization|controlling terminal' "$test_root/err"

# A cancelled interactive authorization is also a hard failure; it neither
# falls back to direct Docker nor consumes application stdin.
: >"$test_root/tty"
: >"$FAKE_SUDO_LOG"
if FAKE_DIRECT=0 FAKE_SUDO_POLICY=deny FAKE_SUDO_AUTH_EXIT=130 \
    LATEX_CORE_TEST_TTY_PATH="$test_root/tty" LATEX_CORE_DOCKER_MODE=sudo \
    LATEX_CORE_DOCKER_BIN="$test_root/bin/docker" \
    "$test_root/repo/scripts/docker-exec.sh" info >"$test_root/out" 2>"$test_root/err"; then
  echo 'cancelled sudo authorization unexpectedly succeeded' >&2
  exit 1
fi
grep -Fq -- '-v' "$FAKE_SUDO_LOG"
grep -Fq 'fake sudo auth diagnostic' "$test_root/err"

echo 'Installer Docker privilege tests passed.'

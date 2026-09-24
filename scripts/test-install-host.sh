#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
test_root="$(mktemp -d /tmp/latex-core-host-test.XXXXXX)"
cleanup() { rm -rf -- "$test_root"; }
trap cleanup EXIT
mkdir -p "$test_root/bin" "$test_root/os-release"

cat >"$test_root/bin/docker" <<'EOF'
#!/usr/bin/env bash
case "$*" in
  'info') exit 0 ;;
  'compose version') echo 'Docker Compose version v2.20.0' ;;
  'compose version --short') echo '2.20.0' ;;
  'buildx version') echo 'github.com/docker/buildx v0.12.0' ;;
  'version --format {{.Server.Version}}') echo '24.0.0' ;;
  'context show') echo 'default' ;;
  'context inspect default --format {{.Endpoints.docker.Host}}') echo 'unix:///var/run/docker.sock' ;;
  'info --format {{.OSType}}') echo 'linux' ;;
  'info --format {{.Architecture}}') echo 'x86_64' ;;
  'info --format {{json .SecurityOptions}}') echo '[]' ;;
  'info --format {{.DockerRootDir}}') echo "$TEST_DOCKER_ROOT" ;;
  'info --format {{.MemTotal}}') echo '8589934592' ;;
  *) echo "unexpected docker invocation: $*" >&2; exit 1 ;;
esac
EOF
chmod +x "$test_root/bin/docker"

run_contract() {
  local name="$1" id="$2" version="$3" expected="$4" release
  release="$test_root/os-release/$name"
  printf 'ID=%s\nVERSION_ID="%s"\n' "$id" "$version" >"$release"
  if PATH="$test_root/bin:$PATH" TEST_DOCKER_ROOT="$test_root" LATEX_CORE_OS_RELEASE_FILE="$release" \
      "$root/scripts/check-install-host.sh" >"$test_root/$name.out" 2>"$test_root/$name.err"; then
    [[ "$expected" == pass ]] || { echo "$name unexpectedly passed" >&2; exit 1; }
  else
    [[ "$expected" == fail ]] || { cat "$test_root/$name.err" >&2; exit 1; }
    grep -Fq 'supported releases are Ubuntu 22.04 LTS and Ubuntu 24.04 LTS' "$test_root/$name.err"
  fi
}

run_contract ubuntu-22 ubuntu 22.04 pass
run_contract ubuntu-24 ubuntu 24.04 pass
run_contract ubuntu-20 ubuntu 20.04 fail
run_contract debian-12 debian 12 fail
echo 'Install host release contract tests passed.'

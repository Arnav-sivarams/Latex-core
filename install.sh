#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
cd "$root"
# shellcheck source=scripts/install-common.sh
source "$root/scripts/install-common.sh"
local_image='latex-core-texlive:2026-m7'
default_source='ghcr.io/arnav-sivarams/latex-core-texlive:2026-m7'
expected='sha256:8db804f76b8e80e5be9fb28ba14b0938df5989b7a8250ca6b0e9f3c200c4ee38'
skip_admin=false
configure_mail=false
verify_only=false
admin_email=''
admin_password_stdin=false
phase=argument-validation
install_log=''

usage() {
  cat <<'EOF'
Usage: ./latex-core install [--verify-only|--skip-admin|--configure-mail]
       ./latex-core install --admin-email EMAIL --admin-password-stdin

  --verify-only     Run installation checks without applying migrations or restarting services.
  --skip-admin      Do not prompt to create the first Admin.
  --configure-mail  Configure SMTP interactively during installation.
  --admin-email      First administrator's email (requires --admin-password-stdin).
  --admin-password-stdin  Read the first administrator's password from standard input.
EOF
}
while (($#)); do
  case "$1" in
    --help|-h) usage; exit 0 ;;
    --verify-only) verify_only=true ;;
    --skip-admin) skip_admin=true ;;
    --configure-mail) configure_mail=true ;;
    --admin-email) [[ $# -ge 2 ]] || { usage >&2; exit 2; }; admin_email="$2"; shift ;;
    --admin-password-stdin) admin_password_stdin=true ;;
    *) usage >&2; exit 2 ;;
  esac
  shift
done
if [[ "$admin_password_stdin" == true ]]; then
  [[ -n "$admin_email" && "$skip_admin" == false ]] || { echo '--admin-password-stdin requires --admin-email and cannot be combined with --skip-admin.' >&2; exit 2; }
  IFS= read -r admin_password || { echo 'No administrator password received on standard input.' >&2; exit 2; }
elif [[ -n "$admin_email" ]]; then
  echo '--admin-email requires --admin-password-stdin.' >&2
  exit 2
fi
if [[ "$verify_only" == true ]]; then exec "$root/scripts/verify-install.sh"; fi

on_exit() {
  local status=$?
  trap - EXIT
  if ((status != 0)); then
    echo "Installation failed during phase: $phase (exit $status)." >&2
    if [[ -n "$install_log" && -f "$install_log" ]]; then
      echo "Last installation output (full log: $install_log):" >&2
      tail -n 35 "$install_log" >&2
    fi
    echo 'Correct the named failure and rerun ./latex-core install. Existing .env, accounts, blobs, and database volumes were not reset.' >&2
  fi
  exit "$status"
}
trap on_exit EXIT

phase=host-contract
"$root/scripts/install-prerequisites.sh"
"$root/scripts/check-install-host.sh"

for required_file in .env.example deploy/compose/docker-compose.yml deploy/Dockerfile migrations/0001_initial_schema.sql; do
  [[ -f "$root/$required_file" ]] || { echo "Repository is incomplete: missing $required_file. Restore the checkout and rerun ./latex-core install." >&2; exit 1; }
done
echo '[1/8] Checking environment          OK'

phase=environment-configuration
if [[ ! -f "$root/.env" ]]; then
  "$root/scripts/generate-local-env.sh"
  echo 'Basic access is bound to 127.0.0.1 for local use or an SSH tunnel; SMTP remains disabled.'
else
  echo 'Existing .env preserved.'
fi
latex_core_init "$root"
python3 "$root/scripts/validate-install-config.py" "$LATEX_CORE_ENV_FILE"

if [[ "$configure_mail" == true ]]; then
  "$root/scripts/configure-smtp.sh"
elif [[ -t 0 ]]; then
  read -r -p 'Configure SMTP password email delivery now? [y/N] ' answer
  case "$answer" in y|Y|yes|YES) "$root/scripts/configure-smtp.sh" ;; esac
fi
python3 "$root/scripts/validate-install-config.py" "$LATEX_CORE_ENV_FILE"
"${LATEX_CORE_COMPOSE[@]}" config --quiet
"$root/scripts/check-install-ports.sh"
echo '[2/8] Preparing configuration      OK'

phase=worker-staging
latex_core_prepare_staging
echo '[3/8] Preparing directories        OK'

phase=compiler-image-verification
actual="$(latex_core_docker image inspect "$local_image" --format '{{.Id}}' 2>/dev/null || true)"
if [[ -z "$actual" ]]; then
  source_image="$(latex_core_env_value LATEX_CORE_M7_IMAGE_SOURCE "$LATEX_CORE_ENV_FILE")"
  source_image="${source_image:-$default_source}"
  echo "Frozen M7 image is missing; pulling published source $source_image ..."
  latex_core_docker pull "$source_image"
  pulled="$(latex_core_docker image inspect "$source_image" --format '{{.Id}}')"
  [[ "$pulled" == "$expected" ]] || { echo "Pulled M7 image identity mismatch: expected $expected, got $pulled" >&2; exit 1; }
  latex_core_docker tag "$source_image" "$local_image"
  actual="$(latex_core_docker image inspect "$local_image" --format '{{.Id}}')"
fi
platform="$(latex_core_docker image inspect "$local_image" --format '{{.Os}}/{{.Architecture}}')"
[[ "$actual" == "$expected" ]] || { echo "Frozen M7 image identity mismatch: expected $expected, got $actual" >&2; exit 1; }
[[ "$platform" == linux/amd64 ]] || { echo "Frozen M7 image platform mismatch: expected linux/amd64, got $platform" >&2; exit 1; }
echo 'Frozen M7 compiler image identity and platform verified.'

phase=service-startup
mkdir -p "$root/.install-diagnostics"
umask 077
install_log="$(mktemp "$root/.install-diagnostics/install.XXXXXX.log")"
echo 'Starting database, migrating schema, building and starting services (this may take several minutes).'
if ! "$root/scripts/start-install-services.sh" >"$install_log" 2>&1; then
  echo 'Service startup failed; see installation diagnostics below.' >&2
  exit 1
fi
echo '[4/8] Starting database            OK'
echo '[5/8] Running migrations           OK'
echo '[6/8] Building application         OK'
echo '[7/8] Starting services            OK'

phase=admin-bootstrap
if [[ "$skip_admin" != true ]]; then
  user_list="$("$root/latex-core" user list)"
  if grep -q 'v2=admin' <<<"$user_list"; then
    echo 'Administrator account already exists. Skipping bootstrap.'
  elif [[ "$admin_password_stdin" == true ]]; then
    printf '%s\n' "$admin_password" | "$root/latex-core" admin create --email "$admin_email" --password-stdin
    unset admin_password
  elif [[ -t 0 ]]; then
    read -r -p 'No administrator account exists. Create the first administrator now? [Y/n] ' answer
    case "$answer" in n|N|no|NO) echo 'Administrator bootstrap skipped.' ;; *) "$root/latex-core" admin create ;; esac
  else
    echo 'No administrator exists and input is non-interactive. Rerun ./latex-core install in a terminal or use --skip-admin and then ./latex-core admin create.' >&2
    exit 1
  fi
fi

phase=final-verification
"$root/scripts/verify-install.sh"
echo '[8/8] Health check                 OK'
url="$("$root/latex-core" url)"
mail_state=disabled
[[ "$(latex_core_env_value LATEX_CORE_MAIL_ENABLED "$LATEX_CORE_ENV_FILE")" == true ]] && mail_state=configured
phase=complete
trap - EXIT
cat <<EOF
------------------------------------------------------------
LaTeX Core is ready
------------------------------------------------------------

URL: $url
SMTP: $mail_state

Login with the Admin account created during installation.
For a remote basic installation, tunnel the configured HTTP port over SSH.
Institution-facing HTTPS requires operator-provided DNS/TLS/reverse-proxy policy.

Commands:
  ./latex-core status
  ./latex-core doctor
  ./latex-core diagnose
  ./latex-core stop
  ./latex-core restart
------------------------------------------------------------
EOF

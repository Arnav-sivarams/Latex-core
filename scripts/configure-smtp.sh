#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
env_file="$root/.env"
[[ -f "$env_file" ]] || { echo 'Missing .env; run ./scripts/generate-local-env.sh first.' >&2; exit 1; }
read -r -p 'Enable mail delivery? [y/N] ' enable
case "$enable" in
  y|Y|yes|YES) enabled=true ;;
  n|N|no|NO|'') enabled=false ;;
  *) echo 'Please answer y or n.' >&2; exit 2 ;;
esac
declare -A updates
updates[LATEX_CORE_MAIL_ENABLED]="$enabled"
if [[ "$enabled" == true ]]; then
  read -r -p 'SMTP host: ' smtp_host
  read -r -p 'SMTP port: ' smtp_port
  read -r -p 'SMTP security (starttls/tls): ' smtp_security
  read -r -p 'SMTP username (blank for none): ' smtp_username
  smtp_password=''
  if [[ -n "$smtp_username" ]]; then
    read -r -s -p 'SMTP password: ' smtp_password
    printf '\n'
  fi
  read -r -p 'From email: ' from_email
  read -r -p 'From name: ' from_name
  read -r -p 'Public base URL: ' public_base_url
  [[ "$smtp_host" && "$smtp_host" != *[[:space:]]* ]] || { echo 'SMTP host must be non-empty and contain no whitespace.' >&2; exit 2; }
  if [[ ! "$smtp_port" =~ ^[0-9]+$ ]] || ((smtp_port < 1 || smtp_port > 65535)); then
    echo 'SMTP port must be between 1 and 65535.' >&2
    exit 2
  fi
  smtp_security="${smtp_security,,}"
  [[ "$smtp_security" == starttls || "$smtp_security" == tls ]] || { echo 'SMTP security must be starttls or tls.' >&2; exit 2; }
  [[ "$from_email" == *@* ]] || { echo 'From email is invalid.' >&2; exit 2; }
  [[ -n "$from_name" ]] || { echo 'From name is required.' >&2; exit 2; }
  [[ "$public_base_url" == http://* || "$public_base_url" == https://* ]] || { echo 'Public base URL must start with http:// or https://.' >&2; exit 2; }
  updates[LATEX_CORE_SMTP_HOST]="$smtp_host"
  updates[LATEX_CORE_SMTP_PORT]="$smtp_port"
  updates[LATEX_CORE_SMTP_SECURITY]="$smtp_security"
  updates[LATEX_CORE_SMTP_USERNAME]="$smtp_username"
  updates[LATEX_CORE_SMTP_PASSWORD]="$smtp_password"
  updates[LATEX_CORE_SMTP_FROM_EMAIL]="$from_email"
  updates[LATEX_CORE_SMTP_FROM_NAME]="$from_name"
  updates[LATEX_CORE_PUBLIC_BASE_URL]="$public_base_url"
fi
temporary="$(mktemp "$root/.env.smtp.tmp.XXXXXX")"
trap 'rm -f "$temporary"' EXIT
umask 077
declare -A seen
while IFS= read -r line || [[ -n "$line" ]]; do
  key="${line%%=*}"
  if [[ -v "updates[$key]" ]]; then
    printf '%s=%s\n' "$key" "${updates[$key]}"
    seen[$key]=1
  else
    printf '%s\n' "$line"
  fi
done <"$env_file" >"$temporary"
for key in "${!updates[@]}"; do
  [[ -v "seen[$key]" ]] || printf '%s=%s\n' "$key" "${updates[$key]}" >>"$temporary"
done
chmod 600 "$temporary" 2>/dev/null || true
mv -f "$temporary" "$env_file"
trap - EXIT
chmod 600 "$env_file" 2>/dev/null || true
if [[ "$enabled" == true ]]; then echo 'SMTP configuration saved. The password was not displayed.'; else echo 'SMTP delivery disabled.'; fi
echo 'Run:'
echo '  ./latex-core restart'
echo '  ./latex-core doctor'

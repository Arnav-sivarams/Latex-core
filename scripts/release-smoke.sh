#!/usr/bin/env bash
set -euo pipefail

base_url="${LATEX_CORE_URL:-http://127.0.0.1:8080}"
cookie_file="$(mktemp)"
trap 'rm -f "${cookie_file}"' EXIT
email="smoke-$(date +%s)-${RANDOM}@example.test"
password='release-smoke-password'

curl -fsS -c "${cookie_file}" -H 'Content-Type: application/json' \
  -d "{\"email\":\"${email}\",\"password\":\"${password}\"}" \
  "${base_url}/api/auth/register" >/dev/null
project="$(curl -fsS -b "${cookie_file}" -H 'Content-Type: application/json' -d '{"name":"release smoke"}' "${base_url}/api/projects")"
project_id="$(printf '%s' "${project}" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')"
version="$(printf '%s' "${project}" | sed -n 's/.*"version":\([0-9]*\).*/\1/p')"
test -n "${project_id}" && test -n "${version}"
printf '\\documentclass{article}\n%% release smoke %s\n\\begin{document}\nRelease smoke.\n\\end{document}\n' "${email}" |
  curl -fsS -b "${cookie_file}" -X PUT -H "If-Match: \"${version}\"" --data-binary @- "${base_url}/api/projects/${project_id}/files/main.tex" >/dev/null
job="$(curl -fsS -b "${cookie_file}" -H 'Content-Type: application/json' -d '{}' "${base_url}/api/projects/${project_id}/compile")"
job_id="$(printf '%s' "${job}" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')"
test -n "${job_id}"
for _ in $(seq 1 90); do
  job="$(curl -fsS -b "${cookie_file}" "${base_url}/api/jobs/${job_id}")"
  state="$(printf '%s' "${job}" | sed -n 's/.*"state":"\([^"]*\)".*/\1/p')"
  case "${state}" in
    succeeded) break ;;
    failed|timed_out|cancelled) printf '%s\n' "${job}" >&2; exit 1 ;;
  esac
  sleep 1
done
test "${state}" = succeeded
artifacts="$(curl -fsS -b "${cookie_file}" "${base_url}/api/jobs/${job_id}/artifacts")"
pdf_id="$(printf '%s' "${artifacts}" | sed -n 's/.*"id":"\([^"]*\)","name":"[^"]*\.pdf".*/\1/p')"
test -n "${pdf_id}"
curl -fsS -b "${cookie_file}" "${base_url}/api/jobs/${job_id}/artifacts/${pdf_id}" >/dev/null
printf 'Release smoke passed: %s\n' "${job_id}"

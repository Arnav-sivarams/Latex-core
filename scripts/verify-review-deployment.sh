#!/usr/bin/env bash
# Read-only identity checks. Run in the actual deployment checkout on its host.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
expected="${1:?Usage: verify-review-deployment.sh APPROVED_SHA PUBLIC_BASE_URL}"
base="${2:?Usage: verify-review-deployment.sh APPROVED_SHA PUBLIC_BASE_URL}"
[[ "$expected" =~ ^[0-9a-f]{40}$ ]] || { echo 'Expected a full approved commit SHA.' >&2; exit 2; }
actual="$(git -C "$root" rev-parse HEAD)"
printf 'checkout_sha=%s\n' "$actual"
[[ "$actual" == "$expected" ]] || { echo 'Checkout does not match approved SHA.' >&2; exit 1; }
[[ -z "$(git -C "$root" status --porcelain)" ]] || { echo 'Checkout has unreviewed changes.' >&2; exit 1; }
source "$root/scripts/install-common.sh"
latex_core_init "$root"
id="$(latex_core_container_id api)"
[[ -n "$id" ]] || { echo 'No API container for the configured deployment.' >&2; exit 1; }
latex_core_docker inspect "$id" | python3 -c '
import json,sys
value=json.load(sys.stdin)[0]
labels=value.get("Config",{}).get("Labels") or {}
env=dict(item.split("=",1) for item in value.get("Config",{}).get("Env",[]) if "=" in item)
actual=labels.get("com.docker.compose.project")
configured=env.get("LATEX_CORE_COMPOSE_PROJECT") or env.get("COMPOSE_PROJECT_NAME") or "latex-core"
print("api_container="+value["Id"])
print("api_image="+value["Image"])
print("api_running="+str(value.get("State",{}).get("Running")))
print("actual_compose_project="+str(actual))
print("runtime_logs_project="+configured)
if actual != configured: sys.exit("Runtime Logs project mismatch.")
if not value.get("State",{}).get("Running"): sys.exit("API is not running.")
'
python3 - "$root" "$base" "$expected" <<'PY'
import hashlib,pathlib,sys,urllib.request,urllib.parse
root=pathlib.Path(sys.argv[1]);base=sys.argv[2].rstrip('/')
url=urllib.parse.urlsplit(base)
if url.scheme not in ('http','https') or not url.hostname or url.username or url.password or url.query or url.fragment:
    sys.exit('Expected an HTTP(S) origin without credentials, query or fragment.')
failed=False
with urllib.request.urlopen(base+'/api/build-info',timeout=15) as response:
    import json
    build=json.load(response)
print('api_source_revision='+str(build.get('source_revision')))
if build.get('schema_version')!=1 or build.get('source_revision')!=sys.argv[3]:
    sys.exit('Running API source revision does not match the approved commit.')
for name in ['review.js','writer.js','admin.js']:
    request=urllib.request.Request(base+'/static/'+name,headers={'Cache-Control':'no-cache'})
    with urllib.request.urlopen(request,timeout=15) as response:
        served=hashlib.sha256(response.read()).hexdigest()
        policy=response.headers.get('Cache-Control','')
    expected=hashlib.sha256((root/'services/server/static'/name).read_bytes()).hexdigest()
    print(f'{name}: served_sha256={served} checkout_sha256={expected} cache_control={policy}')
    embedded=build.get(name.replace('.js','_js_sha256'))
    failed |= served!=expected or served!=embedded or (name!='admin.js' and policy!='no-cache')
with urllib.request.urlopen(base+'/',timeout=15) as response: print('http_root_status='+str(response.status))
print('browser_cached_assets=UNVERIFIED; inspect the actual browser Network responses before clearing cache')
print('browser_review_workflow=UNVERIFIED; run the disposable-report acceptance described in docs/audit/REVIEW_IP_FAILURE.md')
if failed: sys.exit('Served asset or cache policy mismatch.')
PY

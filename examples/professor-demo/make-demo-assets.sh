#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
demo="$root/examples/professor-demo"
dist="$demo/dist"
command -v zip >/dev/null 2>&1 || { echo 'The zip command is required.' >&2; exit 1; }
mkdir -p "$dist"
(
  cd "$demo/main-template"
  zip -q -r "$dist/main-template.zip" .
)
(
  cd "$demo/front-matter"
  zip -q -r "$dist/front-matter.zip" .
)
echo "Created $dist/main-template.zip"
echo "Created $dist/front-matter.zip"

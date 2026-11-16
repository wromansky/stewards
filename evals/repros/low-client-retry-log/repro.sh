#!/usr/bin/env bash
set -euo pipefail

root="${FIXTURE_ROOT:-$(cd "$(dirname "$0")/../../fixture" && pwd)}"
cd "$root/client"

store="$(mktemp)"
rm -f "$store"
trap 'rm -f "$store"' EXIT
export ITEMS_STORE="$store"

out="$(node client.ts create-item ok 2>&1 >/dev/null)"
echo "$out"

! grep -q "retrying" <<<"$out"

#!/usr/bin/env bash
set -euo pipefail

root="${FIXTURE_ROOT:-$(cd "$(dirname "$0")/../../fixture" && pwd)}"
cd "$root/client"

store="$(mktemp)"
rm -f "$store"
trap 'rm -f "$store"' EXIT
export ITEMS_STORE="$store"

# The test passes unless the wall-clock second changes between two readings.
# tick.mjs forces that case, so the failure shows on every run instead of rarely.
NODE_OPTIONS="--import=$(cd "$(dirname "$0")" && pwd)/tick.mjs" \
  node --test --test-name-pattern="current time" client.test.ts

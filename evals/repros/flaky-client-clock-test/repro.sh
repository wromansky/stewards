#!/usr/bin/env bash
set -euo pipefail

here="$(cd "$(dirname "$0")/../../.." && pwd)"
cd "$here/evals/fixture/client"

store="$(mktemp)"
rm -f "$store"
trap 'rm -f "$store"' EXIT
export ITEMS_STORE="$store"

fails=0
for _ in 1 2 3 4 5; do
  if ! node --test --test-name-pattern="current time" client.test.ts >/dev/null 2>&1; then
    fails=$((fails + 1))
  fi
done

echo "clock test failures: $fails/5"
[[ "$fails" -eq 0 ]]

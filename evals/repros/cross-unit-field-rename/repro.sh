#!/usr/bin/env bash
set -euo pipefail

here="$(cd "$(dirname "$0")/../../.." && pwd)"
cd "$here/evals/fixture/cli"

store="$(mktemp)"
rm -f "$store"
trap 'rm -f "$store"' EXIT
export ITEMS_STORE="$store"

out="$(cargo run --quiet -- create widget 2>/dev/null)"
echo "$out"

grep -q '"name":"widget"' <<<"$out"

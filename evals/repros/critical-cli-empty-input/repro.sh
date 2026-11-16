#!/usr/bin/env bash
set -euo pipefail

root="${FIXTURE_ROOT:-$(cd "$(dirname "$0")/../../fixture" && pwd)}"
cd "$root/cli"

out="$(cargo run --quiet -- create 2>&1 || true)"
echo "$out"

! grep -q "panicked" <<<"$out" && grep -qi "usage" <<<"$out"

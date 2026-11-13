#!/usr/bin/env bash
set -euo pipefail

here="$(cd "$(dirname "$0")/../../.." && pwd)"
cd "$here/evals/fixture/cli"

out="$(cargo run --quiet -- create 2>&1 || true)"
echo "$out"

! grep -q "panicked" <<<"$out" && grep -qi "usage" <<<"$out"

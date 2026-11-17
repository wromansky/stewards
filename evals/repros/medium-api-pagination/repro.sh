#!/usr/bin/env bash
set -euo pipefail

root="${FIXTURE_ROOT:-$(cd "$(dirname "$0")/../../fixture" && pwd)}"
cd "$root/api"

python3 - <<'PY'
import items

items._ITEMS = []
for name in ("a", "b", "c"):
    items.create_item(name)

page = items.list_items(page=2, page_size=2)
print(page)
assert len(page["items"]) == 1, f"page 2 should contain the last item, got {page['items']}"
PY

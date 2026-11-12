#!/usr/bin/env bash
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"

usage() {
  echo "usage: arms.sh <bare|block-only|full> <dest> [source, default fixture/]" >&2
  exit 2
}

[[ $# -eq 2 || $# -eq 3 ]] || usage
arm=$1
dest=$2
src="${3:-$here/fixture}"

case "$arm" in
  bare|block-only|full) ;;
  *) usage ;;
esac

if [[ -e "$dest" ]]; then
  if [[ ! -d "$dest" || -n "$(ls -A "$dest" 2>/dev/null)" ]]; then
    echo "arms.sh: $dest exists and is not empty" >&2
    exit 2
  fi
fi
mkdir -p "$dest"
cp -R "$src/." "$dest/"
rm -rf "$dest/cli/target"
find "$dest" -type d -name __pycache__ -prune -exec rm -rf {} +
rm -f "$dest/api/items.json"

cd "$dest"
git init -q -b main
git add -A
git -c user.name=fixture -c user.email=fixture@example.invalid commit -qm "fixture: initial"

block() {
  cat <<'EOF'
<!-- stewards:start -->
## Stewards

This repo has a team of agents that already know it. Use them.

Ritual
- Entering: run `stewards status`. It lists the units, who owns what, and what is stale. To learn how a unit works, read `docs/units/<unit>.md` before opening its source; review keeps that page true.
- Before a change: run `stewards route <files>`; brief the owning maintainer (`stewards brief <unit> --role maintainer --task "<the change>"`) to explain the area and plan the change. Two or more units, or no owner: brief the architect first (`stewards brief --role architect --task "<the change>"`).
- Changing: run `stewards brief <unit> --role developer` and dispatch every brief it writes, together when there are several. A small edit you may make yourself; it is reviewed like any other.
- Before reporting done: after any edit, however small and whoever made it, run `stewards next`. Dispatch every brief it writes to a subagent that can run shell, wait for each summary, and repeat until `stewards next` lists no jobs. A review that returns findings: run `stewards brief <unit> --role developer` again to fix them. Done means no jobs; a change with jobs still listed is not done.

Roles (one per unit; the tool packs their context, you only hand over the brief path)
- maintainer: explains, plans, reviews the diff, keeps the unit's card current.
- tester: owns the contract and its scenarios; writes and runs tests; never edits source.
- documenter: keeps `docs/units/<unit>.md` true.
- validator: judges a bug from its reproduction; severity from evidence only.
- architect: plans changes that cross units; records flows and decisions.
- developer: implements one task of the maintainer's plan inside its lane.

Editing
- Think first: state assumptions; ask when the request has two readings.
- Simplicity: the ladder, in order: not needed → already in the repo → stdlib → platform → installed dependency → one line → minimum code.
- Surgical: every changed line traces to the request; match the existing style.
- Goal-driven: every non-trivial change leaves one runnable check that fails if the logic breaks.
- A deliberate shortcut is a `simplified:` comment naming its ceiling and upgrade path.

Hygiene: branch off `main`; commit only when asked; lint touched files clean, no disables.

`stewards guide` prints the long form, including how to delegate in this harness. No `stewards` on PATH: install with `cargo install stewards` and follow the rules above meanwhile.
<!-- stewards:end -->
EOF
}

case "$arm" in
  bare)
    ;;
  block-only)
    block > AGENTS.md
    git add AGENTS.md
    git -c user.name=fixture -c user.email=fixture@example.invalid commit -qm "add AGENTS.md block"
    ;;
  full)
    if ! command -v stewards >/dev/null 2>&1; then
      echo "arms.sh: full arm needs a 'stewards' binary on PATH (install it with 'cargo install stewards')" >&2
      exit 1
    fi
    stewards init
    ;;
esac

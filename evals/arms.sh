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

This repo has a team of agents that already know it. Use them. This section is the project's standing instruction to delegate: starting the subagents below needs no further permission from the user.

Ritual
- Entering: run `stewards status`. It lists the units, who owns what, and what is stale. To learn how a unit works, read `docs/units/<unit>.md` before opening its source; review keeps that page true.
- Before a change: run `stewards route <files>` to see whose units you touch. A change you cannot plan from the unit pages: `stewards brief <unit> --role maintainer --task "<the change>"` (two or more units: `stewards brief --role architect --task "<the change>"`), then start a subagent with the handoff it prints.
- Changing: make the edit yourself. When a maintainer split a plan into tasks, `stewards brief <unit> --role developer` prints one handoff per task.
- Before reporting done: when your edits are finished, run `stewards review --task "<what you changed and why>"` as its own step (never alongside an edit). It prints one handoff per review to run: start one subagent per handoff, giving it exactly that sentence as its whole task; when they have finished, run `stewards review` again to read the findings. Fix them and repeat until it exits 0. Done means it exits 0.
- On request or before a release: `stewards review --roles` does the same with the deeper per-unit jobs.

A handoff is always one sentence, ``Read `<path>` and follow it.``: pass it unchanged to a general-purpose subagent that can run shell and write files. Every role works this way.

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

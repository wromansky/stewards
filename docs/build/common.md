## Rules for every writer
- First: `test -e target || cp -al <repo>/target target` (warm build cache). `protoc` is at `~/.local/bin/protoc` (on PATH).
- `DESIGN.md` is the spec. Where your task changes behaviour it describes, update that text in the same change. If the task conflicts with the spec in a way the brief does not settle, use contact_supervisor; do not drift.
- Touch only the files your task names (plus tests and DESIGN.md text for it). Do not edit `.stewards/` or `docs/units/`; do not run `stewards submit`, `stamp`, `docs`, `next` or `init` in your worktree.
- Smallest code that fully does the job. Reuse existing helpers. No new dependencies unless the brief says so. Mark a deliberate shortcut with a `simplified:` comment (never `shortcut:`).
- Tests make no model or network calls.
- Before you finish: `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and the full `cargo test` green, run twice.
- Do not commit, stage or push. Report: what changed per file, tests added, anything you skipped or are unsure of.

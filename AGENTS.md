<!-- stewards:start -->
## Stewards

This repo has a team of agents that already know it. Use them.

Ritual
- Entering: run `stewards status`. It lists the units, who owns what, and what is stale. To learn how a unit works, read `docs/units/<unit>.md` before opening its source; review keeps that page true.
- Before a change: run `stewards route <files>` to see whose units you touch. A change you cannot plan from the unit pages: `stewards brief <unit> --role maintainer --task "<the change>" --run` prints the owning maintainer's explanation and plan; for two or more units use `stewards brief --role architect --task "<the change>" --run`.
- Changing: make the edit yourself. When a maintainer split a plan into tasks, `stewards brief <unit> --role developer --run` carries them out.
- Before reporting done: when your edits are finished, run `stewards review --task "<what you changed and why>"` as its own step (never alongside an edit). It reviews the whole change in a separate session and prints the findings. Fix them and run it again; repeat until it exits 0. Done means it exits 0; a change it still has findings for is not done.
- On request or before a release: `stewards review --roles` runs the deeper per-unit jobs.

Each of these runs in a separate session outside your context: you never launch subagents for stewards. Without `--run`, and with `stewards next`, the same briefs are written but not run, for a harness stewards cannot start.

Roles behind the review: reviewer per change; maintainer, tester, documenter, validator, architect, developer for plans, first reviews and `--roles`.

Editing
- Think first: state assumptions; ask when the request has two readings.
- Simplicity: the ladder, in order: not needed → already in the repo → stdlib → platform → installed dependency → one line → minimum code.
- Surgical: every changed line traces to the request; match the existing style.
- Goal-driven: every non-trivial change leaves one runnable check that fails if the logic breaks.
- A deliberate shortcut is a `simplified:` comment naming its ceiling and upgrade path.

Hygiene: branch off `main`; commit only when asked; lint touched files clean, no disables.

`stewards guide` prints the long form, including the manual path for a harness stewards cannot start. No `stewards` on PATH: install with `cargo install stewards` and follow the rules above meanwhile.
<!-- stewards:end -->

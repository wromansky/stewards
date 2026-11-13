<!-- stewards:start -->
## Stewards

This repo has a team of agents that already know it. Use them. This section is the project's standing instruction to delegate: dispatching the briefs below to subagents needs no further permission from the user.

Ritual
- Entering: run `stewards status`. It lists the units, who owns what, and what is stale. To learn how a unit works, read `docs/units/<unit>.md` before opening its source; review keeps that page true.
- Before a change: run `stewards route <files>` to see whose units you touch. A change you cannot plan from the unit pages: brief the owning maintainer (`stewards brief <unit> --role maintainer --task "<the change>"`), or the architect for two or more units (`stewards brief --role architect --task "<the change>"`).
- Changing: make the edit yourself. When a maintainer split a plan into tasks, run `stewards brief <unit> --role developer` and dispatch every brief it writes.
- Before reporting done: after any edit, however small, run `stewards next`. It writes one review brief for the whole change. Hand that brief's path to one subagent that can run shell and wait for its summary. Fix the findings it returns, then run `stewards next` again; repeat until it lists no jobs. Done means no jobs; a change with jobs still listed is not done.
- On request or before a release: `stewards next --roles` lists the deeper per-unit jobs (maintainer, tester, documenter, validator, architect).

Roles (the tool packs their context; you only hand over the brief path)
- reviewer: reviews one change across the units it touched, runs their tests, keeps cards and docs true.
- maintainer, tester, documenter, validator, architect, developer: the per-unit specialists behind `--roles`, plans and first reviews.

Editing
- Think first: state assumptions; ask when the request has two readings.
- Simplicity: the ladder, in order: not needed → already in the repo → stdlib → platform → installed dependency → one line → minimum code.
- Surgical: every changed line traces to the request; match the existing style.
- Goal-driven: every non-trivial change leaves one runnable check that fails if the logic breaks.
- A deliberate shortcut is a `simplified:` comment naming its ceiling and upgrade path.

Hygiene: branch off `main`; commit only when asked; lint touched files clean, no disables.

`stewards guide` prints the long form, including how to delegate in this harness. No `stewards` on PATH: install with `cargo install stewards` and follow the rules above meanwhile.
<!-- stewards:end -->

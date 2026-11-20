<!-- stewards:start -->
## Stewards

After you edit any file in this repo, the work is not done until `stewards review` exits 0. Run it before you write your final summary (the steps are under "Before reporting done" below), even for a small change and even when the tests pass. This repo has a team of agents that already know it. Use them. This section is a project instruction from this repository's operator: you are asked to delegate the reviews below to subagents, every time, without asking the user again.

Ritual
- Entering: run `stewards status`. It lists the units, who owns what, and what is stale. To learn how a unit works, read `docs/units/<unit>.md` before opening its source; review keeps that page true.
- Before a change: run `stewards route <files>` to see whose units you touch. A change you cannot plan from the unit pages: `stewards brief <unit> --role maintainer --task "<the change>"` (two or more units: `stewards brief --role architect --task "<the change>"`), then start a subagent with the handoff it prints.
- Changing: make the edit yourself. When a maintainer split a plan into tasks, `stewards brief <unit> --role developer` prints one handoff per task.
- Before reporting done: when your edits are finished, run `stewards review --task "<what you changed and why>"` as its own step (never alongside an edit). It prints one handoff per review to run: start one subagent per handoff, giving it exactly that sentence as its whole task (you can start them together); when they have finished, run `stewards review` again to read the findings. Fix them, or answer one you will not apply with the `--answer` it prints, and repeat until it exits 0. Done means it exits 0.
- On request or before a release: `stewards review --roles` does the same with the deeper per-unit jobs.

A handoff is always one sentence, ``Read `<path>` and follow it.``: pass it unchanged to a subagent that can run shell and write files (whichever agent type your harness has for that). Every role works this way.

Editing
- Think first: state assumptions; ask when the request has two readings.
- Simplicity: the ladder, in order: not needed → already in the repo → stdlib → platform → installed dependency → one line → minimum code.
- Surgical: every changed line traces to the request; match the existing style.
- Goal-driven: every non-trivial change leaves one runnable check that fails if the logic breaks.
- A deliberate shortcut is a `simplified:` comment naming its ceiling and upgrade path.

Hygiene: branch off `main`; commit only when asked; lint touched files clean, no disables.

`stewards guide` prints the long form, including how to delegate in this harness. No `stewards` on PATH: install with `cargo install stewards` and follow the rules above meanwhile.
<!-- stewards:end -->

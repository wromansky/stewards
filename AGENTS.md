<!-- stewards:start -->
## Stewards

This repo has a team of agents that already know it. Use them.

Ritual
- Entering: run `stewards status`. It lists the units, who owns what, and what is stale.
- Before a change: run `stewards route <files>`; brief the owning maintainer (`stewards brief <unit> --role maintainer --task "<the change>"`) to explain the area and plan the change. Two or more units, or no owner: brief the architect first (`stewards brief --role architect --task "<the change>"`).
- Changing: run `stewards brief <unit> --role developer` and dispatch every brief it writes, together when there are several. A small edit you may make yourself.
- Before reporting done: run `stewards next`. Dispatch every brief it writes to a subagent that can run shell, wait for each summary, and repeat until `stewards next` lists no jobs. A review that returns findings: run `stewards brief <unit> --role developer` again to fix them. Done means no jobs.

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

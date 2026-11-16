# Task 10: DESIGN.md spec debt

Repo: stewards, branch off `main` after every other task of this plan is merged. Follow
`docs/build/common.md`. Model: Haiku 5.5, thinking high. Docs only:
no code change; `cargo test` must still pass (`tests/guide.rs` and `tests/llm.rs` read DESIGN.md).

## Why
DESIGN.md is the spec and the record. Parts of it describe the code as it was, not as built.

## Do
Fix each item of "Spec debt" in `docs/build/stage3/PLAN.md` (items 1–5,
7–9, 11, and those of 10 and 12–17 that the earlier tasks did not already settle; skip 6, which is
in `.stewards/`), checking each against the code on `main`, not against this list:
- Module API: retitle "(as built)"; one line per public function of every module in `src/lib.rs`,
  signatures copied from the code; drop what no longer exists.
- First-session walkthrough: every JSON shape matches what the commands print today (run them on a
  temp copy of `tests/fixtures/single` if that helps; never in this repository's `.stewards/`).
- Build order and Waves: say what happened (numbered PRs on topic branches), keep the scope decision.
- Handoff: "State" and "Next action" describe `main` after this plan; keep the dated eval sections.
- "stewards' own units": the ten units of `.stewards/units.toml` with their paths and interface ids
  from the cards.
- Evals and Verification text: the drivers that exist (Pi, Claude Code, Hermes), the unit count,
  and the cleanliness, docs-accuracy, Codex, and CI-cadence lines as OQ 3 was answered (the
  supervisor gives you the answer; if none, mark each "not in 0.1.0" without deleting it).
- Error table and `src/guide.rs`: `harness_missing` names `hermes` too (the only code edit allowed,
  and only if the guide text must match the table).
- Facts harness table: the column no longer says "rendered".
- Risk register 4: the kill criterion's signal as OQ 3 was answered.
- LLM modes: `--direct` is honoured by `init --draft` only (unless an earlier task changed that).

## Files
Own: `DESIGN.md`; `src/guide.rs` (the one error row, if needed). Nothing else.

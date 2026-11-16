# Task 06: `validate --report`

Repo: stewards (Rust), branch off `main` after tasks 01 and 04 are merged. Follow
`docs/build/common.md`. Model: Sonnet 5.5, thinking high.

Read DESIGN.md: Verification and operations › Validation (line ~426), Knowledge › Bugs, Command
surface `validate`, Rules › Pack (validator sections, `repro_result`), Role prompts › Validator,
Trust and safety › Execution. Then `src/validate.rs` (stub), `src/run.rs`, `src/submit.rs`
(`check_bugs`, `apply_bugs`, `validator`, `read_answer`, the lost-update helpers), `src/pack.rs`
(the validator branch), `src/brief.rs` (`write`, `build`, the validator header with the bug title),
`tests/submit_roles.rs` (validator tests).

## Why
"A bug report must carry a repro command; `validate` runs it through the allowlist; a failing run
is proof; the validator assigns severity from evidence." Today `validate` is a stub and every
validator brief says the repro was not run.

## Build
1. **`stewards validate --report <file>`**, on the current repo. DECIDED BY PLANNER (the spec gives
   no shape): the file is one JSON object, strict like an answer (unknown or duplicate fields
   rejected, read with `submit`'s answer reader and its size cap): `{"unit", "title", "repro",
   "evidence", "interface"?}`, the tester's `Bug` plus its unit. Checks: the unit exists
   (`unknown_unit`); `check_bugs` as for a tester (caps, the repro on the unit's allowlist, a known
   interface id); errors come back as `answer_parse_failed` with `errors[]`, as `submit` does. An agent must be
   able to write the file from the tool's output alone: the `--report` help text and every
   rejection's fix print the shape with an example object (`{"unit": "api", "title": "…",
   "repro": "<a commands.test prefix> …", "evidence": "…", "interface": null}`).
2. **Record** with `apply_bugs` (same title updates the gap and keeps severity and issue; a probable
   duplicate is not added and its message is printed), written through `Repo::write` under the
   lost-update check.
3. **Run the repro** (`run::run`, the unit's allowlist). DECIDED BY PLANNER: `reproduced` = the
   command exited non-zero; a timeout or a command that could not start is not proof (`reproduced:
   false`, with the reason).
4. **Brief the validator** for that bug: the pack's validator branch takes the bug by title when
   given (else, as today, the first bug with no severity), and `repro_result` is the run's
   `RunResult` plus `reproduced`. DECIDED BY PLANNER: every validator brief that is written runs the
   bug's repro first (`brief::write` for the validator), so `next --roles` and `brief <unit> --role
   validator` carry a result too; `next --no-brief` and `pack --role validator` run nothing and show
   `repro_result: null`, rendered as "not run". The validator role text is unchanged.
5. **Output**: `{recorded: {unit, title}, duplicate?, repro_result, reproduced, brief_path,
   handoff, then}`; `then` = `Start one subagent with the handoff below as its whole task; when it
   has finished, run stewards review to read its judgment.` A duplicate writes no brief and `then`
   says to reuse the existing title or give another anchor. Exit 0; errors exit 2.
6. `validate` with neither flag: `usage` (`pass --report <file> to record a bug, or --export to file
   judged bugs as issues`). `--export` stays `not_implemented` (task 09).
7. **DESIGN.md**: Validation, Bugs, the `validate` row (the report shape), Pack (`repro_result`
   shape), Trust and safety › Execution (a validator brief runs the repro).

## Tests (no network; repros are `sh` scripts allowlisted in a temp repo)
Recorded on the card with no severity, the interface's anchor taken; a repro outside the allowlist,
an unknown interface id, an over-cap title, and an unknown field are rejected and nothing is
written; an unknown unit is `unknown_unit`; a repro exiting 1 → `reproduced: true` and the brief's
pack shows it; exiting 0 → `false`; a probable duplicate → not added, no brief; the same title →
updated, severity kept; then a validator `submit` against that brief sets severity (end to end);
`next --roles` writes a validator brief with a `repro_result`; `next --no-brief` runs nothing (a
repro that would create a marker file does not).

## Files
Own: `src/validate.rs`, `src/pack.rs` (validator branch only), `src/brief.rs` (validator brief
only), `src/submit.rs` (visibility of `check_bugs`, `apply_bugs`, the answer reader; no behaviour
change), `tests/validate.rs` (new); shared append: DESIGN.md sections above. Do not touch
`next.rs`, `run.rs`, `baml_src/`. New file: `tests/validate.rs` → unit `cli` (with `src/validate.rs`).

# Task 08: the remaining deterministic eval graders

Repo: stewards (Rust), branch off `main` after task 04 is merged. Follow
`docs/build/common.md`. Model: Sonnet 5.5, thinking high.

Read DESIGN.md: Evals (Suites, Fixture, Seeded bugs, Tasks, Runner: the review outcome and
`check_green` paragraphs, Pass criteria), Knowledge › Plans and the plan behaviour task 04 built.
Then `src/eval/mod.rs` (`next_check`, `reviewed_units` and how a child's `submit` output is found in
each driver's transcript, the run record, the per-run steps after a session), `src/eval/grade.rs`
(summary), `src/eval/suite.rs`, `evals/suites/fixture.toml`, `evals/repros/*/repro.sh`,
`evals/tasks/02-fix-pagination.txt` and `05-priority-field.txt`, `tests/eval.rs`.

## Why
OQ 3 (PLAN.md), recommended answer: the gate stays the ritual; two cheap, deterministic,
report-only graders remain worth having: did task 5 produce a plan that closed, and did task 2
actually fix the seeded bug. The old "tester's repro reproduces and the validator's severity equals
the seeded one" needs a validator job the per-change ritual never lists, and the LLM judges and the
Codex driver are out of 0.1.0. Build only these two unless the supervisor says otherwise.

## Build
1. **The runner's own check** runs `stewards next --no-brief --no-run` (DECIDED BY PLANNER: since
   task 04, `next` runs flow and step checks; the gate measures review coverage, not test results,
   and the session's own `review` already ran them). Same for the setup checks that call
   `next_check`. Update the Runner and Pass criteria text.
2. **Bug fixed** (every arm): a task in a suite manifest may name `repro = "<dir under
   evals/repros>"`; fixture task 2 names `medium-api-pagination`. After each run of that task (and
   before its directory is deleted), the runner runs `bash evals/repros/<dir>/repro.sh` from the
   checkout with `FIXTURE_ROOT=<run dir>`, timeout 60 s, and records `bug_fixed: true` when it exits
   0, `false` otherwise, `null` when the task names no repro. Each `repro.sh` takes its fixture root
   from `FIXTURE_ROOT`, defaulting to today's `evals/fixture` (edit all five, so they stay alike). An
   unknown repro directory is a `usage` error at manifest load.
3. **Plan stored and closed** (`full` runs): a task may set `expects_plan = true`; fixture task 5
   does. The run records `plan: {stored, closed}`: `stored` when a child's `stewards submit --role
   architect` printed a summary containing `plan <id> stored` (found the way `reviewed_units` finds
   a reviewer's submit output, on every driver); `closed` when stored and no `.stewards/plans/*.toml`
   is left in the run directory after the session.
4. **Summary**: per task, `bug fixed` as `k/n` per arm, and `plan stored k/n, closed k/n` for `full`;
   report-only, never part of a verdict. Pass criteria in DESIGN.md: replace the "Bug tasks" and
   "Task 5" sentences with these two; leave the cleanliness and docs-accuracy sentences for task 10
   (they depend on OQ 3's answer).

## Tests (no model, no network)
Manifest: `repro` and `expects_plan` parse; an unknown repro is `usage`. Bug fixed: copy
`evals/fixture` to a temp dir, run the pagination repro there with `FIXTURE_ROOT` → `false`; apply
its `fix.patch` → `true` (python3 is present here and in CI). Plan: from a recorded transcript of
each driver with an architect child whose submit printed `plan … stored` → `stored: true`; a run
directory with a plan file left → `closed: false`, none left → `true`. The runner's next check
passes `--no-run` (assert the argv). The summary renders both lines and no verdict changes.

## Files
Own: `src/eval/**`, `evals/**` (manifests, the five `repro.sh`), `tests/eval.rs`,
`tests/fixtures/transcripts/**` (new samples if needed); shared append: DESIGN.md Evals. Do not run
a real eval session. No new files outside the `eval` unit's paths.

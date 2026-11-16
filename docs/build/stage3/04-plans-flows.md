# Task 04: architect plans, step checks and flow checks in `next`

Repo: stewards (Rust), branch off `main` after tasks 01 and 02 are merged. Follow
`docs/build/common.md`. Model: Opus 5.5, thinking medium (a new stored
schema). Spec review on the patch.

Read DESIGN.md: Knowledge › Plans (line ~376), Team › Triggers, Schemas › `plans/<id>.toml` and
`repo.toml` `[[flow]]`, Rules › next (the architect, flow-check, and exit bullets), Rules › review
(Then, Exit), Trust and safety › Execution and Writes, Role prompts › Architect and the answer shape
`plan`. Then `src/run.rs` (task 01), `src/next.rs` (`next`, `role_jobs` and its architect block,
`run`), `src/submit.rs` `architect`, `src/review.rs` (the `then` logic), `src/model.rs`
(`load_repo`, how cards are read safely), `tests/next.rs`, `tests/submit_roles.rs` (architect tests).

## Why
DESIGN.md: "the architect's steps are state in `.stewards/plans/<id>.toml`; `next` runs each stale
unit's step check through the allowlist, marks pass or fail, and closes the plan when all pass and
every unit it names is fresh again; a closed plan is deleted". Today `submit` drops the plan ("plan
not stored yet"), the architect job returns every round two stamped units change, and `next`
ignores `--no-run` and never runs a flow's `check`.

## Build
1. **Schema** (`src/model.rs`): `Plan { title, created, step: Vec<PlanStep> }`, `PlanStep { unit,
   change, interfaces, check, result: Option<StepResult> }`, `StepResult` `pass | fail`.
   DECIDED BY PLANNER: `result` is the one state field (DESIGN.md's schema has none, but "marks pass
   or fail" needs it stored). `load_repo` reads `.stewards/plans/*.toml` (sorted; through no link,
   the same way it reads cards; a parse failure is `invalid_toml` with path and line) into
   `Loaded.plans: Vec<(String /*id*/, Plan)>`. A plan naming an unknown unit is `invalid_toml` too.
2. **Storing** (`submit::architect`): validate each step: `unit` is a unit of `units.toml`
   (`answer_parse_failed`, listing the units); `check` passes `run`'s allowlist for that unit
   (`command_not_allowed` as a field error; today it is length-checked only); at least one step.
   Interface ids are not checked (a plan may name an interface the change adds). DECIDED BY PLANNER:
   the id is `<created>-<slug>`, `created` today's date (UTC, `YYYY-MM-DD`, the first ten characters of
   `telemetry::rfc3339`), slug = the title
   lowercased, runs of other characters as `-`, at most 40 characters, `-2`, `-3` … on a clash; it
   must be a valid file name by the unit-name rule. Written with the other writes (lost-update
   check, retire). The summary replaces "plan not stored yet" with `plan <id> stored (<n> steps):
   make each step's change, then run stewards review; it runs each step's check and closes the plan
   when every check passes and every unit is reviewed`.
3. **`next`** (`src/plan.rs` holds the logic; `next.rs` calls it), per repo, unless `--no-run`:
   - a step runs its `check` (`run::run` with the step unit's allowlist) when its unit is stale or
     never stamped, or when its stored result is `fail` (DECIDED BY PLANNER: otherwise a step that
     failed and was then stamped would never run again and its plan never close); the result is
     written back to the plan file (`Repo::write`, only when it changed);
   - a plan whose every step is `pass` and whose every step unit is fresh is removed
     (`Repo::remove`) and reported closed;
   - each flow with a `check` and at least one stale unit runs it (`run::run`, the repo's
     allowlist, no unit) into `flow_checks` (the existing `FlowCheck`; a timeout is a failure whose
     output says so).
   `NextReport` gains `plans[] {id, title, closed, steps[] {unit, change, check, result, fresh,
   output?}}` (output only for a check run in this call). With `--no-run`, nothing runs and stored
   results are shown. Exit 1 also when a flow check or a step result is `fail`.
4. **Architect job**: listed only when no open plan of that repo names every unit that moved
   (replace the "No plans exist yet" block).
5. **What to do next** (OQ 2, until answered): a plan step that is not `pass` carries `todo`: `make
   the change yourself, or run stewards brief <unit> --role maintainer --task "<change>" to split it
   into developer tasks`. Rewrite DESIGN.md Triggers' "(or the unit's step of an open architect
   plan)" to that. `brief --role developer` is unchanged.
6. **`review`'s `then`**: a failing flow or step check adds `Fix the failing checks listed under
   next.flow_checks and next.plans, then run stewards review again.` (after the jobs line when jobs
   are listed; alone otherwise; exit 1). An open plan with steps left and nothing else due is still
   `done` (exit 0), followed by `Plan <id> has steps left; see next.plans.`
7. **DESIGN.md**: Plans, the plans schema (`result`, the id rule), Rules › next (step and flow
   bullets, the rerun rule, `plans` in the report, the architect rule), review Then, Trust and
   safety › Writes (`next` writes step results and deletes closed plans), Triggers (step 5).

Note for the report: this repository's `repo.toml` has five flows whose checks run `cargo test …`;
after this task every `next` and `review` here runs them while a unit of the flow is stale.

## Tests (no network; checks are `sh` scripts allowlisted in a temp repo)
Architect submit with a plan writes `.stewards/plans/<id>.toml` and says so; an unknown step unit,
a disallowed check, and an empty plan are rejected with field errors; two plans with one title get
`-2`. `next`: a stale unit's step check runs and stores `pass`; a failing one stores `fail`, exits
1, and runs again after the unit is stamped; all `pass` and all fresh → the file is deleted and
reported `closed`; `--no-run` runs nothing and writes nothing; a flow check runs only when one of
its units is stale, a failing one exits 1; the architect job is not listed when an open plan names
every moved unit and is listed when the plan names fewer; an invalid plan file is `invalid_toml`
with its path; a plan file that is a symlink is `unsafe_path`. `review`: the failing-checks line
and the "steps left" line.

## Files
Own: `src/plan.rs` (new), `src/model.rs` (plan types, loading), `src/next.rs`, `src/submit.rs`
(`architect` only), `src/review.rs` (the `then` logic only), `tests/plans.rs` (new),
`tests/next.rs`, `tests/review.rs`, `tests/submit_roles.rs` (architect tests); shared append:
`src/lib.rs`, DESIGN.md sections above. Do not touch `run.rs` (call it), `brief.rs`, `pack.rs`,
`baml_src/` (the answer shape does not change). New files: `src/plan.rs` → unit `next`;
`tests/plans.rs` → unit `next`.

# Stages 3 and 4 for 0.1.0: plan

Source: DESIGN.md on `main` at `3bed64d`, the code under `src/`, `tests/`, `.stewards/`, and the
0.1.0 scope decision of 2026-11-14 (DESIGN.md Build order). Every writer follows
`docs/build/common.md`.

## What is still missing (from the code)

| stub or gap | where |
|---|---|
| `validate` (both flags) returns `not_implemented` | `src/validate.rs` |
| `completions` returns `not_implemented`; no man page | `src/completions.rs` |
| `next --pr`, `review --pr`, `init --ci` return `not_implemented` | `src/next.rs:675`, `src/review.rs:423`, `src/init.rs:552` |
| `next --no-run` is ignored; `flow_checks` is always empty | `src/next.rs` `run`, `next` |
| an architect plan is validated, then dropped ("plan not stored yet") | `src/submit.rs` `architect` |
| the architect job ignores plans ("No plans exist yet") | `src/next.rs` `role_jobs` |
| no code runs a command: `submit` checks the allowlist but nothing runs a repro or a check | `src/submit.rs` `allowlist`, `check_command` |
| anchors are text matches; `interface_lines` is a text heuristic | `src/anchors.rs` |
| the validator pack's `repro_result` is always `null` | `src/pack.rs` |
| no release workflow, install script, AUR package, `RELEASING.md` | repo root |
| eval: no plan grader, no bug-fixed grader | `src/eval/` |

## Merge rule for shared files

A task owns its files outright. A few files are shared append points, edited by several tasks only
at their own lines, merged by the parent: `DESIGN.md` (each task edits the sections its brief
names), `Cargo.toml` `[dependencies]` (each task adds its own lines), `Cargo.lock` (the parent
drops lock hunks that do not apply and runs `cargo build --offline` to regenerate),
`src/lib.rs` (one `pub mod` line per new module), and the error-code rows in `src/guide.rs` and the
DESIGN.md error table (a row per new code; `tests/guide.rs` checks the two agree).

New files belong to no unit until the parent adds them to `.stewards/units.toml` (writers never edit
`.stewards/`): until then `stewards review` lists them as `unowned_changed` and does not exit 0. Each
brief names its new files and the unit they suggest. Route them when applying the patch.

## Waves

### Wave 1 (no dependencies)

| task | goal | owns | tier, why | size | extra review |
|---|---|---|---|---|---|
| `01-runner` | one allowlisted command runner every caller uses | `src/run.rs` (new), `src/submit.rs` (allowlist and `check_command` move out), `tests/run.rs` (new) | Opus 5.5 medium: process execution trust boundary | M | security |
| `02-anchors-static` | tree-sitter anchors and interface spans for the eleven static grammars | `src/anchors.rs`, `src/anchors/*.scm` (new), `src/pack.rs` (window, halve), `src/next.rs` (`interfaces_touched`, `BLOCK_LANGUAGES`, the dead-anchor reason), `tests/anchors.rs`, `tests/pack.rs`, `tests/next.rs` | Sonnet 5.5 high: large but well specified | L | none |
| `03-distribution` | completions, man page, release workflow, install script, AUR, `RELEASING.md` | `src/completions.rs`, `src/main.rs`, `src/llm.rs` (`prepare_baml_env` only), `.github/workflows/release.yml`, `install.sh`, `packaging/aur/*`, `RELEASING.md`, `tests/skeleton.rs`, `tests/completions.rs` (new) | Sonnet 5.5 high: glue with a clear spec | M | security (supply chain, secrets) |

### Wave 2 (after 01 and 02 are checkpointed)

| task | goal | owns | tier, why | size | extra review |
|---|---|---|---|---|---|
| `04-plans-flows` | store architect plans; `next` runs step and flow checks, closes plans | `src/plan.rs` (new), `src/model.rs` (`Loaded.plans`), `src/next.rs`, `src/submit.rs` (`architect`), `src/review.rs` (`then` line), `tests/plans.rs` (new), `tests/next.rs`, `tests/review.rs`, `tests/submit_roles.rs` (architect tests) | Opus 5.5 medium: new stored schema | L | spec |
| `05-runtime-grammars` | load extra grammars from `~/.stewards/grammars/<lang>/` | `src/anchors.rs`, `src/check.rs` (one warning), `src/user.rs` (path helper only), `tests/anchors.rs`, `tests/fixtures/grammars/**` (new) | Sonnet 5.5 high: contained, one unsafe seam | M | security (dlopen) |

### Wave 3 (after 04; 06 also needs 01, 07 also needs 03)

| task | goal | owns | tier, why | size | extra review |
|---|---|---|---|---|---|
| `06-validate-report` | `validate --report`: record, run the repro, brief the validator | `src/validate.rs`, `src/pack.rs` (validator bug choice, `repro_result`), `src/brief.rs` (validator brief runs the repro), `src/submit.rs` (visibility of `check_bugs`/`apply_bugs` only), `tests/validate.rs` (new) | Sonnet 5.5 high: reuses existing checks | M | none |
| `07-pr-ci` | `next --pr`, `review --pr`, `init --ci`, the gh/glab seam | `src/forge.rs` (new), `src/next.rs` (`run` only), `src/review.rs` (`run` only), `src/init.rs` (`--ci` only), `tests/forge.rs` (new), `tests/next.rs`, `tests/init.rs`, `tests/review.rs`, `tests/skeleton.rs` (only if OQ 1 is answered (a)) | Sonnet 5.5 high: glue over two CLIs | M | security (external text, tokens) |
| `08-eval-graders` | report-only plan and bug-fixed graders; the runner's check skips commands | `src/eval/**`, `evals/**`, `tests/eval.rs` | Sonnet 5.5 high: inside one known unit | S | none |

### Wave 4 (after 06 and 07)

| task | goal | owns | tier, why | size | extra review |
|---|---|---|---|---|---|
| `09-tracker-export` | `validate --export` to GitHub, GitLab, Jira, Linear | `src/tracker.rs` (new), `src/validate.rs` (`--export`), `tests/tracker.rs` (new) | Opus 5.5 medium: network, secrets, four APIs | L | security |

### Wave 5 (after everything)

| task | goal | owns | tier, why | size | extra review |
|---|---|---|---|---|---|
| `10-spec-debt` | bring DESIGN.md in line with the code | `DESIGN.md` only | Haiku 5.5 high: complete list, mechanical | S | none |

Then the supervisor, by hand, not briefed: route every new file in `.stewards/units.toml`; run this
repo's own `stewards review` until it exits 0 (after 02 this will include units whose anchors died,
see 02); one eval pass on the fixture and `self` suites; the history scrub of private names (last
step before tagging); tag `v0.1.0`; watch the release workflow.

Critical path: 01/02 → 04 → 06/07 → 09 → 10. Nine product tasks and one docs task.

No task adds a per-harness code path (07 and 09 talk to forges and trackers, not harnesses). Every
new or unstubbed command prints a `then` line or a next-step hint. Planner decisions the supervisor
should check: `grep -n "DECIDED BY PLANNER" docs/build/stage3/*.md`.

New dependencies: `tree-sitter` and the eleven grammar crates (02; no parser in std or the tree);
`clap_complete`, `clap_mangen` (03; hand-written completions and roff would drift from clap);
`libloading` (05) and `ureq` (09) are already compiled as dependencies of `baml-sys`: they become
direct dependencies at the locked versions, so nothing new builds.

## OPEN QUESTIONS (the user's to answer; each brief says what it does until answered)

1. **`review --pr <n>`.** DESIGN.md says it "runs maintainer review briefs on the diff and posts
   findings", but stewards never starts a session, and only `init --draft` honours `--direct`.
   Options: (a) cut it from 0.1.0; (b) **recommended:** `review --pr <n>` is `review` plus posting
   what that call reported (findings, notes, the job list) as one PR/MR comment updated in place;
   (c) wire `--direct` so CI can run the review itself (new LLM path, large). 07 builds (b).
2. **Plan steps and developers.** DESIGN.md Triggers says `brief <unit> --role developer` also
   briefs "the unit's step of an open architect plan", but a step has no lane (files). Options:
   **recommended:** drop it: for a pending step `next` prints "make the change yourself, or
   `stewards brief <unit> --role maintainer --task "<step change>"` to split it into developer
   tasks"; or give a step's developer the unit's whole owned file set as its lane. 04 builds the
   recommendation.
3. **Remaining eval graders.** The gate is now the ritual (Pass criteria). Recommended:
   - keep, report-only and deterministic: task 5 "a plan was stored and closed"; task 2 "the seeded
     bug is fixed" (the hidden repro passes after the session, every arm), replacing "the tester's
     repro reproduces and the validator's severity equals the seeded one", which the per-change
     ritual no longer runs (no tester or validator job without `--roles`);
   - drop from 0.1.0: the cleanliness LLM judge (3 votes) and the docs-accuracy LLM judge. They need
     a model-calling path in the runner and cost about 2M tokens a pass; the reviewer's recorded
     findings and notes already measure cleanliness inside `full`. Consequence: risk 4's kill
     criterion ("docs-accuracy fails two releases running") needs a new signal; suggest "dead anchors
     or a docs page out of date with its card at release";
   - drop from 0.1.0: the Codex driver (first in the cut order, and Codex rejects this account's
     model ids) and "eval in CI on release tags" (CI has no harness subscriptions); `RELEASING.md`
     gets a manual eval pass instead.
   08 builds only the "keep" items.
4. **Argument trust after an allowlisted prefix.** A repro, step check, or flow check is written by
   an agent. The prefix rule (`cargo test` allows `cargo test --config build.rustc-wrapper=/x`,
   `pytest -p plugin`) lets that text choose flags that run other programs. Recommended: keep the
   spec's prefix rule, record it in Trust and safety as a known limit, and keep CI on `check`, which
   runs nothing. Alternative: refuse trailing words that start with `-` unless the entry lists them
   (breaks `pytest -k x`). 01 builds the recommendation.
5. **Jira auth.** DESIGN.md names `JIRA_URL` and `JIRA_TOKEN`. Jira Data Center takes that token as
   a Bearer PAT; Jira Cloud needs Basic auth with an account email. Recommended: optional
   `JIRA_EMAIL`; when set, Basic `email:token`, else Bearer. Issue type: `Bug`, named in the error
   when the project has none. 09 builds the recommendation.

## What can be verified on this machine and what cannot

| item | real end-to-end check needs | here |
|---|---|---|
| GitHub issue export, `next --pr`, `review --pr`, the GitHub CI template | `gh` logged in; a scratch repo the supervisor agrees to create (for example a private `wromansky/stewards-scratch`) with an issue, a PR, and Actions enabled | yes, with the supervisor's ok (it creates visible issues and comments) |
| GitLab issue export, MR comments, the GitLab CI template | the `glab` binary, a GitLab account, `glab auth login` with an `api`-scope token, a project with an MR and CI runners | no: recorded-response fakes only |
| Jira export | a Jira site URL, an API token (Cloud: plus the account email, see OQ 5), a project key whose issue types include `Bug` | no: fakes only |
| Linear export | a Linear workspace, a personal API key (`LINEAR_API_KEY`), a team key | no: fakes only |
| static grammars | `cc` (present: gcc) for the grammar crates' C sources | yes |
| runtime grammars, Linux `.so` | `cc` to build a grammar's `parser.c` (and `scanner.c`) with `-shared -fPIC`, and its `tags.scm` | yes (the test builds one; gcc is present; no `tree-sitter` CLI is needed) |
| runtime grammars, macOS `.dylib` | the macOS machine | no |
| release workflow | a tag pushed to a repo with Actions (a scratch repo and a `v0.0.0-rc` tag), GitHub's arm64 Linux and both macOS runners | partly: only on GitHub, with the supervisor's ok |
| `cargo publish` | a crates.io token (`CARGO_REGISTRY_TOKEN`) and the crate name `stewards` free or owned | `cargo publish --dry-run` and `cargo search stewards` only |
| AUR | an AUR account with an SSH key (`AUR_SSH_PRIVATE_KEY` secret) | `makepkg --printsrcinfo` and `makepkg -o` on this Artix machine only |
| bundled `libbaml_cffi` | network to github.com/boundaryml releases to hash each target's library once | yes (download only) |

## Spec debt (DESIGN.md stale against the code as built; task 10 fixes them)

1. Module API "(as built, stage 1)": no `brief`, `submit`, `review`, `llm`, `docs`, `guide`, `harness`, `init`, `telemetry`, `user`, `eval`; `review_due` returns `Vec<DueUnit>`, not `Vec<String>`; `NextReport` lacks `unowned_changed`.
2. First-session walkthrough: `status` prints `{units, uninitialized, stamp_claim}`, not `{units, stale, warnings}`; check every other shape in it against the code.
3. Build order branch names (`feat/stage-1-core` … `feat/stage-4-edges`) and "one PR per stage": work landed as numbered PRs (#4–#18) on topic branches.
4. Handoff "State (2026-11-12)" and "Next action" describe stage-2 branches and an eval rerun that has since happened (2026-11-14).
5. "stewards' own units" table lists 8 units (`cli`…`evals`); `units.toml` has 10 (`jobs`, `setup`, `eval`) with other paths.
6. `.stewards/units.toml` `cli` owns `src/render.rs`, which no longer exists (a `.stewards/` fix, for the supervisor, not task 10).
7. Verification and operations "Evals: drives Claude Code, Codex, and Pi": the built drivers are Pi, Claude Code, Hermes.
8. The `self` suite (Evals) and Handoff say nine adopted units; there are ten.
9. Error table `harness_missing` names `pi` and `claude` only; the Hermes driver also needs `hermes`.
10. Execution says "a `commands.*` entry" (so `lint` too); the code allows `commands.test` and the unit's `test_command` only, and checks a plan step's `check` for length only (01 and 04 settle this).
11. Facts harness table: the column "role-file format (rendered)" names the cut `render`.
12. Command surface `review --pr`: "runs maintainer review briefs" contradicts "never starts a session" (OQ 1).
13. Trust and safety "never the PR … comments" against "a comment updated in place", which must find its own comment (07 settles it).
14. Shape "LLM modes": `--direct` is "for CI and batch", but only `init --draft` honours it.
15. Build order stage 4 ("LLM judges, the Codex driver") and the Waves table row 4 ("judges ‖ drivers") predate OQ 3.
16. Evals "Cadence: … in CI on release tags": no such job exists (OQ 3).
17. Risk register 4's kill criterion depends on the docs-accuracy grader (OQ 3).

## ANSWERS (user, 2026-11-15)
1. `review --pr`: post what `review` reported (option b). 2. Plan steps: do it yourself or ask the maintainer (recommendation). 3. Eval graders: the two cheap report-only checks; drop LLM judges, Codex driver, eval-in-CI. 4. Command flags: keep the prefix rule, document the limit. 5. Jira: optional `JIRA_EMAIL` (supervisor took the recommendation).
All five match what the briefs already build.

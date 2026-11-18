# Picking up the stewards build on another machine

State as of 2026-11-17. Everything merged is on `main`. This folder holds the plan, the writer briefs and the loop scripts the build session used.

## Machine setup
- Rust (stable, edition 2024), `protoc` 28 or newer on PATH (BAML's build needs proto3 optional fields).
- BAML 0.226.3 tooling for regenerating the client: `baml-cli generate --from baml_src`, then `cargo test --lib llm::tests::refresh_llm_schema -- --ignored`, with `BAML_LIBRARY_PATH` pointing at the matching `libbaml_cffi` library. Only needed when `baml_src/` changes.
- `gh` logged in to GitHub.
- For evals: the harness CLIs (`pi` with the pi-subagents extension, `claude`, `hermes`) and their logins. The eval drivers build an isolated config per run; see DESIGN.md Evals. On macOS Claude Code keeps its login in the Keychain, so its eval sessions run in the user's own config directory (user settings and MCP servers off, the user's `CLAUDE.md` still loaded).
- Build once (`cargo build`) and put `target/debug/stewards` on PATH: the review loop calls `stewards` by name.

## Where the work stands
- Stage 1, stage 2, `stewards review`, the eval runner with three drivers, the command runner, distribution, and tree-sitter anchors are merged.
- All ten stage 3 tasks are merged (PRs #22 to #28): stored plans, runtime grammars, `validate --report` and `--export`, PR/MR comments and the CI template, the eval graders, and the spec debt.
- Open PRs: #29 (evals on macOS: leftover cleanup, the Claude Code login, the transcript reader for Claude Code 2.1.326) and #30 on top of it (handoff wording that names no agent type, the eval summaries of pass r9, follow-ups). Both are reviewed; merge #29 first.
- The release is on hold by the user's decision (2026-11-17): the project is not close to a real 0.1.0, and nothing goes to crates.io or the AUR for a while. Do not tag. Until the crate is published, the `cargo install stewards` line in the AGENTS.md block does not work.
- Done from `RELEASING.md` so far: the `release` environment (owner as required reviewer) and a `v*` tag ruleset exist; no secrets are set. Checks 1 to 4 passed on 2026-11-17, the eval pass is below, and the history scrub was pushed on 2026-11-17 (work address and a company name removed from every commit of `main` and the two open branches; trees unchanged). GitHub still serves the old commits through the merged pull request refs.
- Next: `stage4/PLAN.md`, the seven gaps between what is built and a real 0.1.0, in three waves. It is a draft for the user to cut or reorder; nothing in it has started.
- `stage3/FOLLOWUPS.md` lists small open items.

## How a branch goes through review
1. `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.
2. `stewards review --task "<what changed and why>"`. Add any new file to a unit in `.stewards/units.toml` when it says a changed file has no owner.
3. Start one subagent per printed handoff, with exactly that sentence as its task; run `stewards review` again; fix findings; repeat until it exits 0. `loop.js` (a pi-subagents workflow script) and `loop-step.md` automate this: a step worker runs `review` and applies small fixes, reviewers get the handoffs.
4. Tasks that open a trust boundary also get a read-only security review (see PLAN.md).
5. Push, PR, squash-merge when CI is green.

## Models the user chose
Main session on the user's model. Writers: Opus 5.5 (medium thinking) only for contract-heavy tasks, Sonnet 5.5 (high) otherwise. Review-loop reviewers: DeepSeek Flash. Step workers: Haiku 5.5 (high). Security reviews stay on Claude. The Claude usage window, not the CPU, was the limiter on the old machine.

## Eval results
`eval-summaries/` holds the summary of every run from the last four passes (file name: pass and start time). Pass r9 (2026-11-17, macOS, Haiku 5.5, one run per task):
- Pi, medium thinking: fixture pass. Self failed task 3 with the old handoff wording (the model asked for a `general-purpose` agent, which Pi lacks, and stopped); with the new wording task 1 and task 3 pass (the full rerun was cut by a network outage, so task 3 was rerun alone).
- Pi, thinking off: adoption stops after the first `review --roles` round (2 of 2), so no task ran.
- Claude Code, no thinking: fixture pass and self pass, after the transcript reader learned the `Agent` tool and per-child session files.
- Not run: Hermes (not installed on this machine) and DeepSeek Flash (no login here). Earlier pass on the old machine: the review loop completed on Hermes and Pi with DeepSeek Flash (2 of 2 each).
Full transcripts are under `evals/results/` on the machine that ran them.

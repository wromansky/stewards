# Picking up the stewards build on another machine

State as of 2026-11-16. Everything merged is on `main`. This folder holds the plan, the writer briefs and the loop scripts the build session used.

## Machine setup
- Rust (stable, edition 2024), `protoc` 28 or newer on PATH (BAML's build needs proto3 optional fields).
- BAML 0.226.3 tooling for regenerating the client: `baml-cli generate --from baml_src`, then `cargo test --lib llm::tests::refresh_llm_schema -- --ignored`, with `BAML_LIBRARY_PATH` pointing at the matching `libbaml_cffi` library. Only needed when `baml_src/` changes.
- `gh` logged in to GitHub.
- For evals: the harness CLIs (`pi` with the pi-subagents extension, `claude`, `hermes`) and their logins. The eval drivers build an isolated config per run; see DESIGN.md Evals.
- Build once (`cargo build`) and put `target/debug/stewards` on PATH: the review loop calls `stewards` by name.

## Where the work stands
- Stage 1, stage 2, `stewards review`, the eval runner with three drivers, the command runner, distribution, and tree-sitter anchors are merged.
- Two draft PRs hold finished, unreviewed work: #22 (stored plans, step and flow checks; needs the review loop and a spec review) and #23 (runtime-loaded grammars; needs the review loop and a security review of its one `unsafe` block; untested on macOS).
- Remaining tasks, in order: 06 `validate --report`, 07 PR/MR comments and CI template, 08 eval graders, 09 tracker export (GitHub, GitLab, Jira, Linear), 10 spec debt. Briefs are in `stage3/`; `stage3/PLAN.md` has the waves, the user's answers to the open questions, and what cannot be verified without accounts for GitLab, Jira and Linear.
- `stage3/FOLLOWUPS.md` lists small open items.
- Before a release: the one-time repository setup in `RELEASING.md`, an eval pass, and the history scrub of private names (the user's decision: last step before tagging).

## How a branch goes through review
1. `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.
2. `stewards review --task "<what changed and why>"`. Add any new file to a unit in `.stewards/units.toml` when it says a changed file has no owner.
3. Start one subagent per printed handoff, with exactly that sentence as its task; run `stewards review` again; fix findings; repeat until it exits 0. `loop.js` (a pi-subagents workflow script) and `loop-step.md` automate this: a step worker runs `review` and applies small fixes, reviewers get the handoffs.
4. Tasks that open a trust boundary also get a read-only security review (see PLAN.md).
5. Push, PR, squash-merge when CI is green.

## Models the user chose
Main session on the user's model. Writers: Opus 5.5 (medium thinking) only for contract-heavy tasks, Sonnet 5.5 (high) otherwise. Review-loop reviewers: DeepSeek Flash. Step workers: Haiku 5.5 (high). Security reviews stay on Claude. The Claude usage window, not the CPU, was the limiter on the old machine.

## Eval results
`eval-summaries/` holds the summary of every run from the last three passes (file name: pass and start time). Latest self-suite pass on weak models: the review loop completed on Hermes and Pi with DeepSeek Flash (2 of 2 each); the Claude legs were cut off by the usage limit and need rerunning. Full transcripts stayed on the old machine.

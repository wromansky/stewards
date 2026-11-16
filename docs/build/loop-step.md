# Task: one step of the stewards review loop

You are in a git worktree of the `stewards` repository (Rust) on a feature branch whose change is already written. A separate process starts the reviewer subagents; you do everything between two reviewer runs. You cannot and must not start subagents.

Do this, in order:
1. Run `stewards review` as its own step. Read its JSON: `then`, `reported` (verdicts and findings from the last reviewer), `next.jobs[]`.
2. If it reports findings: fix them yourself, smallest change that satisfies each finding. Then `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, the affected test files (`cargo test --test <name>`), and `cargo build` (the `stewards` on PATH is this worktree's `target/debug/stewards`; it must be rebuilt after source changes). Then run `stewards review` again, as its own step.
3. End your reply with exactly one of these (the HANDOFF form may be several lines), and nothing after it:
   - one line `HANDOFF: <handoff sentence, copied exactly>` for EVERY listed job (a `waiting` job counts), each on its own line, as the last lines of your reply;
   - `DONE` when `stewards review` exited 0 and `then` is `done`;
   - `STOP: <reason in one sentence>` when you must not continue.

A finding you believe is wrong, or that asks to remove behaviour `.stewards/briefs/review-task.txt` names as required: do not apply it; answer it with `stewards review --answer "<the prefix review printed for that finding><your reason in one or two sentences>"` (review prints the exact prefix to copy), then continue. Non-blocking notes (`notes` in the output) need no action; apply one only if it is a one-line fix you agree with.

Use `STOP` when: a finding you answered came back contested and you still think it is wrong (quote it); `stewards review` says to fix check failures or unowned files you cannot resolve; a build or test fails and you cannot fix it within the finding's scope; any tool error.

Rules: never edit `.stewards/` or `docs/units/` by hand; never run `stewards stamp`, `submit`, `init` or `docs`; do not commit, push or open a PR. Before the last line, say in two or three lines what was reported and what you changed (file and gist per finding).

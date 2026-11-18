# Stage 4: from "the plan is built" to a real 0.1.0

Source: `main` at `79b68b5` (2026-11-17), the eval pass r9 in `docs/build/eval-summaries/`, and
`docs/build/stage3/FOLLOWUPS.md`. The ten stage 3 tasks are merged. The user's decision of
2026-11-17: the project is not close to a real 0.1.0, and nothing is published to crates.io or the
AUR for a while. This plan lists the seven gaps found while doing the pre-release work, and four
more from a feature comparison with an earlier tool of the same kind. The user's priority
(2026-11-17): weak-model performance comes first, because the intended use is a local model of
about 27B parameters. It is a draft for the user to cut, reorder, or extend; no task has started.

The review process is unchanged (`docs/build/RESUME.md`, "How a branch goes through review").

## The seven gaps

| # | gap | what is true today | evidence |
|---|---|---|---|
| 1 | Nothing has run against a real service | `next --pr`, `review --pr`, `init --ci`, and `validate --export` (GitHub, GitLab, Jira, Linear) are tested against fakes only | `tests/forge.rs`, `tests/tracker.rs`; `stage3/PLAN.md` "cannot be verified" |
| 2 | Adoption and the review loop are fragile on weak models | Pi with Haiku and thinking off stops after the first `review --roles` round (2 of 2); one run per task, one small model | r9 summaries; FOLLOWUPS |
| 3 | The overhead is measured but not judged, and the benefit is not measured | `full` costs about 2 to 4 times `bare` in tokens; the gate checks that the ritual ran, not that the result is better; the bug-fixed and plan graders are report-only | r9 summaries; DESIGN.md Evals |
| 4 | Eval sessions are not isolated on macOS, and one harness is unrun here | Claude Code sessions load the user's `CLAUDE.md` in every arm; Hermes is not installed; Hermes's daemon kill reads `/proc` | `src/eval/claude.rs` `isolate`; FOLLOWUPS |
| 5 | There is no working install path | `cargo install stewards` (named in the AGENTS.md block) fails until the crate is published; the release workflow has never run | `src/guide.rs` block; `RELEASING.md` |
| 6 | Small open items | nine lines in FOLLOWUPS | `stage3/FOLLOWUPS.md` |
| 7 | GitHub still serves the pre-scrub history | the 32 merged pull request refs hold the old commits | FOLLOWUPS, last line |

## Four more gaps, from the comparison

| # | gap | what is true today |
|---|---|---|
| 8 | No named agent per unit | every role is a general subagent given a brief; rendered harness agent files were cut from 0.1.0, so the main agent must pick an agent type itself (it picked a missing one on Pi) |
| 9 | Bug handling is thin | no duplicate handling by root cause, no reuse of earlier evidence when a unit's boundary is unchanged, no test levels by kind of component |
| 10 | Two cases have no rule | a brand-new repository (when to adopt), and a read-only reference repository in a workspace |
| 11 | Nothing written for a person deciding to adopt | README only; no "why", no measured comparison |

## Order

Wave W is first: gaps 2 and 8, aimed at one named local model. Then Wave A (6, 4), which needs
nothing from the user but a go. Wave B needs accounts or money: 1, 3. Wave C is the user's own
action or decision: 5, 7. Wave D is later work: 9, 10, 11. Inside a wave the tasks are independent
unless a task says otherwise.

### Wave W: a local 27B model completes the loop

What is known today, all from Haiku 5.5, which is stronger than the target: with thinking off,
adoption stalls after the first `review --roles` round (2 of 2 on Pi); the main agent asked for an
agent type the harness lacks; one session stopped to ask instead of acting. Nothing has been run on
a local model. `--direct` knows three hosted providers and no local endpoint. The review brief
budget is a constant 48,000 bytes; a role pack's budget is set per role in `[budgets]`.

**W1. Fix the target and make it runnable.** Name the model, its server (Ollama, llama.cpp,
LM Studio), its context window as served, and the harness that will drive it. Then:
- the eval runner accepts that model through the chosen harness (Pi with a local provider is the
  likely first; check whether its subagents work with a local model at all);
- `--direct` gains an OpenAI-compatible endpoint (`[models.direct] <role> = "local/<model>"` with a
  base URL in the user config), so stewards can run a role itself where the harness cannot
  delegate;
- the summary records the context window and tokens per second beside the model.
Done when: `stewards eval --suite fixture --task 1 --arm bare` completes on the local model.
Needs from the user: the model and server; the machine it runs on.
Extra review: security review of the new endpoint setting (a URL the tool sends repo content to).

**W2. Baseline, and a list of how it fails.** Run adoption alone, then the fixture suite at one
run, on the local model. Read every transcript and sort each failure into one of: tool call
malformed; stopped to ask; wrong agent type; context overflow or truncation; typed answer refused
by `submit`; looped; gave up; wrong but confident. Count them.
Done when: the counts are in `docs/build/eval-summaries/` with the summaries. No fixes in this task.

**W3. Fixes, in the order the counts say, each proved against the eval before the next.**
Candidates known now, cheapest first:
- *Fewer decisions for the main agent.* Every `then` line names the exact next command and says
  the round belongs to the same task. `init --draft` states the whole loop once.
- *Named agents (gap 8).* `init` renders one agent file per role for the detected harness, so a
  handoff is "start `<name>` with this sentence" and no agent type is chosen. Restores the cut
  `render` in its smallest form.
- *Briefs that fit.* `[budgets]` and the review brief budget are set from the model's context
  window (a user config key), not constants; a brief over the window is split, never truncated by
  the harness.
- *Answers a weak model can produce.* When `submit` refuses a typed answer, the error names the
  field and shows the smallest valid example for that role; count refusals per role in telemetry.
- *No delegation at all.* Where the harness or model cannot start a subagent, `review --direct`
  runs each brief through the local endpoint itself and the session only reads the findings.
- *Smaller rounds.* Adoption does documenter and validator jobs one unit at a time when the
  window is small.
Done when: adoption completes 3 of 3 and the fixture gate passes on at least 3 of the 4 edit
tasks at `--runs 3`, on the local model, thinking off if the model has the setting.

**W4. Keep it true.** A `weak` suite manifest pinned to the local model, run before any release;
DESIGN.md Evals states the supported floor ("tested on <model> at <window>") and what is known
not to work below it.

Wave W replaces task A3 below.

### Wave A

**A1. Close the follow-ups (gap 6).** One small change each, each with its test:
- the results directory name gets a suffix when the second-resolution name exists, so two evals
  started together never share one;
- find why the plan grader reported `plan stored 0/1` on Pi where Claude Code reported 1/1 (read
  the Pi transcript of fixture task 5 in r9 first: either no plan was stored, or the Pi reader
  misses the architect submit);
- find what the idle Claude Code session waited on after its final message; if it is a background
  child, have the driver end the session when the `result` line arrives;
- the stored review task showing `from none` on a later round, and the `release.yml` shared-runner
  note, as already listed;
- the GitLab CI template installs `glab` or says which image has it.
Done when: FOLLOWUPS holds only items that need a decision.

**A2. Eval isolation on macOS, and Hermes (gap 4).**
- Claude Code: try an isolated config directory that authenticates without the Keychain file
  (`CLAUDE_CODE_OAUTH_TOKEN` passed through when the user has set it; otherwise the present
  fallback stays and the summary keeps saying so). Do not read the Keychain.
- Hermes: `stop_daemon` finds its process through `lsof`/`ps` on macOS, as `kill_leftovers` does;
  un-gate the Hermes leftover test with a non-system fake binary.
- Install Hermes on the eval machine and run the fixture suite once.
Done when: a fixture run on Claude Code records no user `CLAUDE.md` in its sessions when a token is
set, and the Hermes fixture run has a summary.
Needs from the user: whether to install Hermes here; whether to set a token.

**A3. Adoption that a weak model finishes (gap 2).**
- Read the two stalled Pi transcripts: the model treated the second `review --roles` round as
  outside its instruction. Candidates, to be tried one at a time against the eval, cheapest first:
  the `then` line of `review --roles` says the round is part of the same task; `init --draft` ends
  by naming the full loop; adoption splits the documenter and validator jobs into the first round.
- Rerun adoption alone (not the whole suite) on Pi with thinking off until it completes 3 of 3.
- Then one full pass at `--runs 3` on Pi and Claude Code with Haiku, to replace the single runs.
Done when: adoption completes 3 of 3 with thinking off, and the r10 summaries are in the repo.
Cost: about $1 on Pi; Claude Code uses the usage window (about 3 hours for both suites at one run).

### Wave B

**B1. One real run of every outward command (gap 1).** In a scratch repository the user names:
- GitHub: `init --ci` committed, a pull request opened, `next --pr` and `review --pr` each post and
  then update one comment from the workflow's token; `validate --export github` files one issue
  and records its URL; a second export does not file it twice.
- GitLab, Jira, Linear: the same for the merge request comment and for one exported issue each,
  on a free or trial account.
- Every difference from the recorded fakes becomes a fix and a new recorded response.
Done when: `stage3/PLAN.md`'s "cannot be verified" table is empty or each row names why it stays.
Needs from the user: the scratch repository and project names, and logins for `glab`, Jira and
Linear. These runs create visible comments and issues, so each is run only after the user says go.
Extra review: security review of any change to `src/forge.rs` or `src/tracker.rs`.

**B2. Judge the overhead (gap 3).** The question to answer: what does the `full` arm buy for 2 to
4 times the tokens?
- Add graders that measure the result, not the ritual: hidden acceptance tests per fixture task
  (run by the runner after the session, never visible to it), and for the seeded bugs whether the
  review loop reported a bug the `bare` arm shipped.
- Run fixture and self at `--runs 3` on one strong and one weak model.
- Write the answer in DESIGN.md Evals as numbers per arm: tasks correct, seeded bugs caught,
  tokens. If `full` is not better on any of them, say so and name what to cut.
Done when: the summary reports correctness per arm, and DESIGN.md states the measured trade.
Cost: the strong-model pass is the expensive one; estimate it from r9 token counts before running
and ask first.

### Wave C

**C1. An install path that works (gap 5).** The user's decision, three options:
- publish nothing, and change the block's last line to the build-from-source command until a
  release exists (smallest change; recommended while 0.1.0 is far);
- cut a prerelease tag (`v0.1.0-rc.1`): the workflow makes a GitHub prerelease with the four
  archives and `install.sh`, skips the AUR, and the crates.io job stays unapproved. This is also
  the first real run of `release.yml`;
- publish to crates.io.
Done when: the command the block names installs a working binary on Linux and macOS.

**C2. Purge the pre-scrub commits from GitHub (gap 7).** The user's action:
- ask GitHub Support to remove the cached pull request refs and collect garbage, or
- delete the repository and push `main` to a new one of the same name (loses the pull requests;
  the `release` environment and the `v*` tag ruleset must be created again).
Done when: an old commit id from before 2026-11-17 no longer resolves on GitHub.

### Wave D (later)

**D1. Bug handling (gap 9).** A root-cause key on each judged bug, so a second report of the same
cause links to the first instead of filing again; the validator pack shows earlier evidence when
the unit's stamp is unchanged; scenario levels per kind of component. Start only after B1 shows how
real trackers behave.

**D2. Two missing rules (gap 10).** One sentence each in the block and the guide: adopt a new
repository once its layout and tests are stable; in a workspace, a read-only repository is asked
through its own units, not read.

**D3. A page for the person deciding (gap 11).** After B2: what it costs, what it caught, on which
models, from the eval numbers only.

## What a 0.1.0 would then mean

Proposed bar, for the user to change: Wave W done on the named local model, A1, A2 and B1 done,
B2 answered in writing, C1 chosen. C2 and Wave D are independent of the version.

# r10 baseline: how a weaker model fails (stage 4, task W2)

Run `r10-2026-11-18T02-21-41Z`: fixture suite, Claude Code 2.1.326 on macOS, `claude-haiku-4-5`
(the weakest model this install still serves; Haiku 3 and 3.5 are refused), thinking off, one run
per task, no fixes applied. It stands in for a local model of about 27B parameters and is very
likely stronger than one, with a far larger context window: a failure here should be expected
there, a pass here proves nothing there. The user's `CLAUDE.md` was loaded in every arm.

Verdict: fail. Adoption completed (16 subagents, runner check clean). Edit tasks through the gate:
2 of 4 (tasks 2 and 3). All 22 sessions ended normally; none timed out, none hit an API error.

## The main session in each `full` run

| session | ran `status` | ran `review` | subagents started | role answers it submitted itself | changed files | runner check |
|---|---|---|---|---|---|---|
| 004 task 1 | no | no | 0 | 0 | yes | not clean |
| 007 task 2 | yes | yes | 1 | 0 | yes | clean |
| 010 task 3 | yes | yes | 1 | 0 | yes | clean |
| 013 task 4 (read-only) | no | no | 0 | 0 | no | clean |
| 016 task 5 | yes | no | 0 | 0 | no | clean |
| 021 task 5 again (setup of task 6) | yes | yes | 0 | 9 | yes | clean |
| 022 task 6 (read-only) | yes | no | 0 | 0 | yes | clean |

## Failures by kind

| kind | count | where |
|---|---|---|
| Ignored the block: no `stewards` command at all | 2 of 7 | 004 (an edit task: change made, tests added, never reviewed), 013 (read-only) |
| Stopped after planning: wrote a plan as prose and ended, nothing implemented | 1 | 016 (the prompt says "plan the change across them before implementing") |
| Did every role itself instead of delegating: wrote the architect, maintainer and reviewer answers in the main session and submitted them | 1 | 021: 9 submits, 0 subagents, and the runner's check came out clean |
| Repeated submits of one answer file (refused, then retried) | 3 briefs | 021: architect 3 times, two maintainers twice each |
| Tool call malformed, context overflow, loop, timeout | 0 | |

## What this says

1. The first loss is not inside the loop but before it: in 2 of 7 `full` sessions the model never
   looked at the block. Nothing in the tool can catch a session that never calls it; only the
   harness (a hook, a pre-commit check, CI) or the next session's `status` can.
2. Session 021 shows a hole, not only a weak model: one session wrote and submitted the reviewer's
   answer for its own change, and the unit was stamped. The tool cannot tell who submitted. The
   eval's gate would also have passed it on `check_green`; only `handoff` would have failed.
3. Where the model did follow the ritual on a single-unit task (007, 010), one subagent and one
   round were enough.
4. The multi-unit task is where it breaks: once it planned and stopped, once it role-played the
   whole team.
5. The plan grader reported `plan stored 0/1` for 016 correctly (no plan was stored). 021 is a
   setup run and is not graded.

One run per task: every count above is a single observation.

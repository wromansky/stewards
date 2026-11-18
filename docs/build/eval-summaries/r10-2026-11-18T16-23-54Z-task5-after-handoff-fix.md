# eval: incomplete

4 sessions, model claude-haiku-4-5, thinking off, spent null
suite: fixture
results: <repo>/evals/results/2026-11-18T16-23-54Z
Claude Code runs in the user's own config dir ~/.claude (it has no .credentials.json to link: the login is in the Keychain), with `--setting-sources project,local --strict-mcp-config`: no user settings, hooks, plugins, or MCP servers, but the user's CLAUDE.md, rules, skills, agents and commands are loaded in every arm; children on the session's model (CLAUDE_CODE_SUBAGENT_MODEL). Each arm copy with an AGENTS.md also gets a committed one-line CLAUDE.md, `@AGENTS.md`. Thinking is not set by the runner (MAX_THINKING_TOKENS is removed); each session's effective setting is recorded from its thinking blocks. cost_usd: null; the harness's figure is kept as harness_reported_cost_usd.
adoption (001-adopt): ok, main 125326, total 2440799, $null
Units were set by hand for this eval from evals/full-units.toml, so these numbers do not measure `stewards init`'s unit proposal: on its own it proposed cli and left 6 tracked file(s) unowned. Unowned by design: .gitignore, AGENTS.md.
adoption added: AGENTS.md
gaps on the adopted cards (a task whose bug or answer is listed started known): api: bug: Off-by-one error in list_items pagination; api: gap: No input validation on create_item; api: gap: ID generation assumes immutable list; cli: gap: Missing error handling for required create argument; cli: bug: Script paths may fail when binary is installed; cli: gap: No runtime validation of external executables; client: gap: Misleading retry log message on first attempt; client: gap: JSON.parse errors not handled
other models used: 001-adopt: claude-haiku-4-5, claude-haiku-4-5-20251001; 002-full-t5-r1: claude-haiku-4-5, claude-haiku-4-5-20251001; 003-full-t5-r2: claude-haiku-4-5, claude-haiku-4-5-20251001; 004-full-t5-r3: claude-haiku-4-5, claude-haiku-4-5-20251001
git history read: 002-full-t5-r1: git log --oneline | head -5; 002-full-t5-r1: git show 1e324f4:client/client.ts | head -35
The gate, on each full run of an edit task (read-only tasks are n/a): the runner saw a change (`changed`: outside .stewards/ and docs/units/, against the run's starting commit, untracked files included), a `stewards review` command ran somewhere in the session tree (`review`), a subagent was started whose task contains the handoff sentence for a brief under .stewards/briefs/ (`handoff`), and the runner's own `stewards next --no-brief --no-run` exits 0 afterwards (`check_green`). `status`, `route`, `next`, whether every subagent launch holding a handoff gave each child exactly that sentence as its task (`handoff_exact`), and whether a child ran `stewards submit --role reviewer` (`reviewer_submit`) are recorded, not gated. Steps are best-effort from the transcripts: a `stewards` command is seen only as the program of a `;`, `&`, `|`, `(`, `)`, or newline separated segment (after `VAR=value` words), so `timeout`/`env`/`time` prefixes, `if stewards ...`, `{ ...; }`, `bash -c '...'`, and backticks are misread or missed.

| task | arm | runs | not ok | main | total | peak main | files main | files tree | cost | subagents |
|---|---|---|---|---|---|---|---|---|---|---|
| 5 | full | 3 | 0 | 299592 | 3693586 | 64766 | 9 | 14 | null | 2,3,2 |

- task 5: incomplete (gate 2/3): needs both the bare and a full arm; full run 3: the runner's `stewards next --no-brief --no-run` did not exit 0 (overhead, full/bare: main_tokens null, total_tokens null, cost_usd null, peak_main_context null); plan stored 3/3, closed 2/3 (report-only)
- overhead medians across tasks, full/bare (reported, not judged): 

review outcomes (report-only):
- 002-full-t5-r1: round 1 api approve (0 findings, 0 notes, 0 contested, bugs: none, tests ran); round 1 client approve (0 findings, 0 notes, 0 contested, bugs: none, tests ran); round 1 cli approve (0 findings, 0 notes, 0 contested, bugs: none, tests ran); 0 answered
- 003-full-t5-r2: round 1 api approve (0 findings, 0 notes, 0 contested, bugs: none, tests ran); round 1 client changes (1 findings, 0 notes, 0 contested, bugs: none, tests ran); round 1 cli approve (0 findings, 0 notes, 0 contested, bugs: none, tests ran); round 2 client approve (0 findings, 0 notes, 0 contested, bugs: none, tests ran); 1 answered
- 004-full-t5-r3: round 1 api approve (0 findings, 0 notes, 0 contested, bugs: none, tests ran); round 1 client approve (0 findings, 0 notes, 0 contested, bugs: none, tests ran); round 1 cli approve (0 findings, 0 notes, 0 contested, bugs: none, tests ran); 0 answered


## Read by hand (stage 4, W3, first fix: briefs tell the caller to hand them off)

Fixture task 5 (the multi-unit task), `full` arm only, `claude-haiku-4-5`, thinking off, 3 runs.
"incomplete" above is only because no `bare` arm was run.

| | baseline (2 runs of this task) | with the fix (3 runs) |
|---|---|---|
| implemented the change | 1 of 2 (one planned in prose and stopped) | 3 of 3 |
| stored an architect plan | 1 of 2 | 3 of 3 |
| role answers written by the main session | 9 in one run, 0 subagents | 0 in every run; every architect and reviewer answer came from a subagent |
| runner's check clean at the end | 2 of 2 (one of them by self-review) | 2 of 3 |

Run 3 is a new kind of failure: every review approved, then `review` reported a failing plan step
check; the session ran `review` twice more, changed nothing, and reported the work complete.

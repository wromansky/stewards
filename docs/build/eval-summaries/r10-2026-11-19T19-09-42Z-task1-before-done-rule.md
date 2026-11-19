# eval: incomplete

4 sessions, model claude-haiku-4-5, thinking off, spent null
suite: fixture
results: /Users/romansky/stewards/evals/results/2026-11-19T19-09-42Z
Claude Code runs in the user's own config dir /Users/romansky/.claude (it has no .credentials.json to link: the login is in the Keychain), with `--setting-sources project,local --strict-mcp-config`: no user settings, hooks, plugins, or MCP servers, but the user's CLAUDE.md, rules, skills, agents and commands are loaded in every arm; children on the session's model (CLAUDE_CODE_SUBAGENT_MODEL). Each arm copy with an AGENTS.md also gets a committed one-line CLAUDE.md, `@AGENTS.md`. Thinking is not set by the runner (MAX_THINKING_TOKENS is removed); each session's effective setting is recorded from its thinking blocks. cost_usd: null; the harness's figure is kept as harness_reported_cost_usd.
adoption (001-adopt): ok, main 85298, total 2314206, $null
Units were set by hand for this eval from evals/full-units.toml, so these numbers do not measure `stewards init`'s unit proposal: on its own it proposed cli and left 6 tracked file(s) unowned. Unowned by design: .gitignore, AGENTS.md.
adoption added: AGENTS.md
gaps on the adopted cards (a task whose bug or answer is listed started known): api: bug: Pagination off-by-one error; api: gap: ID generation assumes no deletion; api: gap: Missing unit documentation; cli: bug: create command panics on missing name argument; cli: bug: client_script runs TypeScript directly with node; client: bug: Console error message misleads on first attempt; client: bug: Error output discarded when retries exhausted; client: gap: Inconsistent error handling across functions; client: gap: No validation of JSON response format
other models used: 001-adopt: claude-haiku-4-5, claude-haiku-4-5-20251001; 002-full-t1-r1: claude-haiku-4-5, claude-haiku-4-5-20251001; 003-full-t1-r2: claude-haiku-4-5, claude-haiku-4-5-20251001; 004-full-t1-r3: claude-haiku-4-5, claude-haiku-4-5-20251001
git history read: 003-full-t1-r2: git log --oneline -1
The gate, on each full run of an edit task (read-only tasks are n/a): the runner saw a change (`changed`: outside .stewards/ and docs/units/, against the run's starting commit, untracked files included), a `stewards review` command ran somewhere in the session tree (`review`), a subagent was started whose task contains the handoff sentence for a brief under .stewards/briefs/ (`handoff`), and the runner's own `stewards next --no-brief --no-run` exits 0 afterwards (`check_green`). `status`, `route`, `next`, whether every subagent launch holding a handoff gave each child exactly that sentence as its task (`handoff_exact`), and whether a child ran `stewards submit --role reviewer` (`reviewer_submit`) are recorded, not gated. Steps are best-effort from the transcripts: a `stewards` command is seen only as the program of a `;`, `&`, `|`, `(`, `)`, or newline separated segment (after `VAR=value` words), so `timeout`/`env`/`time` prefixes, `if stewards ...`, `{ ...; }`, `bash -c '...'`, and backticks are misread or missed.

| task | arm | runs | not ok | main | total | peak main | files main | files tree | cost | subagents |
|---|---|---|---|---|---|---|---|---|---|---|
| 1 | full | 3 | 0 | 1000598 | 1081418 | 48569 | 6 | 6 | null | 0,1,0 |

- task 1: incomplete (gate 1/3): needs both the bare and a full arm; full run 1: `stewards review` never ran; no subagent was given a handoff; the runner's `stewards next --no-brief --no-run` did not exit 0; full run 3: `stewards review` never ran; no subagent was given a handoff; the runner's `stewards next --no-brief --no-run` did not exit 0 (overhead, full/bare: main_tokens null, total_tokens null, cost_usd null, peak_main_context null)
- overhead medians across tasks, full/bare (reported, not judged): 

review outcomes (report-only):
- 002-full-t1-r1: no reviewer submit; 0 answered
- 003-full-t1-r2: round 1 api approve (0 findings, 0 notes, 0 contested, bugs: none, tests ran); round 1 client approve (0 findings, 0 notes, 0 contested, bugs: none, tests ran); 0 answered
- 004-full-t1-r3: no reviewer submit; 0 answered

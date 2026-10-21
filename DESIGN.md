# stewards — design record

Rebuilt from first principles. Earlier decisions are hypotheses until they
re-appear under **Settled**.

## Goals

Five, all primary, in the user's words:

1. Same behavior across harnesses.
2. Maximum token efficiency.
3. Maximum code cleanliness.
4. Documentation generated from code (possibly a documentation role per unit).
5. Knowledge persists across sessions.

Audience for year one: the author, across their own repos.

Feel: a team to delegate to. The agent's main move is handing work to a
specialist that already knows the area.

What persists: structure (areas, owners, anchors), contracts (interfaces and
expected behavior, with scenarios), and judgment (gaps, shortcuts, decisions).

## Settled

- **Harnesses:** Claude Code, Codex CLI, Cursor, Copilot, Gemini CLI,
  OpenCode, Pi, Hermes, ZeroClaw. All must behave the same.
- **Efficiency:** all three numbers go down (main context per task, total
  tokens per task, repeated work across sessions), and each agent gets the
  exact context its job needs: no more, no less.
- **Docs:** human-readable, rendered per unit, generated from code plus the
  persisted knowledge.
- **Team member:** native subagent files where the harness supports them,
  generated from one source; a handed-over brief elsewhere.

- **Context:** pack by default, pull allowed. The tool computes the minimum
  pack for unit + role + task (card sections, anchors the task touches, source
  at those anchors); the brief states a budget; the subagent may pull more by
  anchor and must report what it added.
- **Writing knowledge:** both. Every delegated job ends by submitting a typed
  update the tool validates and writes; a periodic regen reconciles with code
  and git.
- **Docs:** one knowledge file per unit holds structured facts and prose
  sections. The documenter writes only the prose, through the tool.
  `docs/<unit>.md` is rendered from it on change.
- **Roles:** maintainer, tester, documenter, validator.
- **Agent files:** one native agent per role per harness (four files), the
  delegation message carries the unit's pack. Briefs for harnesses without
  file-defined subagents.
- **Validator:** reproduction first. A report must carry a repro command; a
  failing run is proof. The validator assigns severity from evidence, and
  judges by reasoning only when no runnable repro exists.

- **Delivery:** the repo's AGENTS.md block is the one truth: the ritual, the
  four roles, how to delegate. Every harness reads it with no install.
  User-level hooks and instructions are optional accelerators. ZeroClaw gets
  the same block in its workspace AGENTS.md.
- **Units:** manifests, directory layout, and git co-change history propose
  boundaries; the LLM names and describes each; a units file is written for
  the user to edit before anything is drafted.
- **Format:** TOML. Typed tables for anchors, interface, scenarios, gaps;
  multi-line strings for prose.
- **Clean means:** Karpathy's four principles as the editing stance; the
  Ponytail ladder named per finding in maintainer review; `simplified:` debt
  tracked per unit; touched files lint clean and every non-trivial change
  leaves one runnable check.

- **Stack:** Rust CLI, BAML for every LLM step. Rust owns facts (units,
  anchors, packs, staleness, debt, rendering). BAML defines each role's prompt
  and typed answer schema, so every job's output is validated the same way in
  every harness.
- **LLM:** the harness's own agents think by default: the tool prints the
  brief, the agent answers, the tool validates. `--direct` makes the tool call
  the model via BAML with a key from env, for CI and batch. Missing key is an
  error.
- **Job loop:** the brief ends with: write your answer to a file, run
  `stewards submit <unit> --role <r> --file <f>`, fix and retry on a
  validation error. The main agent receives only the printed summary.
- **Staleness:** each unit file records the commit it was last reviewed at.
  `check` fails when the unit's files changed after that sha with no submit
  since, an anchor's file or symbol no longer resolves, or a scenario
  references an unknown interface id. Uncovered interfaces warn. Runs in the
  ritual and in CI.

- **Triggers:** before changing, the maintainer of the units the task names
  (explain and plan). Before done, `stewards next` maps the diff to units and
  prints the jobs: maintainer review for every touched unit; tester when an
  interface anchor changed or a scenario is affected; documenter when
  prose-relevant facts changed; validator only when a bug report exists. The
  AGENTS.md block says "run `stewards next` and do what it prints".
- **Workspaces:** day one. `stewards.toml` with `repos = [...]` in the parent
  folder. Read commands span all repos with units named `<repo>/<unit>`;
  write commands act on one repo.
- **Distribution:** `cargo install stewards`; GitHub releases with
  `libbaml_cffi` bundled and an install script; AUR package; a thin Claude
  Code plugin in the same repo (SessionStart hook runs `stewards guide`, or
  prints an install hint plus bundled guide text when the binary is missing;
  a `stewards` skill).
- **Self-test:** Rust tests against fixture repos in `tests/fixtures/`; BAML
  `test` blocks run by `baml-cli test` in CI with a key.

- **Renderers (v1):** Claude Code `.claude/agents/*.md` (Cursor reads it),
  Codex `.codex/agents/*.toml`, Copilot `.github/agents/*.agent.md`, Gemini
  CLI `.gemini/agents/*.md`, OpenCode `.opencode/agents/*.md`, and Pi via
  `pi-subagents` at `.pi/agents/*.md`. Hermes and ZeroClaw use briefs.
- **Docs path:** `docs/units/<unit>.md`, committed; configurable in
  `repo.toml`.
- **Debt:** computed on demand by `stewards debt`, attached to the owning
  unit by route; never stored.
- **License:** MIT, no attribution section.
- **Repo-level knowledge, no oracle agent.** `repo.toml` holds `[[flow]]`
  (name, the ordered units it crosses, entry anchor) and `[[decision]]` (why
  a boundary or convention exists), next to repo-wide facts (test roots,
  lint and test commands, primary paths, docs path). `pack` includes the
  flows that touch the unit. The two moments that need repo-level judgment,
  `route` on a path with no owner and `init --draft` for a new unit, pack
  flows and decisions into their LLM step. An architect role (pack = unit
  summaries + flows + decisions) is deferred until a real "explain the
  architecture" or cross-cutting plan has no maintainer to go to.

## Command surface

| Command | Does |
|---|---|
| `guide` | prints the ritual, the roles, and the editing rules |
| `init` | proposes `units.toml` from facts; `init --draft` names units, drafts files, writes the AGENTS.md block |
| `route <path>` | owning unit and file; with no owner, an LLM step packed with flows and decisions proposes one |
| `pack <unit> --role <r> [--diff]` | the context pack: card sections, anchors, source at anchors |
| `brief <unit> --role <r>` | role prompt + pack + answer schema + submit instructions |
| `submit <unit> --role <r> --file <f>` | validates a typed answer with BAML, writes the update, bumps the reviewed sha |
| `next` | maps the diff to units, runs `check`, prints the jobs to run |
| `check` | fails on stale units, dead anchors, unknown interface ids; warns on uncovered interfaces |
| `sync` | regen pass: reconciles unit files with code and git |
| `validate --report <f>` | runs the repro command; non-zero exit is proof; then severity |
| `debt` | `simplified:` comments attached to units |
| `docs [unit]` | renders `docs/units/<unit>.md` |
| `render --harness <h>` | writes the four native role agents for a harness |
| `status` | units, coverage, freshness, across the workspace |

JSON by default, `--human` for people. `--direct` makes any LLM step call
the model through BAML.

## Settled in v1 (formerly deferred)

All three are in v1: the architect role with tracked plans; tree-sitter
anchors (static core grammars for rust, python, typescript, javascript, go,
bash, c, cpp, c-sharp, hcl, yaml; runtime extras from `~/.stewards/grammars/`
via libloading; definitions from each grammar's `tags.scm`, custom queries
for YAML, HCL, Bash; text fallback for BAML and unknown languages); evals with
the fixture, all five graders, and drivers for Claude Code, Codex, and Pi.

### Architect role

- Invoked by the main agent when a task spans units: `route` on the files or
  areas the task names resolves to 2+ units or none → brief the architect
  first. `next` flags a diff that touched 2+ units with no plan recorded.
- Pack: each unit's purpose line and interface ids, every `[[flow]]`, every
  `[[decision]]`. No anchors, no source.
- Submits: a plan (ordered per-unit steps, each with unit, change, affected
  interface ids, the check that proves it); `[[flow]]` and `[[decision]]`
  updates; unit boundary proposals written to `units.toml` for the user to
  confirm, never applied. Explain mode submits nothing.
- Plans are state: `submit` writes `.stewards/plans/<id>.toml`; `next` maps
  the diff to steps, reports which are done, and tells each maintainer which
  step it reviews. A plan closes when every step's check passes.

### Anchor parsing

- tree-sitter for: Rust, Python, TypeScript/JavaScript, Go, Bash, BAML,
  Terraform (HCL), YAML, C, C++, C#, and more. Text fallback elsewhere.
- Grammar crates on crates.io (checked 2026-10-20): tree-sitter 0.27.1 and
  grammars for rust, python, typescript, javascript, go, bash, c, cpp,
  c-sharp, hcl, yaml all exist. No `tree-sitter-baml` crate.
  `tree-sitter-loader` 0.27.1 can compile grammars from source at runtime
  (needs a C compiler); prebuilt grammar `.so`/`.dylib` files can be loaded
  with `libloading`.
- Linking: static core + runtime extras (see above).

### Evals

- Metrics, one per goal: tokens bare vs stewards (main and total); ritual
  steps executed per harness, parsed from each transcript; cleanliness rubric
  on the diff by an LLM grader; docs accuracy against code; second-session
  reuse scored on files opened and tokens.
- Harnesses driven: Claude Code, Codex, Pi. Manual spot checks elsewhere.
- Fixture: a small purpose-built polyglot repo under `evals/fixture/` with
  seeded bugs and a task backlog.
- Cadence: on demand, and in CI on release tags.
- Drivers (verified 2026-10-20 on claude 2.1.305 and codex-cli 0.154.0; Pi
  from docs only, not installed here):
  - Claude Code: `claude -p "<prompt>" --output-format stream-json --verbose
    --model <m> --max-turns N --max-budget-usd X --no-session-persistence`.
    One JSON object per line; `assistant` lines carry `tool_use` blocks and
    `message.usage`; the final `result` line carries `usage`, `modelUsage`,
    `total_cost_usd`, `subagent_stats`. `--bare` skips hooks, CLAUDE.md,
    plugins. `claude plugin eval` exists (cases under `evals/<case>/`, six
    grader types, with/without ablation, `--json`) but graders are fixed
    types and runs start in an empty workspace with `scaffold_script`.
  - Codex: `codex exec --json --skip-git-repo-check -C <dir> -s
    workspace-write -m <model> "<prompt>"`. Events: `item.completed`
    (`command_execution`, `agent_message`, ...), `turn.completed{usage}`
    (sum across turns; no total). Rollout at
    `~/.codex/sessions/YYYY/MM/DD/rollout-<ts>-<thread_id>.jsonl` with
    `function_call`/`function_call_output` pairs and `token_count` events.
    Local blocker: this machine's ChatGPT-account Codex rejects the configured
    model ids with 400; API-key auth or an accepted model id is needed before
    the driver can be tested here.
  - Pi: `pi --mode json "<prompt>"` for JSONL events (`tool_execution_start`
    /`_end`, `message_update.usage`, `message_end`); `--session-dir`,
    `--session-id` to control the transcript; session files under
    `~/.pi/agent/sessions/`. Assistant `message.usage` has
    `{input, output, cacheRead, cacheWrite, totalTokens, cost}`. Package is
    `@mariozechner/pi-coding-agent` (0.73.1) or `@earendil-works/pi-coding-agent`
    (1.3.0); which is current is unverified.

### Also in v1 (round S)

- Git and PR integration, all four pieces: a pre-push hook that runs `check`
  (offered by `init`); `next --pr <n>` posts the jobs as a PR comment updated
  in place; `review --pr <n>` runs maintainer review briefs on the PR diff
  and posts findings with ladder rungs as review comments; `init` writes a
  CI workflow template that runs `check` and `next --pr`. GitHub and GitLab
  both: the remote host picks `gh` or `glab`; the CI template has a GitHub
  Actions and a GitLab CI variant.
- Telemetry: `.stewards/telemetry/*.jsonl`, gitignored, one line per
  submit: unit, role, pack bytes and anchor count, pulled anchors, tokens the
  subagent reported (null when the harness hides them), harness, timestamp.
  `status --telemetry` summarizes.
- Memory: `~/.stewards/memory.toml`, hand-edited one-liners (style, tools,
  never-do) with optional language or role filters; roles never write it;
  filtered entries packed into every brief under a fixed byte budget.
- Export: validated bugs to GitHub Issues, Jira, and GitLab Issues.
  `repo.toml` `[tracker] kind = "github"|"gitlab"|"jira"` plus the project
  or repo id. Auth from `gh`, `glab`, and `JIRA_URL`/`JIRA_TOKEN` env; no
  secrets in any file. Dedupe by repro command: search before create.
- Onboarding humans: `docs/units/README.md` index rendered with flows and
  decisions, the same map the agents use.
- Pack budgets: `repo.toml` sets per-role byte budgets; `pack` reports when
  it truncated and what it dropped.
- Shell completions (`stewards completions <shell>`) and a man page, shipped
  in releases and the AUR package.
- Schema version: `repo.toml` carries `schema = N`; the tool refuses newer
  schemas and migrates older ones in place.
- Model per role: `~/.stewards/config.toml` maps role → model alias per
  harness; `render` writes the pin; `--direct` uses the same table. Repo
  files never name a model.
- Repro safety: a repro must start with one of the repo's declared test
  commands in `repo.toml`, runs from the repo root with a timeout; anything
  else is rejected with the reason.
- Persistence is measured by the eval's second-session grader only.

## v1 scope (final)

Everything under Settled, Settled in v1, and Also in v1. Nothing deferred.

## Build order

1. Core, no LLM: TOML schemas, workspace resolution, `route`, `check` (text
   anchors), `status`, `debt`, `pack`, `next`. Fixture tests for each.
2. Roles: BAML functions, `brief`/`submit`, `init` (two steps), `sync`,
   `docs`, `guide`, memory, telemetry. Dogfood starts here: stewards gets its
   own `.stewards/` and every later stage goes through its own ritual.
3. Depth: architect + plans, `validate` + tracker export, tree-sitter
   anchors, `render` for six harnesses.
4. Edges: git hooks, PR/MR comments, CI templates, evals with three drivers,
   Claude Code plugin shell, completions and man page, release packaging
   (cargo, GitHub releases with `libbaml_cffi`, AUR).

## Open

(none)

## Prior hypotheses (not yet re-admitted)

Rust + BAML CLI; brief/accept loop; two TOML cards per unit; maintainer and
tester roles; validate by reproduction; `check` on reviewed sha; `next` from
git diff; `debt` from `simplified:` comments; workspaces via `stewards.toml`;
thin Claude Code plugin; cargo + GitHub releases + AUR; MIT.

## Facts

- 6 of 7 harnesses support file-defined subagents with fresh context and a
  model pin: Claude Code (`.claude/agents/*.md`), Codex (`.codex/agents/*.toml`),
  Copilot (`.github/agents/*.agent.md`), Cursor (`.cursor/agents/`, also reads
  `.claude/` and `.codex/`), Gemini CLI (`.gemini/agents/*.md`), OpenCode
  (`.opencode/agents/*.md`). Aider has none. Ad-hoc subagents from a free-form
  prompt: Claude Code, Codex, Copilot VS Code only. AGENTS.md says nothing
  about subagents.
- Pi (TypeScript): reads AGENTS.md/CLAUDE.md in cwd and parents, user-level
  in `~/.pi/agent/`; Agent Skills from `.agents/skills/`, `~/.agents/skills/`,
  `.pi/skills/`; no subagents; session-start only via a TS extension; shell yes.
- Hermes (Python): first match of `.hermes.md` > `AGENTS.override.md` >
  `AGENTS.md` > `CLAUDE.md`; skills from `~/.hermes/skills/`, `.agents/skills/`
  (after `hermes skills trust`); ad-hoc `delegate_task` only; shell hook
  `hooks.on_session_start[].command` in `~/.hermes/config.yaml`; shell yes.
- ZeroClaw (Rust): general agent runtime, not a coding CLI. Reads AGENTS.md
  only from its own agent workspace, not the repo cwd; `SKILL.md`/`SKILL.toml`
  in the workspace; ephemeral `spawn_subagent` or `delegate` to another
  configured agent; no shell hook found; shell yes, policy-gated.
- pi-subagents (`nicobailon/pi-subagents`, `pi install npm:pi-subagents`):
  agents at `.pi/agents/**/*.md` (project) and `~/.pi/agent/agents/**/*.md`
  (user); md + YAML frontmatter; fields include `name`, `description`,
  `model`, `tools`, `excludeTools`, `inheritProjectContext`, `defaultContext`
  (`fresh`|`fork`), `memory`. Clean system prompt by default. No ad-hoc
  launch without a definition.
- BAML Rust: cold debug build 10.8s, binary 468 KB. The crate `dlopen`s
  `libbaml_cffi`, auto-downloaded on first LLM use; `BAML_LIBRARY_PATH`
  overrides. Generated client has `build_request()` and `parse()`.

# stewards — design record

The single record of what stewards is and why. Decisions here are settled;
reopen one by editing this file, not by drifting in code.

## Goals

Five, all primary, in the author's words:

1. Same behavior across harnesses.
2. Maximum token efficiency.
3. Maximum code cleanliness.
4. Documentation generated from code.
5. Knowledge persists across sessions.

Audience for year one: the author, across their own repos.

Feel: a team to delegate to. The agent's main move is handing work to a
specialist that already knows the area.

What persists: structure (areas, owners, anchors), contracts (interfaces and
expected behavior, with scenarios), and judgment (gaps, shortcuts, decisions).

## Settled decisions

### Shape

- **Harnesses:** Claude Code, Codex CLI, Cursor, Copilot, Gemini CLI,
  OpenCode, Pi, Hermes, ZeroClaw, Borg (adelphidata/borg). All must behave
  the same.
- **Stack:** a Rust CLI that agents call through their shell tool. No MCP
  server. Rust owns facts (units, anchors, packs, staleness, debt,
  rendering). BAML defines each role's prompt and typed answer schema, so
  every job's output is validated the same way in every harness.
- **LLM modes:** the harness's own agents think by default: the tool writes
  a brief, the agent answers, the tool validates. `--direct` (or
  `STEWARDS_DIRECT=1`) makes the tool call the model through BAML with a key
  from the usual env vars, for CI and batch. A missing key is an error,
  never a silent fallback.
- **Delivery:** the repo's `AGENTS.md` block is the one truth: the ritual,
  the roles, the editing rules, how to delegate. Every harness reads it with
  no install. User-level hooks and instructions are optional accelerators.
  ZeroClaw gets the same block in its workspace `AGENTS.md`. `init` places
  the block at the top of `AGENTS.md`: Borg truncates project docs at 32 KiB
  across the tree and Hermes takes only the first matching context file.
- **Efficiency:** three numbers go down (main context per task, total tokens
  per task, repeated work across sessions), and each agent gets the exact
  context its job needs: no more, no less.
- **License:** MIT, no attribution section. Clean room: no text from the
  adelphi-adlc-plugin repo.

### Team

- **Roles, five per repo, one set per unit:** maintainer (explains, plans,
  reviews the diff for cleanliness and surgical scope, keeps the card
  current), tester (owns the contract and its scenarios, writes and runs
  tests, never edits source), documenter (writes the prose sections of the
  unit's card), validator (judges a bug from its reproduction, severity from
  evidence only), architect (plans changes that cross units, records flows
  and decisions, proposes unit boundaries).
- **Team member, physically:** native subagent files where the harness
  supports them, one file per role per harness, generated from one source by
  `render`; the delegation message carries the unit's pack. A handed-over
  brief elsewhere. Renderers in v1: Claude Code `.claude/agents/*.md` (Cursor
  reads it), Codex `.codex/agents/*.toml`, Copilot `.github/agents/*.agent.md`,
  Gemini CLI `.gemini/agents/*.md`, OpenCode `.opencode/agents/*.md`, Pi via
  `pi-subagents` at `.pi/agents/*.md`. Hermes, ZeroClaw, and Borg have no
  file-defined subagents (verified in source, see Facts) and use briefs: the
  brief carries the role text, and their native spawn tool is told to read
  the brief path. An optional ZeroClaw renderer (a `config.toml` fragment
  with `[agents.<role>]` entries plus per-role workspace dirs holding the
  role prompt as `AGENTS.md`) may follow in stage 3; Borg roles live only in
  its database, so none is planned.
- **Render target:** user-level dirs with model pins from
  `~/.stewards/config.toml` (`~/.claude/agents/`, `~/.codex/agents/`,
  `~/.pi/agent/agents/`, `~/.gemini/agents/`, `~/.config/opencode/agents/`).
  `--repo-level` writes the repo-level dirs with no `model` field. Copilot
  is repo-level only (no verified user path).
- **Model per role:** `~/.stewards/config.toml` maps role → model alias per
  harness; `render` writes the pin; `--direct` builds a BAML `ClientRegistry`
  from the same table. Repo files never name a model.
- **Context:** pack by default, pull allowed. The tool computes the minimum
  pack for unit + role + task; the brief states a budget; the subagent may
  pull more by anchor and must report what it added in `pulled_anchors`.
- **Job loop:** `next` writes briefs to `.stewards/briefs/` and prints their
  paths; the main agent hands a path to a subagent that can run shell; the
  subagent writes its answer to a file and runs `stewards submit <unit>
  --role <r> --file <f>`, fixing and retrying up to three times on a
  validation error; the main agent receives only the one-line summary submit
  prints. Every LLM step uses this loop, including `init --draft` and `sync`.
- **Triggers:** before changing, `route` the files and brief the owning
  maintainer to explain and plan; two or more units, or no owner, brief the
  architect first. Before reporting done, `next` maps the diff to units and
  writes the jobs: maintainer review for every touched unit; tester when an
  interface anchor's file or a file under `test_roots` changed; documenter
  when an interface anchor's file changed; architect when two or more units
  changed with no plan recorded; validator only when a bug report exists.
  Done means `check` is green.
- **Delta re-review:** when a unit was stamped earlier in the same `next`
  cycle, the maintainer's pack carries only the diff since that stamp.
- **Task text:** `next` sets each job's task to the branch name plus the
  last commit subject, overridable with `--task`; `brief --task` for ad-hoc
  briefs.

### Knowledge

- **Units:** manifests, directory layout, and git co-change history propose
  boundaries (`init`); the LLM names and describes each (`init --draft`); the
  units file is written for the user to edit in between.
- **Format:** TOML. Typed tables for anchors, interface, scenarios, gaps;
  multi-line strings for prose. Two files per unit: the card and the test
  card. Interface entries with ids live in the card; scenarios reference them.
- **Writing knowledge:** both ways. Every delegated job ends in a typed
  `submit` the tool validates and writes; `sync` regenerates stale units from
  code and git. Agents never hand-edit cards.
- **Staleness:** `[reviewed]` records a content fingerprint (sha256 over the
  unit's tracked file blobs), the commit for display, and the stamped tree.
  `check` fails when the fingerprint differs with no stamp since, an anchor
  no longer resolves, or a scenario references an unknown interface id.
  Uncovered interfaces warn. Runs in the ritual and in CI.
- **Anchors:** `path::name`. Resolved by text in stage 1 (file exists, whole
  word match). With tree-sitter (stage 3), `name` must be a definition per
  the grammar's `tags.scm`; a mere mention is dead. Static grammars for rust,
  python, typescript, javascript, go, bash, c, cpp, c-sharp, hcl, yaml;
  runtime extras from `~/.stewards/grammars/<lang>/` via `libloading`;
  custom queries for YAML keys, HCL blocks, Bash functions; text fallback
  for BAML and unknown languages.
- **Repo-level knowledge, no oracle agent:** `repo.toml` holds `[[flow]]`
  and `[[decision]]` next to repo-wide facts. `pack` includes the flows that
  touch the unit. The architect's pack is every unit's purpose and interface
  ids plus all flows and decisions; no anchors, no source.
- **Plans:** the architect's steps are state in `.stewards/plans/<id>.toml`;
  `next` runs each touched step's allowlisted check, marks pass or fail, and
  closes the plan when all pass.
- **Debt:** `simplified:` comments are computed on demand by `debt` and
  attached to the owning unit by route; never stored.
- **Docs:** `docs/units/<unit>.md` is rendered from the card; the documenter
  writes only the prose sections; `docs/units/README.md` is the human index
  with flows and decisions.
- **Memory:** `~/.stewards/memory.toml`, hand-edited one-liners with optional
  language or role filters; roles never write it; matching entries are
  packed into every brief under a byte budget.
- **Workspaces:** `stewards.toml` with `repos = [...]` in a parent folder.
  Read commands (`route`, `status`, `debt`, `check`, `next`) span all repos
  with units named `<repo>/<unit>`; write commands act on one repo.

### Cleanliness

- Karpathy's four principles as the editing stance: think first,
  simplicity, surgical, goal-driven.
- The Ponytail ladder is the procedure behind simplicity, and the maintainer
  names the rung each finding fails: not needed, already in the repo,
  stdlib, platform, installed dependency, one line, untraced, no check.
- Every non-trivial change leaves one runnable check that fails if the
  logic breaks; the tester owns scenario tests.
- Hygiene: branch off `main`, commit only when asked, lint touched files
  clean, no disables.
- A deliberate shortcut is a `simplified:` comment naming its ceiling and
  upgrade path.

### Verification and operations

- **Validation:** a bug report must carry a repro command; `validate` runs
  it through the allowlist; a failing run is proof; the validator assigns
  severity from evidence, and judges `real` by reasoning only when no
  runnable repro exists. `--export` creates an issue on GitHub, GitLab, or
  Jira (`repo.toml` `[tracker]`; auth from `gh`, `glab`, `JIRA_URL` and
  `JIRA_TOKEN`), deduped by repro string.
- **Git and PR:** `init --hooks` installs a pre-push hook running `check`;
  `next --pr <n>` posts the jobs as a PR or MR comment updated in place;
  `review --pr <n>` runs maintainer review briefs on the diff and posts
  findings with rungs; CI templates for GitHub Actions and GitLab CI. The
  remote host picks `gh` or `glab`.
- **Telemetry:** `.stewards/telemetry/<yyyy-mm>.jsonl`, gitignored, one line
  per submit: unit, role, task, pack bytes and anchor count, pulled anchors,
  tokens the subagent reported (null when hidden), harness, timestamp.
- **Evals:** `stewards eval` drives Claude Code, Codex, and Pi over a
  purpose-built fixture, three arms, graders for all five goals; on demand
  and on release tags. Manual spot checks for the other harnesses.
- **Self-test:** Rust tests against fixture repos in `tests/fixtures/`; BAML
  `test` blocks run by `baml-cli test` in CI with a key.
- **Distribution:** `cargo install stewards`; GitHub releases with the
  matching `libbaml_cffi` beside the binary and an install script; an AUR
  package; shell completions and a man page; a thin Claude Code plugin in
  this repo whose SessionStart hook runs `stewards guide` or, with no binary,
  prints an install hint and the bundled guide text.
- **Versioning:** semver on the CLI contract (major for a JSON output or
  command change that breaks agents, minor for new commands or fields);
  `repo.toml` `schema` is an integer, older schemas migrate in place on
  first write, newer are refused with the fix named.

## Glossary

- **unit**: a part of a repo that changes together; owns paths; has one
  maintainer, tester, and documenter.
- **card**: a unit's knowledge file, `.stewards/units/<unit>.toml`; **test
  card**: `.stewards/units/<unit>.test.toml`.
- **anchor**: `path::name`, a place in source a card points at.
- **interface**: an entry in a card naming a contract the unit promises,
  with an id scenarios reference.
- **scenario**: a test-card entry proving one interface behavior.
- **gap**: a known shortfall or future item on a card; **debt**: a
  `simplified:` comment in code, computed, never stored.
- **flow**: an ordered chain of units a request crosses; **decision**: why a
  boundary or convention exists. Both live in `repo.toml`.
- **pack**: the exact context the tool computes for a unit, role, and task.
- **brief**: a file holding role instructions, the pack, memory, the answer
  schema, and submit instructions; handed to a subagent by path.
- **submit**: the command that validates a typed answer and writes it.
- **stamp**: recording that a unit was reviewed at its current fingerprint.
- **fingerprint**: sha256 over the unit's tracked file blobs; staleness is a
  fingerprint mismatch.
- **job**: one (unit, role) task `next` decides is due.
- **plan**: an architect's ordered steps across units, tracked by `next`.
- **ritual**: entering → before change → before done.
- **arm**: an eval configuration (`bare`, `block-only`, `full`).

## Schemas (`.stewards/`)

`repo.toml`

```toml
schema = 1
name = "backend"
test_roots = ["tests"]
docs_path = "docs/units"
[commands]
test = ["pytest", "cargo test"]          # allowlist prefixes: repro, step checks, scenarios
lint = ["ruff check"]
timeout_seconds = 600                    # per run; an entry may be { cmd = "...", timeout_seconds = N }
[budgets]                                # bytes of the --human rendering, per role
maintainer = 24000
tester = 24000
documenter = 16000
validator = 8000
architect = 16000
memory = 2000                            # user memory entries appended to every brief
[pack]
exclude = [".env*", "*.pem", "*.key", "*secret*", "*credential*", "id_rsa*"]
[[flow]]
name = "create-item"
units = ["api", "store"]
entry = "src/api/items.py::create_item"
[[decision]]
title = "store has no HTTP knowledge"
why = "..."
[tracker]                                # optional
kind = "github"                          # github | gitlab | jira
project = "owner/repo"
```

`units.toml` (boundaries and purpose; the user-edited proposal, one truth)

```toml
[[unit]]
name = "api"
purpose = "HTTP surface"
paths = ["src/api/**", "tests/api/**"]
```

`units/<unit>.toml` (maintenance + prose; `purpose` comes from `units.toml`)

```toml
unit = "api"
[reviewed]                               # absent = never reviewed
commit = "f0b4435"                       # informational
fingerprint = "sha256:..."               # what check compares
tree = "<git write-tree sha>"            # the tree stamped; delta re-review diffs against it
[[interface]]
id = "create-item"
anchor = "src/api/items.py::create_item"
contract = "POST /items; 201 with body; 422 on invalid"
[[anchor]]
path = "src/api/items.py"
name = "router"
[[gap]]
kind = "gap"                             # gap | future
title = "..."
anchor = "src/api/items.py::create_item"  # optional
note = "..."
[doc]                                    # documenter-owned prose
overview = """..."""
how_it_works = """..."""
how_to_change = """..."""
```

`units/<unit>.test.toml`

```toml
unit = "api"
primary_paths = ["create-item"]          # interface ids the user declared
test_command = "pytest tests/api"
[[scenario]]
id = "create-ok"
interface = "create-item"                # must exist in the unit file
given = "valid body"
expect = "201 and the item is stored"
level = "integration"
command = "pytest tests/api/test_items.py::test_create_ok"   # optional; run through the allowlist, never parsed for paths
```

`plans/<id>.toml` (stage 3)

```toml
title = "add priority end to end"
created = "2026-10-21"
[[step]]
unit = "api"
change = "add priority to the item schema and endpoints"
interfaces = ["create-item", "list-items"]
check = "pytest tests/api -k priority"   # must start with a commands.test prefix
```

`stewards.toml` (workspace, in a parent folder)

```toml
repos = ["backend", "frontend"]          # paths relative to this file
```

User-level files: `~/.stewards/config.toml` (`[models.<harness>] <role> =
"<alias>"`), `~/.stewards/memory.toml` (`[[entry]] text, languages?,
roles?`), `~/.stewards/grammars/<lang>/{lib.so|.dylib, tags.scm}`.

Anchor grammar: `path` is the text before the first `::`, `name` is the rest
(Rust names contain `::`). A text match is an occurrence of `name` whose
neighbouring chars are not `[A-Za-z0-9_]`; case-sensitive; hand-rolled char
check, no regex crate.

Fingerprint: sha256 over the sorted list of `(path, blob)` for tracked files
under the unit's paths, where `blob` is from `git ls-files -s` and, for dirty
or untracked files, `git hash-object`.

## Command surface

| Command | Does |
|---|---|
| `guide [--harness h] [--all] [--errors] [--glossary]` | prints the ritual, roles, editing rules, delegation text for the detected harness, error codes, glossary |
| `init [--draft] [--hooks] [--ci]` | step 1 proposes `units.toml` and `repo.toml` from facts and writes the AGENTS.md block; `--draft` briefs the naming and card drafting; `--hooks` installs the pre-push hook; `--ci` writes the CI template |
| `status [--telemetry]` | units, freshness, coverage, gaps, across the workspace; uninitialized repos |
| `route <path>...` | owning unit per path; with no owner, an LLM step packed with flows and decisions proposes one |
| `pack <unit> --role <r> [--diff] [--task t]` | the context pack |
| `brief <unit> --role <r> [--task t] [--stdout]` | writes the brief file, prints its path and size |
| `submit <unit> --role <r> --file <f> [--attempt n]` | validates a typed answer with BAML, writes the update, stamps |
| `stamp <unit>` | records `[reviewed]` from the current tree |
| `next [--base ref] [--task t] [--no-brief] [--no-run] [--pr n]` | maps the diff to units, runs `check` and plan step checks, writes the briefs, prints the jobs |
| `check` | failures: stale units, dead anchors, unknown interface ids; warnings: uncovered interfaces, overlapping globs |
| `sync` | refresh briefs for every stale unit's maintainer and documenter; unit proposals for new dirs |
| `validate --report <f> [--export]` | runs the repro through the allowlist; validator brief for severity; issue export |
| `review --pr <n>` | maintainer review briefs on a PR or MR diff; findings posted as review comments |
| `debt` | `simplified:` comments attached to units |
| `docs [unit]` | renders `docs/units/<unit>.md` and the index |
| `render --harness <h> [--repo-level]` | writes the five native role agents |
| `eval [--harness h] [--arm a] [--task n] [--max-cost usd]` | runs the eval suite |
| `completions <shell>` | shell completions |

Global flags: `--human`, `--repo <name>`, `-C <dir>`, `--direct`. JSON by
default, markdown with `--human`. Exit 0 ok, 1 `check` failures, 2 errors.

## Rules

### Pack

```
Pack { unit, role, task, budget, bytes, over_budget, purpose, interface[],
       anchors[{path, name, line, source}], scenarios[], gaps[], flows[],
       doc, changed_files[], hunks[{path, diff}], other_changed_files[],
       memory[], repro_result, dropped[] }
```

Absent sections are `null`. `--diff` restricts `anchors` to those whose path
is in `changed_files`, fills `hunks` with the unit's own files' diff (`git
diff -U3` per file) and `other_changed_files` with the rest.

| role | sections |
|---|---|
| maintainer | purpose, interface, anchors (+source), gaps, flows, hunks + other_changed_files with `--diff` |
| tester | interface (+source at interface anchors), scenarios, test_command, primary_paths, hunks with `--diff` |
| documenter | purpose, interface, doc prose, flows |
| validator | purpose, test_command, commands.test, repro_result |
| architect | every unit's purpose and interface ids, all flows, all decisions; no anchors, no source |

Source windows are 20 lines before and after the match (the definition span
with tree-sitter). Budget is bytes of the `--human` rendering. Shrink order:
halve every source window once; drop windows last-listed first; drop gaps;
drop flows; truncate hunks to their first 40 lines each. Purpose, interface,
scenarios, and `other_changed_files` are never dropped. If still over,
`over_budget = true` with the size and no further truncation. Every removal
is listed in `dropped`. Files matching `[pack] exclude` are named, never
inlined.

### Route

Repo-relative forward-slash paths matched with `globset`
(`literal_separator(true)`). Specificity: longest literal prefix, then fewest
`*`/`**`; remaining tie → first unit in `units.toml`, and `check` warns on
two units matching the same tracked file at equal specificity.

### next

Changed files = `git diff --name-only <base>` ∪ `git status --porcelain`
paths. `--base <ref>`; default is the merge-base with `origin/HEAD`, else
`main`, else `master`; when HEAD is that branch, only uncommitted and staged
changes count. Jobs:

- every touched unit → maintainer review
- a changed file is an interface anchor's path, or is owned by the unit and
  under `test_roots` → tester
- a changed file is an interface anchor's path → documenter
- 2+ units touched with no open plan → architect
- changed files with no owner → `unrouted`
- added lines in `git diff -U0` (and untracked files) that match the debt
  rule → listed under the owning unit's maintainer job
- `check` failures are included; `next` exits 1 when `check` fails

### Debt

Scan `git ls-files` (skipping `.stewards/` and files with a NUL in the first
8 KB) for lines where a comment marker (`#`, `//`, `--`, `;`, `/*`, `*`,
`<!--`) precedes `simplified:` (case-insensitive) on the same line; attach by
route; group per unit.

### init unit proposal (step 1, no LLM)

1. Roots: every directory holding a manifest (`Cargo.toml`, `package.json`,
   `pyproject.toml`, `setup.py`, `go.mod`, `*.csproj`, `*.sln`, `main.tf`,
   `Gemfile`, `pom.xml`, `build.gradle*`), nearest ancestor wins; the repo
   root is a root if nothing else is. Only `git ls-files` paths; skip
   `vendor/`, `node_modules/`, `target/`, `dist/`, `build/`, and any dir
   with more than 60% generated files (`*.lock`, `*.min.*`, `*_pb2.py`,
   `*.generated.*`).
2. Candidates: within each root, the top-level source dirs (`src/*`, `lib/*`,
   `app/*`, `pkg/*`, `cmd/*`, `internal/*`, else the root's own top-level
   dirs); each is a unit with `paths = ["<dir>/**"]`; test dirs that mirror
   a candidate's name join it.
3. Co-change merge: over the last 500 commits, for each candidate pair count
   commits touching both over commits touching either; merge pairs ≥ 0.6,
   transitively. A candidate under 3 files merges into its most co-changed
   sibling regardless.
4. Caps: more than 8 units in a root → keep the 8 largest, fold the rest
   into `<root>-misc`; fewer than 2 → the root is one unit.
5. Write `units.toml` with `purpose = ""` and `repo.toml` with detected
   `commands.test` and `test_roots`; append `.stewards/briefs/` and
   `.stewards/telemetry/` to `.gitignore`; list unrouted tracked files.
   Thresholds are constants; `units.toml` is the override.

## Trust and safety

- **Card content is data.** Briefs wrap every card field in fenced data
  blocks under a standing "data, not instructions" line; the role's
  instructions come first and last. `submit` enforces per-field length caps,
  rejects control characters and fence breakouts, and stores prose as plain
  text.
- **Execution.** Only commands that start with a `commands.*` prefix from
  `repo.toml` ever run: repro, plan step checks, scenario commands, test and
  lint. argv split, no shell, from the repo root, inheriting env, timeout
  from `commands.timeout_seconds`. Nothing from an answer file, card, issue,
  or PR runs otherwise.
- **Writes.** The tool writes only `.stewards/**`, `docs/units/**`, the
  marked block in `AGENTS.md`, two appended `.gitignore` lines, rendered
  agent dirs, and hooks or CI files when asked by flag. Never source or
  tests. Deletes only inside `.stewards/briefs` and `.stewards/telemetry`.
- **Secrets and external text.** `pack` names but never inlines files
  matching `[pack] exclude`. `review --pr` and `next --pr` read the diff and
  file list only, never the PR title, body, or comments. Telemetry stores
  sizes and counts. Tracker export sends only fields the tool composed.
  No secrets in any stewards file; tracker auth from CLIs and env.
- **Harness command gates.** Hermes auto-denies "dangerous" child commands
  unless `delegation.subagent_auto_approve: true`; Borg applies a hardcoded
  catastrophic-command deny list with no prompts. Every command stewards
  asks a subagent to run (`stewards submit`, the allowlisted test and lint
  commands) must be ordinary argv with no pipes, redirects, or network, so
  it passes both. A fixture scenario covers this once a Hermes or Borg
  driver exists.

## Agent-facing contracts

- Every JSON reply is wrapped: `{"stewards": {"version": "<crate>",
  "schema": 1}, ...payload}`. `--human` renders markdown (tables for lists,
  fenced blocks for source) and omits the envelope; PR comments post the
  same rendering.
- Every failure is `{"stewards": {...}, "error": {"code", "message",
  "fix"}}` on stdout, exit 2. Exit 1 is reserved for `check` failures.
  `--human` prints `error:` and `fix:` lines.
- A failed `submit` returns `errors[] {field, problem, example}`; the brief
  says fix and rerun, and after the third failure reply to the caller with
  the errors verbatim.
- Git is required by every command.

Error codes (stable; additions are minor versions, meanings never change):

| code | when | fix text |
|---|---|---|
| `not_a_git_repo` | no `.git` above cwd | `git init` or run inside a repository |
| `no_stewards_dir` | no `.stewards/` above cwd | `stewards init` |
| `schema_too_new` | `repo.toml` schema > tool's | upgrade stewards (`cargo install stewards`) |
| `invalid_toml` | parse failure, with file and line | fix the file at `<path>:<line>` |
| `unknown_unit` | unit name not in `units.toml` | one of the listed units |
| `unknown_role` | role not in the five | one of maintainer, tester, documenter, validator, architect |
| `unit_unreviewed` | `stamp` needed before `pack --diff` or `brief` | `stewards stamp <unit>` |
| `answer_parse_failed` | `submit` can't parse the file | the field errors listed; rerun submit |
| `answer_dead_anchor` | a patch names an anchor that doesn't resolve | correct the anchor or drop it |
| `answer_too_long` | field over cap | shorten `<field>` to ≤ N chars |
| `command_not_allowed` | command outside the allowlist | prefix with one of `commands.test` or add it to `repo.toml` |
| `command_timeout` | allowlisted command exceeded its timeout | raise `timeout_seconds` or fix the test |
| `workspace_repo_missing` | `stewards.toml` lists a path that doesn't exist | fix `repos` in `<path>` |
| `baml_runtime_unavailable` | the native lib can't load | set `BAML_LIBRARY_PATH` or allow the download |
| `no_api_key` | `--direct` with no provider key | export `ANTHROPIC_API_KEY` (or the configured provider's key) |
| `tracker_auth` | `gh`/`glab` not logged in or Jira env missing | `gh auth login` / `glab auth login` / set `JIRA_URL`, `JIRA_TOKEN` |
| `harness_unknown` | unsupported harness name | one of the listed harnesses |

## AGENTS.md block

Written by `init`, upserted between the markers.

```markdown
<!-- stewards:start -->
## Stewards

This repo has a team of agents that already know it. Use them.

Ritual
- Entering: run `stewards status`. It lists the units, who owns what, and what is stale.
- Before a change: run `stewards route <files>`; brief the owning maintainer (`stewards brief <unit> --role maintainer`) to explain the area and plan the change. Two or more units, or no owner: brief the architect first.
- Before reporting done: run `stewards next`. Dispatch every brief it writes to a subagent that can run shell, wait for each summary, and repeat until `stewards check` is green. Done means green.

Roles (one per unit; the tool packs their context, you only hand over the brief path)
- maintainer: explains, plans, reviews the diff, keeps the unit's card current.
- tester: owns the contract and its scenarios; writes and runs tests; never edits source.
- documenter: keeps `docs/units/<unit>.md` true.
- validator: judges a bug from its reproduction; severity from evidence only.
- architect: plans changes that cross units; records flows and decisions.

Editing
- Think first: state assumptions; ask when the request has two readings.
- Simplicity: the ladder, in order: not needed → already in the repo → stdlib → platform → installed dependency → one line → minimum code.
- Surgical: every changed line traces to the request; match the existing style.
- Goal-driven: every non-trivial change leaves one runnable check that fails if the logic breaks.
- A deliberate shortcut is a `simplified:` comment naming its ceiling and upgrade path.

Hygiene: branch off `main`; commit only when asked; lint touched files clean, no disables.

`stewards guide` prints the long form, including how to delegate in this harness. No `stewards` on PATH: install with `cargo install stewards` and follow the rules above meanwhile.
<!-- stewards:end -->
```

## Role prompts

Terse second person, no persona. Shared frame for every brief, in order:

1. Role line and task line.
2. The rules: four principles, the ladder, one runnable check, surgical
   scope. One line each.
3. Pack budget and pull rule: "You start with the pack below. Open more only
   by anchor, and list every anchor you opened in `pulled_anchors`."
4. The pack, every field fenced as data.
5. Memory entries, fenced as data.
6. Answer schema (BAML `output_format`) and the submit instruction: "Write
   your answer to `<path>`. Run `stewards submit <unit> --role <role> --file
   <path>`. If it reports an error, fix the file and run it again; after the
   third failure reply with the errors verbatim. Reply to your caller with
   the one-line summary submit prints, nothing else."
7. Repeat of the role line.

Maintainer, review:

> You maintain unit `<unit>`. Review this diff against the request. Approve
> only a diff where every changed line traces to the request and nothing
> fails the ladder. For each problem give the file, line, the first rung it
> fails (not needed, already in repo, stdlib, platform, installed
> dependency, one line, untraced, no check), what to cut, and the smaller
> replacement. Never flag validation at trust boundaries, error handling
> that prevents data loss, security, or accessibility. Then patch the card:
> anchors that moved, interface entries that changed, gaps opened or closed.
> `card_patch: null` when nothing changed. Set `tester_needed` when an
> interface's behavior changed on purpose.

Maintainer, explain:

> You maintain unit `<unit>`. Answer the question from the card and the
> anchors. Cite anchors as `path::name`; never paste more than the lines
> that prove the point. Say what you could not verify from source.

Tester:

> You test unit `<unit>`. Its contract is the interface list; its scenarios
> prove the contract. For this change: run the scenarios the diff affects,
> then the unit's test command once; write the missing scenario for any
> interface entry the diff touched; record results. A failing scenario is a
> bug: give a title, the exact repro command (it must start with one of the
> repo's test commands), and the evidence. You may write tests under the
> repo's test roots. You never edit source.

Documenter:

> You document unit `<unit>` for a human reader. Write three sections:
> overview (what it is for, in the reader's terms), how it works (the path
> through the anchors), how to change it (where to start, what to keep
> true). Plain prose, no headings inside sections, no code longer than three
> lines. Every claim must be grounded in an anchor or the card.

Validator:

> You judge one bug report for unit `<unit>`. The repro result is given; you
> do not run anything. Decide severity: critical = a primary path cannot
> complete or data is lost; medium = wrong or missing output with no visible
> error; low = visible, recoverable, non-blocking. With no repro result,
> decide `real` from the evidence alone and say what evidence is missing.

Architect:

> You see every unit's purpose and interface, the flows, and the decisions.
> For a plan: order the steps by unit, name the interface entries each
> touches, and give each step the one check that proves it. Prefer fewer
> steps; a step that changes two units is two steps. Record new flows and
> decisions the change creates. Propose a unit split or merge only when the
> change makes a boundary wrong. For an explanation: answer from the
> summaries; name the unit to ask for detail.

Answer shapes: maintainer `verdict approve|changes`, `findings[] {file,
line, rung, cut, replace_with}`, `card_patch {interface[], anchors[],
gaps[]} | null`, `tester_needed`; tester `scenario_patch[]`, `results[]
{scenario, pass, output}`, `bugs[] {title, repro, evidence, interface?}`,
`tests_written[]`; documenter `doc {overview, how_it_works, how_to_change}`;
validator `real, severity, reason, evidence[]`; architect `plan | null,
flows[], decisions[], unit_proposals[], explanation`. All carry
`pulled_anchors[]`, `notes`, `tokens_used?`.

`guide` prints the long form: the ritual, the roles, the editing rules, the
hygiene rules, and the delegation paragraph for the harness detected from
env (`CLAUDE_CODE`, `CODEX_*`, `PI_*`, `HERMES_*`, Borg: env var
unverified, else generic); `--harness <name>` forces one, `--all` prints
every variant. Delegation paragraphs for the brief-only harnesses:

- Hermes: `delegate_task(tasks=[{goal: "Read <absolute brief path> and
  follow it", context: "<repo path>"}])`. Children run in the background by
  default; wait for the completion message before running `stewards next`
  again. `role` is ignored by the source; the brief carries the role.
- ZeroClaw: `spawn_subagent{prompt: "Read <absolute brief path> and follow
  it"}` (same agent, fresh session, blocks until done; the risk profile must
  not exclude `spawn_subagent`), or `delegate{agent, prompt, context}` to a
  same-profile agent in bounded mode.
- Borg: `spawn_agent(role="coder", blocking=true, message="Read <absolute
  brief path> and follow it", context="<task>")`; or, headless, a fresh
  `borg ask` session per brief. Run inside the repo workspace (`-w` or
  `borg acp`), because `borg ask` defaults to the daemon's cwd.

## Docs templates

`docs/units/<unit>.md`: a banner ("generated by stewards from
`.stewards/units/<unit>.toml`; change prose through the documenter"), H1
unit name with purpose, Overview, How it works, How to change it, Interface
table (id, contract, anchor as a relative link with line), Scenarios table,
Flows this unit is part of, Gaps, last reviewed commit. `docs/units/README.md`:
one line per unit with a link, flows as ordered unit chains, decisions list.

## Evals

- **Fixture, zero external deps:** `evals/fixture/` with `api/` (Python
  in-memory items store, `unittest`), `client/` (TypeScript, `tsc` +
  `node:test`), `cli/` (Rust binary that shells out to the api); one flow
  `create-item` crossing all three; about 400 lines.
- **Seeded bugs, five:** critical, `cli` panics on empty input; medium, `api`
  pagination skips the last item; low, `client` logs a misleading retry
  message; cross-unit, `api` renamed a response field and `client` still
  reads the old name; flaky, a `client` test that depends on wall clock.
  Each has a hidden reference repro.
- **Tasks, six:** (1) add a `count` endpoint in `api`; (2) fix pagination
  from a bug report; (3) a refactor with a tempting over-build; (4) explain
  how create-item flows; (5) add a `priority` field end to end (expects an
  architect plan); (6) a second-session question about `priority` after
  task 5.
- **Arms, three:** `bare` (no `.stewards/`, no block), `block-only` (block
  present, binary off PATH; ritual and persistence graders `n/a`), `full`.
- **Runner:** `stewards eval` drives `claude -p --output-format stream-json
  --verbose`, `codex exec --json`, `pi --mode json`; parses each transcript
  to `Run {main_tokens {input, output, cache_read}, total_tokens,
  tool_calls[], files_opened[], shell_commands[], ritual_steps{status,
  route, brief, next, submit, check_green}, cost_usd?, duration_s}`.
  Main tokens = the parent session only; total = parent + all children.
  Ritual steps are shell commands whose argv starts with `stewards <cmd>`;
  `brief` when a subagent is dispatched with a `.stewards/briefs/` path or a
  `stewards submit` appears; `check_green` when `check` or `next` exits 0
  after the last edit. `files_opened` counts distinct repo-relative paths
  read in the main session.
- **Pass criteria:** 3 runs per arm per task, medians compared. Tokens:
  `full` main-context ≤ 80% of `bare` and total ≤ 110% of `bare`; task 6
  files opened ≤ half of `bare` task 4; both token criteria failing fails
  the task, one is a warning. Cleanliness: BAML judge, 3 votes, majority,
  counting rung failures, untraced lines, missing checks; pass when `full`
  has 0 untraced lines, ≤ 1 rung failure, ≥ 1 runnable check, and is ≤
  `bare` on every count. Docs accuracy: no claim in `docs/units/api.md`
  contradicted by code after task 1. Bug tasks: the tester's repro
  reproduces and the validator's severity equals the seeded one. Task 5: a
  plan exists and closes. Ritual: all six steps appear in `full`. The suite
  fails when any task fails; warnings are listed.
- **Cadence:** on demand and in CI on release tags; `--max-cost` ceiling.

## Release and versioning

Releases from tags via GitHub Actions; matrix `x86_64-unknown-linux-gnu`,
`aarch64-unknown-linux-gnu`, `aarch64-apple-darwin`, `x86_64-apple-darwin`;
each archive bundles the matching `libbaml_cffi` and the install script sets
`BAML_LIBRARY_PATH`; the workflow bumps the AUR PKGBUILD (builds from source,
pins the runtime lib as a source entry) and runs `cargo publish`.
Completions and the man page come from `clap_complete` and `clap_mangen` at
build time. Checklist in `RELEASING.md`.

## Build order

1. **Core, no LLM** (`feat/stage-1-core`): schemas, workspace resolution,
   `route`, `check`, `stamp`, `status`, `debt`, `pack`, `next` (with
   `brief_path: null`). Checkpoints: model + workspace + cli skeleton;
   git + check + stamp; route + debt + pack; next + workspace fixture + CI.
2. **Roles** (`feat/stage-2-roles`): `llm.rs`, BAML classes, `brief` and
   `submit` for the maintainer end to end, then the other roles; `init` two
   steps on stewards itself (first committed `.stewards/`); `sync`, `docs`,
   `guide`, memory, telemetry. Dogfood from here.
3. **Depth** (`feat/stage-3-depth`): architect and plans in `next`;
   `validate` and export (GitHub, then GitLab, Jira); tree-sitter static
   grammars, then runtime extras; `render` for six harnesses.
4. **Edges** (`feat/stage-4-edges`): fixture repo, mechanical graders, Claude
   Code driver; LLM graders, Codex and Pi drivers; git hooks, PR and MR
   comments, CI templates; plugin shell, completions, man page, release
   workflow, AUR.

One PR per stage, one commit per checkpoint, CI green at each commit, squash
merge. Rough single-builder effort: 4, 5–6, 6–8, 7–9 days. If scope slips,
cut in this order: Codex and Pi eval drivers → tracker export → extra render
targets.

Dogfood: stage 1 is verified against a hand-written, uncommitted
`.stewards/` for this repo; the committed one comes from `init` in stage 2,
tested against the known answer below. From stage 2 on, every change runs
stewards' own ritual with briefs dispatched to Claude Code subagents.
stewards itself is the real-repo eval target once the fixture eval passes.

## stewards' own units

| unit | paths | interface ids |
|---|---|---|
| `cli` | `src/main.rs`, `src/output.rs` | `json-envelope`, `errors` |
| `model` | `src/model.rs`, `src/workspace.rs` | `load-repo`, `anchor-parse`, `discover` |
| `git` | `src/git.rs` | `git-wrappers` |
| `analysis` | `src/anchors.rs`, `src/route.rs`, `src/check.rs`, `src/debt.rs` | `resolve`, `owner`, `check-report`, `stamp`, `debt-scan` |
| `pack` | `src/pack.rs` | `pack-shape`, `budget-shrink` |
| `next` | `src/next.rs`, `src/status.rs` | `job-rules`, `base-ref`, `status-report` |
| `llm` (stage 2) | `src/llm.rs`, `baml_src/**`, `src/baml_client/**` | `brief-render`, `submit-parse`, `direct-call` |
| `evals` (stage 4) | `evals/**`, `src/eval/**` | `run-shape`, `graders` |
| `plugin` (stage 4) | `.claude-plugin/**`, `hooks/**`, `skills/**`, `scripts/**` | `session-start` |

Flows: `next-to-submit` (`cli` → `next` → `analysis` → `pack` → `llm` →
`model`), `init` (`cli` → `model` → `git` → `llm` → `model`). Decisions:
facts in Rust, judgment in BAML; git through the CLI; text anchors first,
tree-sitter behind the same function.

## Module API (stage 1)

```rust
// model.rs
pub struct RepoConfig { schema: u32, name: String, test_roots: Vec<String>, docs_path: String,
    commands: Commands, budgets: Budgets, pack: PackConfig, flows: Vec<Flow>, decisions: Vec<Decision> }
pub struct UnitDef { name: String, purpose: String, paths: Vec<String> }
pub struct UnitCard { unit: String, reviewed: Option<Reviewed>, interface: Vec<Interface>,
    anchors: Vec<Anchor>, gaps: Vec<Gap>, doc: Option<Doc> }
pub struct TestCard { unit: String, primary_paths: Vec<String>, test_command: Option<String>, scenarios: Vec<Scenario> }
pub struct Reviewed { commit: String, fingerprint: String, tree: String }
pub struct Anchor { path: String, name: String }
impl Anchor { pub fn parse(s: &str) -> Result<Anchor> }
pub struct Loaded { root: PathBuf, repo: RepoConfig, units: Vec<UnitDef>,
    cards: BTreeMap<String, UnitCard>, tests: BTreeMap<String, TestCard> }
pub fn load_repo(root: &Path) -> Result<Loaded>

// workspace.rs
pub struct Repo { name: String, root: PathBuf, loaded: Option<Loaded> }   // None = uninitialized
pub struct Workspace { repos: Vec<Repo>, is_workspace: bool }
pub fn discover(cwd: &Path) -> Result<Workspace>
impl Workspace { pub fn display_name(&self, repo: &Repo, unit: &str) -> String }

// git.rs
pub fn is_repo(root) -> bool
pub fn ls_files(root) -> Result<Vec<String>>
pub fn ls_files_stage(root, paths: &[String]) -> Result<Vec<(String, String)>>
pub fn hash_object(root, path) -> Result<String>
pub fn write_tree(root) -> Result<String>
pub fn status_paths(root) -> Result<Vec<String>>
pub fn diff_names(root, base: &str) -> Result<Vec<String>>
pub fn diff_file(root, base: &str, path: &str) -> Result<String>
pub fn diff_added_lines(root, base: &str) -> Result<Vec<(String, u32, String)>>
pub fn merge_base(root, a: &str, b: &str) -> Result<String>
pub fn head(root) -> Result<String>
pub fn default_branch(root) -> Result<String>

// anchors.rs
pub enum Resolution { Found { line: u32 }, DeadPath, DeadName }
pub fn resolve(root, a: &Anchor) -> Resolution
pub fn source_window(root, a: &Anchor, before: usize, after: usize) -> Option<String>

// route.rs
pub struct Router { /* compiled globsets in units.toml order */ }
impl Router { pub fn new(units: &[UnitDef]) -> Result<Router>;
    pub fn owner(&self, path: &str) -> Option<&str>;
    pub fn overlaps(&self, files: &[String]) -> Vec<(String, Vec<String>)> }

// check.rs
pub fn fingerprint(root, unit: &UnitDef, router: &Router) -> Result<String>
pub fn stamp(root, unit: &str) -> Result<Reviewed>        // shared with submit
pub struct CheckReport { failures: Vec<Finding>, warnings: Vec<Finding> }
pub fn check(ws: &Workspace, filter: Option<&str>) -> Result<CheckReport>

// pack.rs
pub enum Role { Maintainer, Tester, Documenter, Validator, Architect }
pub struct Pack { /* fields as in Rules */ }
pub fn pack(repo: &Repo, unit: &str, role: Role, task: &str, changed: Option<&[String]>) -> Result<Pack>
impl Pack { pub fn render(&self) -> String; pub fn fit(&mut self, budget: usize) }

// next.rs
pub struct Job { unit: String, role: Role, task: String, reasons: Vec<String>,
    brief_path: Option<PathBuf>, command: String, debt_added: Vec<DebtItem> }
pub struct NextReport { base: String, changed: Vec<String>, jobs: Vec<Job>,
    unrouted: Vec<String>, architect: bool, check: CheckReport }
pub fn next(ws: &Workspace, base: Option<&str>, task: Option<&str>) -> Result<NextReport>

// debt.rs
pub struct DebtItem { path: String, line: u32, text: String }
pub fn scan(repo: &Repo) -> Result<BTreeMap<Option<String>, Vec<DebtItem>>>   // None = unrouted

// status.rs
pub fn status(ws: &Workspace) -> Result<StatusReport>

// output.rs
pub trait Human { fn render(&self) -> String }
pub fn emit<T: Serialize + Human>(v: &T, human: bool)
pub struct CliError { code: &'static str, message: String, fix: String }
```

Dependencies for stage 1: `clap` (derive), `serde`, `serde_json`, `toml`,
`globset`, `sha2`, `anyhow`; dev `tempfile`. No git library, no walker, no
regex. Stage 2 adds `baml` and `toml_edit` (writes keep comments and order).

Stage 1 tests run the binary via `env!("CARGO_BIN_EXE_stewards")` against
`tests/fixtures/single/` (two units, a `simplified:` comment in code and one
in markdown, an uncovered interface, a cross-unit flow) and
`tests/fixtures/workspace/` (two repos, one uninitialized), copied to a temp
dir, `git init`, committed, stamped, committed again. One behavior per test.

## Risk register

| # | Risk | Signal | Mitigation | Kill criterion |
|---|---|---|---|---|
| 1 | BAML Rust client immature | `cargo update` breaks the generated client; runtime download fails | Pin `baml` exactly; vendor the lib in releases; every LLM step behind `llm.rs` so templates + serde is a one-module swap | Two consecutive BAML releases break the client with no fix in 30 days → templates + serde |
| 2 | Harness drift across ten targets | Agent dirs, subagent tools, or env vars change | Per-harness facts in one `harness.rs` table with a last-verified date; evals on three; spot-check list for the rest per release | A harness drops repo AGENTS.md or shell → drop it |
| 3 | Subagents ignore the pack | `pulled_anchors` large; tokens per job far above pack size | Pull rule stated twice; budget shown; token grader | Two prompt-tuning rounds after the first eval; still missing the 20% main-context criterion → reopen delegation before further stage 3 work |
| 4 | Cards rot despite `check` | Many `card_patch: null` while anchors drift; prose contradicts code | Fingerprint staleness; docs-accuracy grader; `sync` | Docs-accuracy fails two releases running → regen-only cards |
| 5 | Prompt injection through cards or reports | A brief makes a subagent act outside its role | Data fencing, caps, allowlist, writes boundary; a red-team task in the fixture | Any confirmed escape → stop-ship until fixed |
| 6 | Scope for one builder | Stages slip; no dogfood by stage 2 | Each stage usable alone; cut order above | Stage 2 not dogfooding in its window → freeze at stages 1–2 |
| 7 | Eval cost or flakiness | Runs exceed estimate; graders disagree | Haiku graders; 3-vote majority; medians; `--max-cost` | Suite flakes on an unchanged tree twice → graders redesigned first |
| 8 | Codex driver untestable locally | `codex exec` 400s under the ChatGPT account | API-key auth for the runner, or Codex arm in CI only | No accepted model id in 30 days → manual spot-check |

## Cost estimate (list prices, 2026-10; assumptions stated)

Opus 5.5 $4 in / $20 out per MTok, cache reads $0.20; Sonnet 5.5 $2 / $10,
cache reads $0.20; Haiku 5.5 $0.10 / $0.50 (prompts ≤ 100K). Claude Code
and Codex under subscriptions cost no API dollars; Pi and `--direct` do.

- One full eval: 54 agent runs per harness at an assumed 150K input (≈60%
  cache reads) and 6K output each: ≈ $12 per harness on Sonnet, ≈ $25 on
  Opus. Graders ≈ 2M input tokens: $0.20 Haiku, $4 Sonnet, $8 Opus. Total
  ≈ $15–35 with Pi API-billed and Haiku graders; triple if all three
  harnesses are API-billed.
- Dogfood per working day: ≈ 750K tokens inside the Claude Code
  subscription; `--direct` on Sonnet ≈ $1.50/day, Opus ≈ $3.
- `init` on stewards itself: ≈ 0.5M tokens, ≈ $1 on Sonnet.
- The soft number is tokens per agent run; telemetry replaces it after the
  first eval.

## First-session walkthrough (expected, Claude Code, `full` arm, task 1)

```
> stewards status
{ units: [api, client, cli], stale: [], warnings: [] }
> stewards route api/items.py
{ unit: "api" }
> stewards brief api --role maintainer --task "add a count endpoint"
{ path: ".stewards/briefs/api-maintainer-1.md", bytes: 6120 }
[Agent] read .stewards/briefs/api-maintainer-1.md and do what it says
  └ subagent: reads brief, opens api/items.py::list_items, writes answer,
    runs `stewards submit api --role maintainer --file ...`
    → "api maintainer: approve; 0 findings; card unchanged; stamped 3f2a9c1"
[edits api/items.py, api/tests/test_items.py]
> stewards next
{ changed: [api/items.py, api/tests/test_items.py], jobs: [
    { unit: api, role: maintainer, brief_path: ".stewards/briefs/api-maintainer-2.md" },
    { unit: api, role: tester,     brief_path: ".stewards/briefs/api-tester-1.md" },
    { unit: api, role: documenter, brief_path: ".stewards/briefs/api-documenter-1.md" } ],
  architect: false, unrouted: [], check: { failures: [api stale], warnings: [] } }
[Agent ×3] each reads its brief, works, submits
  → maintainer: "approve; 1 finding (rung: one line, api/items.py:41); card_patch: +interface count-items"
  → tester: "scenario count-ok added, 4/4 pass; unit test_command pass"
  → documenter: "doc updated"
[applies the finding]
> stewards next
{ jobs: [ { unit: api, role: maintainer, ... } ], check: { failures: [api stale] } }   # delta pack
[Agent] maintainer: "approve; 0 findings; stamped 8b1e044"
> stewards check
{ failures: [], warnings: [] }   exit 0
```

The main agent never reads a brief or a card; every brief path is one line;
`next` runs twice because the finding changed the unit; done is a green
`check`.

## Facts

- 6 of 7 mainstream harnesses support file-defined subagents with fresh
  context and a model pin: Claude Code (`.claude/agents/*.md`), Codex
  (`.codex/agents/*.toml`), Copilot (`.github/agents/*.agent.md`), Cursor
  (`.cursor/agents/`, also reads `.claude/` and `.codex/`), Gemini CLI
  (`.gemini/agents/*.md`), OpenCode (`.opencode/agents/*.md`). Aider has
  none. Ad-hoc subagents from a free-form prompt: Claude Code, Codex,
  Copilot VS Code only. AGENTS.md says nothing about subagents.
- Pi (TypeScript): reads AGENTS.md/CLAUDE.md in cwd and parents, user-level
  in `~/.pi/agent/`; Agent Skills from `.agents/skills/`, `~/.agents/skills/`,
  `.pi/skills/`; no built-in subagents; session-start only via a TS
  extension; shell yes. pi-subagents (`nicobailon/pi-subagents`, `pi install
  npm:pi-subagents`): agents at `.pi/agents/**/*.md` and
  `~/.pi/agent/agents/**/*.md`; md + YAML frontmatter with `name`,
  `description`, `model`, `tools`, `excludeTools`, `inheritProjectContext`,
  `defaultContext` (`fresh`|`fork`), `memory`; clean system prompt by
  default; no ad-hoc launch without a definition.
- Hermes (Python; source read 2026-10-21, `tools/delegate_tool.py`):
  project context is the first match of `.hermes.md` > `AGENTS.override.md`
  > `AGENTS.md` > `CLAUDE.md`; skills from `~/.hermes/skills/`,
  `.agents/skills/` (after `hermes skills trust`); shell hook
  `hooks.on_session_start[].command` in `~/.hermes/config.yaml`.
  `delegate_task` schema: `tasks[] {goal, context, output_schema, images}`,
  `action spawn|list|steer|stop`, `subagent_id`, `message`; no `model` or
  `toolsets`; `role` is accepted but ignored (depth-derived leaf or
  orchestrator). The child is a fresh agent (`skip_context_files`,
  `skip_memory`) whose system prompt is "focused subagent" + `context` +
  workspace path + the repo's `AGENTS.md`/`CLAUDE.md`; it inherits the
  parent's toolsets (terminal, file, web by default) and never gains more;
  shell in the repo; text result, `output_schema` validated with one retry;
  background by default. Config `delegation:` keys: `max_concurrent_children`
  10, `max_spawn_depth` 1, `subagent_auto_approve` false (dangerous commands
  auto-denied), `model`, `provider`, `worktree_isolation`, `child_timeout_seconds`.
  No file-defined subagents; does not read `.claude/agents/`.
- ZeroClaw (Rust; source read 2026-10-21, `crates/zeroclaw-runtime/src/tools/`):
  `spawn_subagent{prompt}` only; depth cap 1; the child is the same agent
  alias (same identity files, risk profile, workspace), fresh session,
  blocks the parent, returns text. `delegate{action, agent, prompt, context,
  background, parallel, task_id(s), timeout_ms ≤ 120000}` to an
  `[agents.<alias>]` entry (`model_provider`, `risk_profile`,
  `runtime_profile`, `skill_bundles`, `workspace {path, access}`,
  `delegates`, `identity`); no inline system prompt, persona = the agent
  workspace's `AGENTS.md`/`SOUL.md`/`IDENTITY.md`. Gated by
  `[risk_profiles.<name>] delegation_policy.mode = "allow"` (default
  forbidden); `bounded` inherits the caller's tool ceiling and, for a
  same-profile target, its workspace; `independent` uses the target's own
  workspace, jailed under `<install>/agents/<alias>/workspace/` unless
  `workspace.path` points at the repo. Reads `AGENTS.md` only from the agent
  workspace. No per-role file; `zeroclaw agents export` bundles config.
- Borg (adelphidata/borg, Rust; source read 2026-10-21): a personal-assistant
  daemon (`borgd`, gRPC over `~/.borg/borgd.sock`) with a `borg` CLI/TUI,
  channels, scheduler, memory, WASM plugins, an MCP client, and a coding
  side (worktrees, `apply_patch`, goals). Headless: `borg ask "<msg>" --json`,
  `borg send`, `borg acp`. Reads `AGENTS.md` then `CLAUDE.md` per directory
  from the session workspace up to the git root (`crates/cortex/src/project_doc.rs:17`),
  one file per dir, 32 KiB total budget with truncation; injected as
  `<project_instructions trust="stored">`. No user-level `AGENTS.md`; persona
  `~/.borg/IDENTITY.md`. Subagents: `spawn_agent{message, context, role,
  role_kind, toolsets, model, thinking, fork_context (default false = fresh),
  blocking, timeout_secs, tasks[]}`; built-in roles `researcher`, `coder`
  (`run_shell`, `apply_patch`, `read_memory`), `writer`; custom roles via the
  `manage_roles` tool, stored in sqlite `agent_roles`, no file format; depth
  1, 5 children per agent, 3 concurrent. Skills: `SKILL.md` from
  `~/.borg/skills/`, `<workspace>/.borg/skills/`, `<workspace>/.agents/skills/`,
  bodies injected by relevance under 4000 tokens. Hooks: `~/.borg/hooks.json`,
  Claude Code-compatible schema with `SessionStart`. Shell: `run_shell` in
  the session workspace, hardcoded deny list, no prompts, OS sandbox only
  for ACP sessions. `borg ask` has no cwd flag: workspace is the daemon cwd
  unless `-w <worktree>` or `borg acp`. Settings are DB-only (`borg settings`).
  Does not read `.claude/agents/`.
- BAML Rust (0.221.0): cold debug build 10.8 s, binary 468 KB. The crate
  `dlopen`s `libbaml_cffi`, downloaded on first LLM use from
  `https://github.com/boundaryml/baml/releases/download/<version>/<prefix>baml_cffi-<target-triple>.<ext>`
  with a `.sha256` beside it, cached under `BAML_CACHE_DIR` (macOS
  `~/Library/Caches/baml/libs/<version>/`, Linux `~/.cache/baml/libs/`), 46
  MB for `aarch64-apple-darwin`; `BAML_LIBRARY_PATH` and
  `BAML_LIBRARY_DISABLE_DOWNLOAD` override. The generated client has
  `build_request()` and `parse()`. Pure-Rust commands never load it.
- tree-sitter crates (2026-10-20): `tree-sitter` 0.27.1 and grammars for
  rust, python, typescript, javascript, go, bash, c, cpp, c-sharp, hcl, yaml
  exist on crates.io; no `tree-sitter-baml`; `tree-sitter-loader` compiles
  grammars from source at runtime (needs a C compiler); prebuilt grammar
  libraries load with `libloading`.
- Drivers (verified 2026-10-20 on claude 2.1.305 and codex-cli 0.154.0; Pi
  from docs): Claude Code `claude -p "<prompt>" --output-format stream-json
  --verbose --model <m> --max-turns N --max-budget-usd X
  --no-session-persistence` (final `result` line carries `usage`,
  `modelUsage`, `total_cost_usd`, `subagent_stats`; `--bare` skips hooks,
  CLAUDE.md, plugins). Codex `codex exec --json --skip-git-repo-check -C
  <dir> -s workspace-write -m <model> "<prompt>"` (`turn.completed{usage}`
  summed across turns; rollout under `~/.codex/sessions/`); this machine's
  ChatGPT-account Codex rejects the configured model ids with 400. Pi `pi
  --mode json "<prompt>"` (`tool_execution_start/end`, `message_update.usage`;
  session files under `~/.pi/agent/sessions/`; package
  `@mariozechner/pi-coding-agent` 0.73.1 or `@earendil-works/pi-coding-agent`
  1.3.0, which is current unverified).
- `claude plugin eval` exists (cases under `evals/<case>/`, six fixed grader
  types, with/without ablation, `--json`) but has no custom-code graders and
  starts in an empty workspace; stewards uses its own runner.

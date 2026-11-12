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

- **Harness contract.** stewards depends on three things a harness must
  provide: it reads the repo's `AGENTS.md`; its agent can run shell in the
  repo cwd; it can start a fresh-context child that can also run shell.
  Three more are used when present: a file format for a reusable role (then
  `render` writes one), a per-call or per-file model pin (then roles run on
  the right tier), parallel children (then `next`'s jobs overlap). Any
  harness meeting the required three is supported; the per-harness facts
  (paths, spawn tool, env detection, last-verified date) live in one
  `harness.rs` table and in Facts, never in the rules. The harnesses used
  to validate the contract are listed in Facts.
- **Stack:** a Rust CLI that agents call through their shell tool. No MCP
  server. Rust owns facts (units, anchors, packs, staleness, debt,
  rendering). BAML defines each role's prompt and typed answer schema; the
  generated Rust types validate every job's output strictly with serde, the
  same way in every harness. The schema and role text shown to agents are
  rendered from BAML at development time and checked in, so no ordinary
  command loads BAML's native library.
- **LLM modes:** the harness's own agents think by default: the tool writes
  a brief, the agent answers, the tool validates. `--direct` (or
  `STEWARDS_DIRECT=1`) makes the tool call the model through BAML with a key
  from the usual env vars, for CI and batch. It is the only mode that needs
  BAML's native library. A missing model entry, key, or library is an
  error, never a silent fallback, and the library is never downloaded
  unless `STEWARDS_BAML_DOWNLOAD` is set. The tool never names a model
  itself.
- **Delivery:** the repo's `AGENTS.md` block is the one truth: the ritual,
  the roles, the editing rules, how to delegate. Every harness reads it with
  no install. User-level hooks and instructions are optional accelerators.
  A harness that reads instructions from its own workspace rather than the
  repo gets the same block there. `init` places the block at the top of
  `AGENTS.md`, because some harnesses truncate project docs or take only
  the first matching file.
- **Efficiency:** three numbers go down (main context per task, total tokens
  per task, repeated work across sessions), and each agent gets the exact
  context its job needs: no more, no less.
- **License:** MIT, no attribution section. Clean room: no text from the
  adelphi-adlc-plugin repo.

### Team

- **Roles, six:** four per unit and two that work across them (the
  validator takes the bug's unit; the architect takes none, so `pack`,
  `brief`, and `submit` omit the unit for it). Developer (implements one
  task of the maintainer's plan, writes only the files in its lane, never
  stamps), maintainer (explains, plans, splits a plan into developer tasks,
  reviews the diff for cleanliness and surgical scope, keeps the card
  current), tester (owns the contract and its scenarios, writes and runs
  tests, never edits source), documenter (writes the prose sections of the
  unit's card), validator (judges a bug from its reproduction, severity from
  evidence only), architect (plans changes that cross units, records flows
  and decisions, proposes unit boundaries).
- **Team member, physically:** the same in every harness: a fresh-context
  child that reads a brief by path and runs shell. Where the harness has a
  file format for a reusable role, `render` writes one file per role from
  one source (a **rendered** harness) and the delegation message carries
  only the brief path. Where it has none (a **briefed** harness), the brief
  also carries the role text and the harness's own spawn tool is told to
  read the brief path. The split is about file formats, not subagent
  capability: most briefed harnesses have full subagents.
- **Render target:** the harness's user-level agent dir with model pins
  from `~/.stewards/config.toml`; `--repo-level` writes the repo-level dir
  with no `model` field, for teams. A harness with no verified user-level
  dir is repo-level only.
- **Model per role:** `~/.stewards/config.toml` maps role → model alias per
  harness; `render` writes the pin; `--direct` builds a BAML `ClientRegistry`
  from the same table. Repo files never name a model.
- **Context:** pack by default, pull allowed. The tool computes the minimum
  pack for unit + role + task; the brief states a budget; the subagent may
  pull more by anchor and must report what it added in `pulled_anchors`.
- **Job loop:** `next` writes briefs to `.stewards/briefs/` and prints their
  paths; the main agent hands a path to a subagent that can run shell; the
  subagent writes its answer as JSON to the path the brief names and runs
  `stewards submit <unit> --role <r> --file <f>`, fixing and retrying up to
  three times on a validation error; the main agent receives only what
  submit prints: a one-line summary, plus the answer's `explanation` (capped
  at 2,000 characters) for explain and plan jobs. Every LLM step uses this
  loop, including `init --draft`.
- **Developers:** a maintainer has one developer by default and several
  when it splits its plan. Each task in the plan names its **lane**: the
  files it may write. `submit` accepts the plan only when every lane is
  owned by the unit and no two lanes share a file, so a unit's developers
  can run at once in one tree, as can developers of different units. The
  maintainer splits only work whose pieces build and check independently;
  anything else is one task. Reporting is by artifact, because most
  harnesses cap subagent depth at one: the maintainer's tasks become the
  developers' briefs, their combined diff becomes the maintainer's one
  review, and the main agent only dispatches. A developer is a fresh child
  per task, not a standing process; what makes it the unit's own is its
  pack. The main agent may make a small edit itself; the review is the same.
- **Lanes, checked:** a developer's answer does not list its files; at its
  `submit` the tool diffs the brief's snapshot against the tree and reports
  any changed file the unit owns that is in no current task's lane, as
  `out of lane` in the summary. It is a report, not a rejection: the write
  has happened, and the maintainer's review sees it as untraced.
- **Fix loop:** a review's findings are the next task. `brief <unit> --role
  developer` reads the maintainer's latest answer: an explain answer's
  `tasks`, or a review's `findings` as one task whose lane is the files the
  findings name. A developer that reports `blocked_on` sends the main agent
  back to the maintainer with that message as the task.
- **Tests, two writers in turn:** a developer writes the check for its task
  inside its lane; the tester writes scenario tests for the contract and
  may edit any test under the test roots. They never overlap: developers
  run in the changing step, the tester in the `next` round after it.
- **Job files:** one open job per (unit, role), or per task for developers.
  The brief is `.stewards/briefs/<unit>-<role>.md` (`architect.md` for the
  architect, `<unit>-developer-<n>.md` per task),
  the answer `<same>.answer.json`; a new job overwrites both, so nothing
  accumulates. The brief's first line is a tool-owned header recording the
  mode, the snapshot it was packed from, the pack's bytes and anchor count,
  and the bug title for a validator job; `submit` reads it.
- **Stamp:** only a maintainer review `submit` with `verdict: approve`
  stamps, and it stamps the brief's snapshot, not the tree at submit time,
  so anything written since (a parallel tester's tests, an applied finding)
  is still stale and gets its own review. `approve` means no findings;
  `submit` rejects an approval that carries any. Explain jobs, `changes`
  verdicts, and the other roles never stamp. `stamp <unit>` is the human
  override.
- **Role marks:** the tester's and documenter's submits each record the
  snapshot they finished at, beside the stamp. Their jobs are due by what
  changed since their own mark, not since the stamp, so a fix round that
  does not touch their paths does not run them again, and a tester's own
  new tests do not call the tester back. With no mark, the stamp is used.
- **Triggers:** before changing, `route` the files and brief the owning
  maintainer to explain and plan; two or more units, or no owner, brief the
  architect first. To change, `brief <unit> --role developer` writes one
  brief per task of the maintainer's plan (or the unit's step of an open
  architect plan) and the main agent dispatches them. Before reporting done, `next` finds the stale units and
  writes the jobs: maintainer review for every stale unit; tester when an
  interface anchor's file or a file under `test_roots` changed since the
  stamp; documenter when an interface anchor's file changed; architect when
  two or more units changed since their stamps and no plan covers them;
  validator for a bug with no severity. Done means `next` lists no jobs.
  `check` covers review only (stamps, anchors, scenario ids): it is what
  the hook and CI run.
- **Modes:** `next` writes review briefs; `brief` writes explain briefs,
  with `--task` as the question. The header line carries the mode.
- **Review diff:** the maintainer's pack carries the diff from the unit's
  stamped tree to the current snapshot, so a re-review after a finding sees
  only what changed since. A unit never stamped, or whose stamped tree git
  has pruned, gets the full pack and no diff: a first review.
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
- **Writing knowledge:** every delegated job ends in a typed `submit` the
  tool validates and writes; `next` puts every stale unit back in front of
  its maintainer. Agents never hand-edit cards.
- **Staleness:** `[reviewed]` records the stamped snapshot tree, the unit's
  fingerprint in it (sha256 over the blobs of the files it owns), and the
  commit for display. `check` fails when a unit's current fingerprint
  differs from its stamp or it has no stamp, an anchor no longer resolves,
  or a scenario references an unknown interface id. Uncovered interfaces,
  overlapping globs, and files no unit owns warn. Runs in the ritual and in
  CI.
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
  `next` runs each stale unit's step check through the allowlist, marks pass
  or fail, and closes the plan when all pass and every unit it names is
  fresh again; a closed plan is deleted (git history keeps it).
- **Debt:** `simplified:` comments are computed on demand by `debt` and
  attached to the owning unit by route; never stored.
- **Bugs:** a tester's `bugs[]` and `validate --report` both land on the
  owning card as `[[gap]] kind = "bug"` with the repro, keyed by title. The
  validator's submit sets `severity`, or removes a bug it judges not real;
  the maintainer closes a fixed one like any gap.
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
  runnable repro exists. `--export` creates an issue on GitHub, GitLab,
  Jira, or Linear (`repo.toml` `[tracker]`; auth from `gh`, `glab`,
  `JIRA_URL` and `JIRA_TOKEN`, `LINEAR_API_KEY`; Jira and Linear through
  their REST and GraphQL APIs), once per bug: the issue URL is recorded on
  the gap.
- **Git and PR:** `init --hooks` installs a pre-push hook running `check`;
  `next --pr <n>` posts the jobs as a PR or MR comment updated in place;
  `review --pr <n>` runs maintainer review briefs on the diff and posts
  findings with rungs; CI templates for GitHub Actions and GitLab CI. The
  remote host picks `gh` or `glab`.
- **Telemetry:** `.stewards/telemetry/<yyyy-mm>.jsonl`, gitignored, one line
  per submit: unit, role, task, pack bytes and anchor count, pulled anchors,
  harness, timestamp. Token counts come from eval transcripts, never from
  what a subagent says about itself.
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
  maintainer, tester, and documenter, and one developer per task.
- **task**: one piece of a maintainer's plan, done by one developer;
  **lane**: the files that task may write.
- **card**: a unit's knowledge file, `.stewards/units/<unit>.toml`; **test
  card**: `.stewards/units/<unit>.test.toml`.
- **anchor**: `path::name`, a place in source a card points at.
- **interface**: an entry in a card naming a contract the unit promises,
  with an id scenarios reference.
- **scenario**: a test-card entry proving one interface behavior.
- **gap**: a known shortfall, future item, or bug on a card; **debt**: a
  `simplified:` comment in code, computed, never stored.
- **flow**: an ordered chain of units a request crosses; **decision**: why a
  boundary or convention exists. Both live in `repo.toml`.
- **pack**: the exact context the tool computes for a unit, role, and task.
- **brief**: a file holding role instructions, the pack, memory, the answer
  schema, and submit instructions; handed to a subagent by path.
- **submit**: the command that validates a typed answer and writes it.
- **snapshot**: a git tree of the working tree as it is now: tracked,
  modified, and untracked-but-not-ignored files.
- **stamp**: recording that a unit was reviewed at a snapshot.
- **fingerprint**: sha256 over the blobs of the files a unit owns in a
  snapshot; **stale**: the current fingerprint differs from the stamped one.
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
developer = 24000
memory = 2000                            # user memory entries appended to every brief
[pack]
exclude = [".env*", "*.pem", "*.key", "*secret*", "*credential*", "id_rsa*"]
[[flow]]
name = "create-item"
units = ["api", "store"]
entry = "src/api/items.py::create_item"
check = "pytest tests/e2e -k create_item"   # optional; allowlisted; next runs it when a unit in the flow is stale
[[decision]]
title = "store has no HTTP knowledge"
why = "..."
[tracker]                                # optional
kind = "github"                          # github | gitlab | jira | linear
project = "owner/repo"                   # gitlab: group/project; jira: project key; linear: team key
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
tree = "<snapshot tree sha>"             # the snapshot stamped; the review diff starts here
tester = "<snapshot tree sha>"           # role marks: where the tester and documenter last finished
documenter = "<snapshot tree sha>"
[[interface]]
id = "create-item"
anchor = "src/api/items.py::create_item"
contract = "POST /items; 201 with body; 422 on invalid"
[[anchor]]
path = "src/api/items.py"
name = "router"
[[gap]]
kind = "gap"                             # gap | future | bug
title = "..."
anchor = "src/api/items.py::create_item"  # optional
note = "..."
repro = "pytest tests/api -k count"       # bug only; must pass the allowlist
severity = "medium"                      # bug only; set by the validator
issue = "https://..."                    # bug only; set by --export
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

User-level files live in `$STEWARDS_HOME` when it is set, else `~/.stewards`:
`config.toml` (`[models.<harness>] <role> = "<alias>"`, and `[models.direct]
<role> = "<provider>/<model>"` for `--direct`, providers anthropic, openai,
google-ai), `memory.toml` (`[[entry]] text, languages?,
roles?`), `~/.stewards/grammars/<lang>/{lib.so|.dylib, tags.scm}`.

Anchor grammar: `path` is the text before the first `::`, `name` is the rest
(Rust names contain `::`). A text match is an occurrence of `name` whose
neighbouring chars are not `[A-Za-z0-9_]`; case-sensitive; hand-rolled char
check, no regex crate.

Snapshot: copy `.git/index` to a temporary file, then `git add -A` and `git
write-tree` with `GIT_INDEX_FILE` pointing at it. The tree covers modified
and untracked files, honors `.gitignore`, and leaves the real index, HEAD,
and refs untouched. One snapshot per command.

Fingerprint: sha256 over the sorted `path blob` lines of `git ls-tree -r
<snapshot>` for the files the unit owns by route.

Unit names match `[a-z0-9][a-z0-9_-]*` (they become file names); anything
else is `invalid_toml`. `.stewards/**` and `docs_path/**` are tool-owned:
never routed, never reported as unowned.

## Command surface

| Command | Does |
|---|---|
| `guide [--harness h] [--all] [--errors] [--glossary]` | prints the ritual, roles, editing rules, delegation text for the detected harness, error codes, glossary |
| `init [--draft] [--hooks] [--ci]` | step 1 proposes `units.toml` and `repo.toml` from facts and writes the AGENTS.md block; `--draft` briefs the naming and card drafting; `--hooks` installs the pre-push hook; `--ci` writes the CI template |
| `status [--telemetry]` | units, freshness, coverage, gaps, across the workspace; uninitialized repos |
| `route <path>...` | owning unit per path, `null` when none; no LLM: an unowned path goes to the architect |
| `pack [unit] --role <r> [--diff] [--task t]` | the context pack; no unit for the architect |
| `brief [unit] --role <r> [--task t] [--stdout]` | writes an explain brief, prints its path and size |
| `submit [unit] --role <r> --file <f>` | validates a typed answer strictly (the BAML-generated types through serde) against the job's brief header, writes the update; an approving maintainer review stamps |
| `stamp <unit>` | records `[reviewed]` at the current snapshot; the human override |
| `next [--task t] [--no-brief] [--no-run] [--pr n]` | finds the stale units, runs `check` and plan step checks, writes the review briefs, prints the jobs |
| `check` | failures: stale or never-stamped units, dead anchors, unknown interface ids; warnings: uncovered interfaces, overlapping globs, unowned files |
| `validate [--report <f>] [--export]` | `--report` records a bug on its unit's card, runs the repro through the allowlist, writes the validator brief; `--export` files an issue for every judged bug that has none |
| `review --pr <n>` | maintainer review briefs on a PR or MR diff; findings posted as review comments |
| `debt` | `simplified:` comments attached to units |
| `docs [unit]` | renders `docs/units/<unit>.md` and the index |
| `render --harness <h> [--repo-level]` | writes the six native role agents |
| `eval [--harness h] [--arm a] [--task n] [--max-cost usd]` | runs the eval suite |
| `completions <shell>` | shell completions |

Global flags: `--human`, `--repo <name>`, `-C <dir>`, `--direct`. JSON by
default, markdown with `--human`. Exit 0 ok, 1 work is due (`check`
failures; for `next`, also any job or failed flow check), 2 errors.

## Rules

### Pack

```
Pack { unit, role, task, budget, bytes, over_budget, purpose, interface[],
       anchors[{path, name, line, source}], scenarios[], test_command,
       primary_paths[], test_commands[], gaps[], flows[], decisions[],
       units[{name, purpose, interface_ids}], doc, changed_files[],
       hunks[{path, diff}], other_changed_files[], memory[], bug, dev_task,
       repro_result, dropped[] }
```

Absent sections are `null`. `--diff` sets `changed_files` the way `next`
does, restricts `anchors` to those whose path is in it, fills `hunks` with
the unit's own files' diff (`git diff -U3 <stamped tree> <snapshot>` per
file) and `other_changed_files` with what other stale units changed. With
no usable stamp `--diff` changes nothing: the full pack is the first review.

| role | sections |
|---|---|
| maintainer | purpose, interface, anchors (+source), gaps, flows, hunks + other_changed_files with `--diff` |
| tester | interface (+source at interface anchors), scenarios, test_command, primary_paths, hunks with `--diff` |
| documenter | purpose, interface, doc prose, flows |
| validator | purpose, interface, primary_paths, the bug, test_command, commands.test, repro_result |
| architect | every unit's purpose and interface ids, all flows, all decisions; no anchors, no source |
| developer | purpose, interface, its task (change, lane, check), anchors (+source) in its lane, gaps, flows |

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

One snapshot per repo per run, shared with the embedded `check`; each job
carries the snapshot of its repo. A unit is stale when its fingerprint in the snapshot
differs from `[reviewed].fingerprint` or it has no `[reviewed]`. Its changed
files are `git diff --name-only <stamped tree> <snapshot>` restricted to the
files it owns; with no stamp, or a stamped tree git has pruned, every owned
file counts as changed and the pack has no hunks. There is no base ref:
committed, staged, unstaged, and untracked changes are all just distance
from the stamp. Jobs:

- every stale unit → maintainer review
- a file changed since the tester's mark is an interface anchor's path, or
  is owned by the unit and under `test_roots` → tester
- a file changed since the documenter's mark is an interface anchor's path
  → documenter
- tester and documenter jobs do not need the unit to be stale; with no
  mark they measure from the stamp
- 2+ units changed since their stamps (never-stamped units do not count)
  and no plan naming all of them → architect
- a bug gap with no `severity` → validator, one per unit per round
- added lines in `git diff -U0 <stamped tree> <snapshot>` that match the debt
  rule → listed under the owning unit's maintainer job
- a flow with a `check` and a stale unit → the command runs through the
  allowlist (unless `--no-run`); a failure is reported against the flow
- `check` failures are included; `next` exits 0 only with no jobs, a green
  `check`, and passing flow checks; otherwise 1

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
- **Git and file reads.** Every rev or tree handed to git follows
  `--end-of-options`, and a path is passed as a literal pathspec, so nothing
  from a card or a file name can become an option or a pattern. An anchor
  path must be made of plain components and pass through no symlink, else
  it is dead: an anchor can never read outside the repo or reach an
  excluded file through a link. Card text printed on one line has its
  control characters replaced; purpose and prose in a rendered pack sit in
  fences longer than any backtick run they contain; rendered docs escape
  card text and refuse a `docs_path` at or outside the repo root.
- **Execution.** Only commands whose argv starts with the whole argv of a
  `commands.*` entry from `repo.toml` ever run (`pytest` does not allow
  `pytest-x`): repro, plan step checks, scenario commands, test and
  lint. argv split, no shell, from the repo root, inheriting env, timeout
  from `commands.timeout_seconds`. Nothing from an answer file, card, issue,
  or PR runs otherwise.
- **Writes.** The tool writes only `.stewards/**`, `docs/units/**`, the
  marked block in `AGENTS.md`, two appended `.gitignore` lines, rendered
  agent dirs, and hooks or CI files when asked by flag. Never source or
  tests. Deletes only inside `.stewards/briefs` and `.stewards/telemetry`,
  and closed plans. Snapshots add unreachable objects to `.git/objects`
  (git prunes them) and never touch the index, HEAD, or any ref.
- **Secrets and external text.** `pack` names but never inlines files
  matching `[pack] exclude`. `review --pr` and `next --pr` read the diff and
  file list only, never the PR title, body, or comments. Telemetry stores
  sizes and counts. Tracker export sends only fields the tool composed.
  No secrets in any stewards file; tracker auth from CLIs and env.
- **Harness command gates.** Some harnesses auto-deny child commands they
  classify as dangerous, with no prompt. Every command stewards asks a
  subagent to run (`stewards submit`, the allowlisted test and lint
  commands) must be ordinary argv with no pipes, redirects, or network, so
  it passes such gates. A fixture scenario covers this once a driver for
  such a harness exists.

## Agent-facing contracts

- Every JSON reply is wrapped: `{"stewards": {"version": "<crate>",
  "schema": 1}, ...payload}`. `--human` renders markdown (tables for lists,
  fenced blocks for source) and omits the envelope; PR comments post the
  same rendering.
- Every failure is `{"stewards": {...}, "error": {"code", "message",
  "fix"}}` on stdout, exit 2. Exit 1 means work is due: `check` failures, or jobs from `next`.
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
| `unknown_role` | role not in the six | one of developer, maintainer, tester, documenter, validator, architect |
| `answer_parse_failed` | `submit` can't parse the file | the field errors listed; rerun submit |
| `answer_dead_anchor` | a patch names an anchor that doesn't resolve | correct the anchor or drop it |
| `answer_too_long` | field over cap | shorten `<field>` to ≤ N chars |
| `command_not_allowed` | command outside the allowlist | prefix with one of `commands.test` or add it to `repo.toml` |
| `command_timeout` | allowlisted command exceeded its timeout | raise `timeout_seconds` or fix the test |
| `workspace_repo_missing` | `stewards.toml` lists a path that doesn't exist | fix `repos` in `<path>` |
| `baml_runtime_unavailable` | `--direct` and the native lib is not found | set `BAML_LIBRARY_PATH` to the library, or set `STEWARDS_BAML_DOWNLOAD=1` to let stewards download it |
| `no_api_key` | `--direct` and the configured provider's key is unset or blank | export the variable named (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, or `GOOGLE_API_KEY`) |
| `tracker_auth` | `gh`/`glab` not logged in, or Jira or Linear env missing | `gh auth login` / `glab auth login` / set `JIRA_URL`, `JIRA_TOKEN` / set `LINEAR_API_KEY` |
| `harness_unknown` | unsupported harness name | one of the listed harnesses |
| `usage` | bad arguments, a bad `-C`, or a role given with or without a unit wrongly | run `stewards <command> --help` |
| `not_in_repo` | the current directory is in no repo of the workspace | run inside a repo or pass `--repo <name>` |
| `unknown_repo` | `--repo` names no repo | one of the listed repos |
| `path_outside_repo` | `route` given a path in no repo | pass a path inside the repo |
| `already_initialized` | `init` where `.stewards/` files exist | edit `.stewards/units.toml` instead |
| `no_brief` | no brief, or an unreadable header, for the job | run the brief command named |
| `no_direct_model` | `--direct` with no `[models.direct]` entry for the role, or an unsupported provider | set `[models.direct] <role> = "<provider>/<model>"` in the user config |
| `direct_call_failed` | the model call failed (fixed text, never the prompt or key) | rerun with `BAML_LOG=info` |
| `not_implemented` | a command or flag not built yet | use a release that has it |
| `internal` | any other failure | report a bug |

## AGENTS.md block

Written by `init`, upserted between the markers.

```markdown
<!-- stewards:start -->
## Stewards

This repo has a team of agents that already know it. Use them.

Ritual
- Entering: run `stewards status`. It lists the units, who owns what, and what is stale.
- Before a change: run `stewards route <files>`; brief the owning maintainer (`stewards brief <unit> --role maintainer --task "<the change>"`) to explain the area and plan the change. Two or more units, or no owner: brief the architect first (`stewards brief --role architect --task "<the change>"`).
- Changing: run `stewards brief <unit> --role developer` and dispatch every brief it writes, together when there are several. A small edit you may make yourself.
- Before reporting done: run `stewards next`. Dispatch every brief it writes to a subagent that can run shell, wait for each summary, and repeat until `stewards next` lists no jobs. A review that returns findings: run `stewards brief <unit> --role developer` again to fix them. Done means no jobs.

Roles (one per unit; the tool packs their context, you only hand over the brief path)
- maintainer: explains, plans, reviews the diff, keeps the unit's card current.
- tester: owns the contract and its scenarios; writes and runs tests; never edits source.
- documenter: keeps `docs/units/<unit>.md` true.
- validator: judges a bug from its reproduction; severity from evidence only.
- architect: plans changes that cross units; records flows and decisions.
- developer: implements one task of the maintainer's plan inside its lane.

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

1. The header line (tool-owned, see Job files), then the role line and task
   line.
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
   exactly what submit prints, nothing else."
7. Repeat of the role line.

Maintainer, review:

> You maintain unit `<unit>`. Review this diff against the request. Approve
> only a diff where every changed line traces to the request and nothing
> fails the ladder; an approval has no findings. For each problem give the file, line, the first rung it
> fails (not needed, already in repo, stdlib, platform, installed
> dependency, one line, untraced, no check), what to cut, and the smaller
> replacement. Never flag validation at trust boundaries, error handling
> that prevents data loss, security, or accessibility. Then patch the card:
> anchors that moved, interface entries that changed, gaps opened or closed.
> `card_patch: null` when nothing changed. Set `tester_needed` when an
> interface's behavior changed on purpose. With no diff in the pack this is
> a first review: draft the card from source and approve.

Maintainer, explain:

> You maintain unit `<unit>`. Answer the question from the card and the
> anchors. Cite anchors as `path::name`; never paste more than the lines
> that prove the point. Say what you could not verify from source. Put the
> answer in `explanation`; it is all your caller will see. When the task is
> a change, also give `tasks`: one unless the work splits into pieces that
> touch different files and can each be built and checked alone; each names
> the change, the files it may write, and the one check that proves it.

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

Developer:

> You implement one task in unit `<unit>`. The task, its lane, and its
> check are below. Write only files in your lane; if the task cannot be
> done inside it, stop and say which file you need and why. Follow the
> rules above: the smallest change that does the task, in the style of the
> code around it. Run the check and report its result as it is. Other
> developers may be working in this unit now: never revert or reformat a
> file outside your lane, and if the build breaks on a file that is not
> yours, run your check once more before reporting it.

Architect:

> You see every unit's purpose and interface, the flows, and the decisions.
> For a plan: order the steps by unit, name the interface entries each
> touches, and give each step the one check that proves it. Prefer fewer
> steps; a step that changes two units is two steps. Record new flows and
> decisions the change creates. Propose a unit split or merge only when the
> change makes a boundary wrong. For an explanation: answer from the
> summaries; name the unit to ask for detail.

Answer shapes: maintainer review `verdict approve|changes`, `findings[]
{file, line, rung, cut, replace_with}`, `card_patch {interface[], anchors[],
gaps[]} | null`, `tester_needed` (true makes `submit` write the tester brief
and print its path); maintainer explain `explanation`, `tasks[] {title, change, files[], check?}`
(empty for a pure question), `card_patch | null`; developer
`check {command, pass, output} | null`, `summary`, `blocked_on | null`; tester `scenario_patch[]`, `results[]
{scenario, pass, output}`, `bugs[] {title, repro, evidence, interface?}`,
`tests_written[]`; documenter `doc {overview, how_it_works, how_to_change}`;
validator `real, severity, reason, evidence[]`; architect `plan | null,
flows[], decisions[], unit_proposals[], explanation`. All carry
`pulled_anchors[]` and `notes`.

Details the shapes leave open: enum values are lowercase snake case
(`approve|changes`, `critical|medium|low`, `gap|future|bug`, the rungs as
`not_needed` … `no_check`). A nullable field may be absent or null; every
other field and every list is required; unknown fields, duplicate fields,
and any text around the JSON are errors. `card_patch` has three optional
lists (`interface`, `anchors`, `gaps`); a list that is present is the full
new list for that part of the card, which is how a patch closes a gap. A
patch gap is `{kind, title, anchor?, note, repro?}`; severity and issue are
the tool's. `scenario_patch` upserts scenarios by id. A plan is `{title,
steps[] {unit, change, interfaces[], check}}`. A unit proposal is `{name,
purpose, paths[]}`. A validation error names the field path, or
`(document)`, and carries an example value.

`guide` prints the long form: the ritual, the roles, the editing rules, the
hygiene rules, and the delegation paragraph for the harness detected from
env; `--harness <name>` forces one, `--all` prints every variant. The
detection rule and the paragraph text per harness come from the
`harness.rs` table (the current entries are in Facts); the generic paragraph
says: start a fresh-context subagent that can run shell, give it the
absolute brief path and nothing else, wait for its one-line summary.

## Docs templates

`docs/units/<unit>.md`: a banner ("generated by stewards from
`.stewards/units/<unit>.toml`; change prose through the documenter"), H1
unit name with purpose, Overview, How it works, How to change it, Interface
table (id, contract, anchor as a relative link with line), Scenarios table,
Flows this unit is part of, Gaps, last reviewed commit. `docs/units/README.md`:
one line per unit with a link, flows as ordered unit chains, decisions list.

## Evals

- **Fixture, zero external deps:** `evals/fixture/` with `api/` (Python
  in-memory items store, `unittest`), `client/` (TypeScript run by Node
  directly with type stripping, Node 22.18 or newer, no `tsc`;
  `node:test`), `cli/` (Rust binary; `create` goes through the client, the
  rest shells out to the api); one flow `create-item` crossing cli, client,
  and api; about 400 lines. The fixture's own tests pass and never expose a
  seeded bug; only the hidden repros under `evals/repros/` do, each with a
  `fix.patch`.
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
  `stewards submit` appears; `check_green` when `next` exits 0
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
   steps on stewards itself (first committed `.stewards/`); `docs`,
   `guide`, memory, telemetry. Dogfood from here.
   **2b. Proof** (`feat/stage-2b-proof`): the eval fixture, the runner, the
   Pi driver, and the token and ritual graders, run on tasks 1, 3, 4, and 6.
   Stage 3 starts only when `full` main context is on track for the 80%
   criterion; otherwise risk 3's kill criterion applies here.
3. **Depth** (`feat/stage-3-depth`): architect and plans in `next`;
   `validate` and export (GitHub, then GitLab, Jira, Linear); tree-sitter static
   grammars, then runtime extras; `render` for six harnesses.
4. **Edges** (`feat/stage-4-edges`): the remaining graders, LLM judges,
   Claude Code and Codex drivers; git hooks, PR and MR
   comments, CI templates; plugin shell, completions, man page, release
   workflow, AUR.

One PR per stage, one commit per checkpoint, CI green at each commit, squash
merge. Rough single-builder effort: 4, 6–7, 3 (2b), 6–8, 5–6 days. The BAML
spike (risk 1) runs beside stage 1 in a scratch directory, before `llm.rs`
is designed around it. If scope slips,
cut in this order: Codex and Claude Code eval drivers → tracker export → extra render
targets.

Dogfood: stage 1 is verified against a hand-written, uncommitted
`.stewards/` for this repo; the committed one comes from `init` in stage 2,
tested against the known answer below. From stage 2 on, every change runs
stewards' own ritual with briefs dispatched to the harness's subagents (Pi
with `pi-subagents` on the Linux machine, Claude Code on the macOS one).
stewards itself is the real-repo eval target once the fixture eval passes.

## Build cycle

How the build itself is run: one orchestrating session and a roster of
subagents, tuned for wall-clock speed. Settled 2026-11-11.

### Roster

Built-in `pi-subagents` agents with the model set per call as
`provider/model`; no agent files in the repo.

| build role | agent | model | used for |
|---|---|---|---|
| orchestrator | the parent session | `anthropic/claude-fable-5-2` | briefs, applying patches, gates, checkpoint commits, triage |
| senior writer | `worker` | `anthropic/claude-opus-5-5` | contract-heavy or cross-cutting tasks |
| standard writer | `worker` | `anthropic/claude-sonnet-5-5` | one well-specified module |
| mechanical writer | `worker` | `deepseek/deepseek-v4-pro` | fixtures, templates, packaging; off the subscription's limits |
| scout | `scout` | `anthropic/claude-haiku-5-5` | what-exists summaries for plan passes; transcript recon |
| researcher | `researcher` | `anthropic/claude-sonnet-5-5` | current docs before code depends on them |
| fact auditor | `evidence-auditor` | `anthropic/claude-opus-5-5` | a researcher's claims, before they enter Facts |
| spec reviewer | `reviewer` | `anthropic/claude-opus-5-5` | merged diff against this document |
| simplicity reviewer | `reviewer` + `ponytail-review` skill | `anthropic/claude-sonnet-5-5` | over-building, untraced lines, missing checks |
| security reviewer | `reviewer` | `anthropic/claude-opus-5-5` | trust-boundary tasks only, before merge |
| second-family reviewer | `reviewer` | `deepseek/deepseek-v4-pro` | each stage's whole diff, once, at the gate |
| second-harness check | `claude-code` | `claude-sonnet-5-5`, pinned in Claude Code's own settings | from stage 2: the ritual run from another harness; not launched until the pin exists |
| oracle | `oracle` | `anthropic/claude-opus-5-5` | rare: a reported conflict with this document |

The parent session is the project's only `claude-fable-5-2` use. Every
launch names its model: a child with no model inherits the parent's, so an
unpinned launch is a mistake, and `delegate` (which always inherits) is not
used. One resolved model per launch. On a quota or availability failure the lane
stops and is reported; the orchestrator never swaps a model silently. A
mechanical writer that fails its gate twice hands its task to a standard
writer, and the user is told.

### Wave loop

1. Start from a clean checkout at the last checkpoint commit.
2. Launch the wave as one async workflow: each writer in a managed worktree
   (`worktree: true`), briefed from its todo task: goal, cwd and ref, owned
   files, spec sections, tests, validation, what to return, and to stop and
   ask the parent on any conflict with this document.
3. Each writer gates itself in its worktree: fmt, clippy, tests. Writers do
   not commit; the worktree hands back a patch.
4. A task flagged for security is reviewed on its patch before it merges.
5. The parent applies the patches in dependency order, runs the full suite,
   and commits a checkpoint (covered by the user's go-ahead for the stage).
6. Review fan-out on the wave's merged diff, in parallel, fresh context,
   read-only: spec and simplicity.
7. The parent classifies every finding (blocker, defer, stale, invalid, out
   of scope, speculative). Accepted fixes go to the original writer,
   resumed; two fix rounds at most, then the user decides. A resumed
   writer hands back no patch: the parent takes `git diff` from its kept
   worktree, which is the change since the first capture.
8. Commit, close the todos, next wave.

A stage gate adds: the second-family review of the whole stage diff, the
second-harness check from stage 2 on, and the gate task's own checks.

### Pace

- No cap on writers: a wave is as wide as the dependency graph allows.
- A task starts when the tasks it truly depends on are checkpointed, not
  when its stage opens. It lands on its own stage's branch, which is
  stacked on the previous stage's and rebased when that one moves
  (`git rebase --onto main <old tip>` after a squash merge). So the
  stage-2 tasks that need only the skeleton run during stage 1, and the
  eval fixture is written early.
- The one hold: nothing from stage 3 or 4 starts before the thin-eval gate
  says delegation pays.
- Reviewers, scouts, researchers, and the auditor run beside the writers.
- Research look-ahead is one stage: BAML during stage 1; Pi transcript
  recon during stage 2; tree-sitter, the HTTP client, and harness rows
  during 2b; driver verification during stage 3. Further ahead, facts go
  stale.
- Security-flagged tasks: maintainer submit and fencing, the allowlist
  runner, both tracker tasks, hooks and PR comments, the release pipeline.

### Rehearsal (2026-11-11, scratch repo, since deleted)

Proven: every roster agent and model resolves; each child ran on the model
it was given and none on the parent's; three worktree writers (Sonnet,
Opus, DeepSeek) returned patches without committing and the patches
applied in order with the suite green; a writer escalated a planted spec
conflict and followed the answer; reviewers on Opus, DeepSeek, and Sonnet
reached the same findings; the oracle ran on Opus with the parent's
history; a resumed writer kept its context; a worktree based on a stacked
branch worked, and `git rebase --onto` carried it across a squash merge.

What the loop takes from it:

- Every agent inherits the parent's model unless one is named. The pin rule
  is what keeps Fable to the parent.
- Passing a skill to a child only makes it available. The brief must tell
  the child to read it and follow it.
- Reviewers have no shell. Give them the file list and the spec text, not
  "the diff".
- A worktree whose `target/` is hard-linked from a built one rebuilds in
  0.6 s against 18.6 s cold. Not yet wired into the worktree setup hook.
- The BAML schemas task starts only on the spike's recorded go, not on the
  spike merely finishing.
- Claude Code here is 1.0.128: its model aliases point at retired models
  and fail, the full name `claude-sonnet-5-5` works, and `pi-subagents`
  cannot pass a model to an external CLI.
- The DeepSeek writer passed the same gate as the others on a small
  fixture-and-parser task. Three reviewers then found the same two loose
  edges in it; none found any in the Sonnet or Opus work.

### Stage 1, as it ran (2026-11-12)

- Five waves and their fix rounds built stage 1, with the stage-2 tasks
  that needed only the skeleton running beside it. Every module's review
  found something real: option injection through a rev, an anchor reading
  outside the repo, a pack leaking an excluded file three ways, `stamp`
  writing to the wrong nested repo, a docs path overwriting the README.
- A signature-only stub committed by the parent lets parallel writers code
  against a module another writer is filling in.
- A resumed writer works in its old worktree: good for a fix inside its own
  files, wrong for a fix that needs newer code, which goes to a fresh
  worker.
- `pi-subagents` reads its config at session start. Until a restart picks
  up the warm-target hook, each brief starts with `test -e target || cp -al
  <built target> target`; hard-linked targets were unchanged after
  concurrent builds.
- The parent must check the test suite's own exit code before a checkpoint
  commit, and must not apply a patch during an unfinished rebase.
- A spike is told to make no network request beyond fetching tools and
  docs.

### Waves

| stage | waves (‖ = together, → = then) |
|---|---|
| 0 | rehearsal in a scratch repo |
| 1 | skeleton ‖ BAML spike ‖ eval fixture → git ‖ workspace ‖ anchors ‖ route ‖ docs ‖ guide ‖ user config → check ‖ debt ‖ init step 1 ‖ BAML schemas → pack ‖ status → next → gate |
| 2 | plan → brief → maintainer submit → other submits → developer → init --draft → gate |
| 2b | runner and Pi driver → thin-eval gate |
| 3 | plan → allowlist runner and plans ‖ tree-sitter ‖ render → validate ‖ runtime grammars ‖ flow checks → other trackers → gate |
| 4 | plan → hooks and PR glue ‖ plugin shell ‖ judges ‖ drivers ‖ release → full eval and gate |

Stage 2's first five tasks and the fixture appear in stage 1's row because
that is when they run; they still land on their own stage's branch.

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
| `evals` (stage 2b) | `evals/**`, `src/eval/**` | `run-shape`, `graders` |
| `plugin` (stage 4) | `.claude-plugin/**`, `hooks/**`, `skills/**`, `scripts/**` | `session-start` |

Flows: `next-to-submit` (`cli` → `next` → `analysis` → `pack` → `llm` →
`model`), `init` (`cli` → `model` → `git` → `llm` → `model`). Decisions:
facts in Rust, judgment in BAML; git through the CLI; text anchors first,
tree-sitter behind the same function.

## Module API (as built, stage 1)

The code is the reference; this is the map. `src/lib.rs` declares every
module and `src/main.rs` is a thin binary over it: clap, then one `run` per
command returning `anyhow::Result<ExitCode>`. A `CliError` anywhere becomes
the JSON error; anything else is `internal`.

```rust
// model.rs: the schemas as serde types, plus
pub enum Role { Developer, Maintainer, Tester, Documenter, Validator, Architect }  // FromStr, as_str
impl Anchor { pub fn parse(s: &str) -> Result<Anchor> }
pub fn load_repo(root: &Path) -> Result<Loaded>     // checks .git and .stewards for the root it is given

// workspace.rs: a stewards.toml above cwd wins over a nearer .git
pub fn discover(cwd: &Path, only: Option<&str>) -> Result<Workspace>   // only = --repo
pub fn here(g: &Global) -> Result<(PathBuf, Workspace)>                 // canonical cwd + discover
impl Repo { pub fn loaded(&self) -> Result<&Loaded> }                    // the one no_stewards_dir
impl Workspace {
    pub fn repo_containing(&self, path: &Path) -> Option<&Repo>          // deepest root
    pub fn current_repo(&self, cwd: &Path) -> Result<&Repo>              // the one not_in_repo
    pub fn loaded_repos(&self) -> Result<Vec<(&Repo, &Loaded)>>          // skips uninitialized siblings
    pub fn display_name(&self, repo: &Repo, unit: &str) -> String
}

// git.rs: the only module that runs git
pub fn ls_files(root) -> Result<Vec<String>>
pub fn snapshot(root) -> Result<String>
pub fn ls_tree(root, tree) -> Result<Vec<(String, String)>>
pub fn has_object(root, sha) -> bool                 // resolves to a tree
pub fn diff_names(root, from, to) -> Result<Vec<String>>
pub fn diff_file(root, from, to, path) -> Result<String>
pub fn diff_added_lines(root, from, to) -> Result<Vec<(String, u32, String)>>
pub fn head_commit(root) -> Result<Option<String>>   // None = no commits yet
pub fn task_text(root) -> Result<String>

// anchors.rs
pub enum Resolution { Found { line: u32 }, DeadPath, DeadName }
pub fn resolve(root, a: &Anchor) -> Resolution
pub fn source_window(root, a: &Anchor, before: usize, after: usize) -> Option<String>

// route.rs
impl Router {
    pub fn new(units: &[UnitDef], docs_path: &str) -> Result<Router>
    pub fn owner(&self, path: &str) -> Option<&str>
    pub fn tool_owned(&self, path: &str) -> bool
    pub fn overlaps(&self, files: &[String]) -> Vec<(String, Vec<String>)>
}

// model.rs also: impl Loaded { pub fn unit(&self, name: &str) -> Result<&UnitDef> }   // the one unknown_unit

// check.rs
pub fn fingerprint(tree: &[(String, String)], unit: &str, router: &Router) -> String
pub fn is_stale(reviewed: Option<&Reviewed>, current: &str) -> bool
pub fn changed_since(..) / pub fn stale_files(..)    // a unit's changed files; all owned files with no usable tree
pub fn stamp(repo: &Repo, unit: &str, tree: &str) -> Result<Reviewed>
pub fn check(ws, filter) -> Result<CheckReport>      // takes the snapshots, then
pub fn check_at(ws, filter, snapshots: &[String]) -> Result<CheckReport>
pub fn one_line(s: &str) -> String

// pack.rs
pub fn pack(repo: &Repo, unit: Option<&str>, role: Role, task: &str, diff: bool) -> Result<Pack>
impl Pack { pub fn render(&self) -> String; pub fn fit(&mut self, budget: usize) }

// next.rs
pub struct Job { unit, role, task, snapshot, reasons, changed, brief_path, command, debt_added }
pub struct NextReport { jobs, architect, flow_checks, check }
pub fn next(ws: &Workspace, task: Option<&str>) -> Result<NextReport>

// debt.rs
pub fn is_debt_line(line: &str) -> bool
pub fn scan(repo: &Repo) -> Result<BTreeMap<Option<String>, Vec<DebtItem>>>

// status.rs
pub fn status(ws: &Workspace) -> Result<StatusReport>

// output.rs
pub struct Global { human, repo, direct }
pub trait Human { fn render(&self) -> String }
pub fn emit<T: Serialize + Human>(v: &T, human: bool)
pub struct CliError { code, message, fix }
```

In a workspace a job's `command` is `stewards --repo <repo> brief <unit>
--role <role>`: write commands act on one repo and look units up by their
plain name.

Dependencies for stage 1: `clap` (derive), `serde`, `serde_json`, `toml`,
`globset`, `sha2`, `anyhow`; dev `tempfile`. No git library, no walker, no
regex. Stage 2 adds `baml` and `baml-sys` (both pinned exactly; building
them needs `protoc`), `toml_edit`, and `serde_path_to_error`.

Tests run the binary via `env!("CARGO_BIN_EXE_stewards")`, and the library
directly where that is simpler, against `tests/fixtures/single/` (two units,
a `simplified:` comment in code and one in markdown, an uncovered interface,
a cross-unit flow, one unowned file) and `tests/fixtures/workspace/` (two
repos, one uninitialized), copied to a temp dir and `git init`ed by the
helpers in `tests/common`. One behavior per test.

## Risk register

| # | Risk | Signal | Mitigation | Kill criterion |
|---|---|---|---|---|
| 1 | BAML Rust client immature | `cargo update` breaks the generated client; runtime download fails | Pin `baml` and `baml-sys` exactly; validation is serde on the generated types, so only `--direct` touches the native lib; vendor the lib in releases; every LLM step behind `llm.rs` so templates + serde is a one-module swap | Two consecutive BAML releases break the client with no fix in 30 days → templates + serde |
| 2 | Harness drift across the supported harnesses | Agent dirs, subagent tools, or env vars change | Per-harness facts in one `harness.rs` table with a last-verified date; evals on three; spot-check list for the rest per release | A harness drops repo AGENTS.md or shell → drop it |
| 3 | Subagents ignore the pack | `pulled_anchors` large; tokens per job far above pack size | Pull rule stated twice; budget shown; token grader | Two prompt-tuning rounds after the first eval; still missing the 20% main-context criterion → reopen delegation before stage 3 starts |
| 4 | Cards rot despite `check` | Many `card_patch: null` while anchors drift; prose contradicts code | Fingerprint staleness; docs-accuracy grader; `next` re-reviews every stale unit | Docs-accuracy fails two releases running → regen-only cards |
| 5 | Prompt injection through cards or reports | A brief makes a subagent act outside its role | Data fencing, caps, allowlist, writes boundary; a hostile fixture card in the Rust tests (fence breakout, control characters, over-cap fields) | Any confirmed escape → stop-ship until fixed |
| 6 | Scope for one builder | Stages slip; no dogfood by stage 2 | Each stage usable alone; cut order above | Stage 2 not dogfooding in its window → freeze at stages 1–2 |
| 7 | Eval cost or flakiness | Runs exceed estimate; graders disagree | Haiku graders; 3-vote majority; medians; `--max-cost` | Suite flakes on an unchanged tree twice → graders redesigned first |
| 8 | Codex driver untestable locally | `codex exec` 400s under the ChatGPT account | API-key auth for the runner, or Codex arm in CI only | No accepted model id in 30 days → manual spot-check |

## Cost estimate (list prices, 2026-10; assumptions stated)

Opus 5.5 $4 in / $20 out per MTok, cache reads $0.20; Sonnet 5.5 $2 / $10,
cache reads $0.20; Haiku 5.5 $0.10 / $0.50 (prompts ≤ 100K). Claude Code,
Codex, and Pi under subscriptions cost no API dollars; `--direct` does.

- One full eval: 54 agent runs per harness at an assumed 150K input (≈60%
  cache reads) and 6K output each: ≈ $12 per harness on Sonnet, ≈ $25 on
  Opus. Graders ≈ 2M input tokens: $0.20 Haiku, $4 Sonnet, $8 Opus. With
  all three harnesses on subscriptions only the graders are metered; each
  API-billed harness adds its ≈ $12–25.
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
{ path: ".stewards/briefs/api-maintainer.md", bytes: 6120 }
[Agent] read .stewards/briefs/api-maintainer.md and do what it says
  └ subagent: reads brief, opens api/items.py::list_items, writes answer,
    runs `stewards submit api --role maintainer --file ...`
    → "api maintainer: explained; card unchanged" and the plan, ≤ 2,000 chars
> stewards brief api --role developer
{ jobs: [ { task: 1, brief_path: ".stewards/briefs/api-developer-1.md" } ] }
[Agent] developer: edits api/items.py, api/store.py, api/tests/test_items.py
    → "api developer 1: 3 files; check pass"
> stewards next
{ jobs: [
    { unit: api, role: maintainer, brief_path: ".stewards/briefs/api-maintainer.md" },
    { unit: api, role: tester,     brief_path: ".stewards/briefs/api-tester.md" },
    { unit: api, role: documenter, brief_path: ".stewards/briefs/api-documenter.md" } ],
  architect: false, check: { failures: [api stale], warnings: [] } }
[Agent ×3] each reads its brief, works, submits
  → maintainer: "changes; 1 finding (rung: one line, api/store.py:41); card_patch: +interface count-items"
  → tester: "scenario count-ok added, 4/4 pass; unit test_command pass"   # marks its snapshot
  → documenter: "doc updated"                                             # marks its snapshot
> stewards brief api --role developer                                     # the finding is the task; lane: api/store.py
[Agent] developer: "api developer 1: 1 file; check pass"
> stewards next
{ jobs: [ { unit: api, role: maintainer, ... } ], check: { failures: [api stale] } }   # api/store.py is on neither mark's paths: no tester, no documenter
[Agent] maintainer: "approve; stamped 8b1e044"
> stewards next
{ jobs: [], check: { failures: [], warnings: [] } }   exit 0
```

The main agent never reads a brief, a card, or source; every brief path is one line;
the finding goes back to a developer, not to the main agent; the second
round is the maintainer alone because the fix touched neither mark's
paths; done is a `next` with no jobs.

## Handoff (for a new session)

Read this section, then the Build order, then start.

**State (2026-11-12).** Stage 1 is built on `feat/stage-1-core`: schemas,
workspace, git snapshots, anchors, `route`, `check`, `stamp`, `debt`,
`status`, `pack`, `next`, each reviewed and fixed, 171 tests. Stacked on it,
`feat/stage-2-roles` holds the harness table and `guide`, user config and
memory, `docs`, `init` step 1, and the BAML schemas with `llm.rs`;
`feat/stage-2b-proof` holds the eval fixture. Nothing is merged to `main`
but this document. Repo `wromansky/stewards` (private), auto-delete of
merged branches on.

**Next action.** The stage-1 gate, then the stage-2 chain on
`feat/stage-2-roles`: `brief`, `submit` for the maintainer, `submit` for the
other roles, the developer role, `init --draft`, then dogfooding on this
repo. The task list in the build session is the plan; Build cycle says how
waves run.

**Deferred, known:** workspace loads every listed repo before applying
`--repo`, so a broken sibling blocks `--repo good`; the harness table has no
`parallel` field; an anchor's symlink check runs before the read, not with
it; rendered docs escape every punctuation character, which is safe but
ugly raw; `guide --errors` lists only the codes that existed when it was
written.

**How we work.**
- Clean room: no text from the adelphi-adlc-plugin repo, ever.
- Never work on `main` for code; branch, PR, squash merge. Docs-only edits
  to `DESIGN.md` may go to `main` directly when the user says so.
- The user runs `git push` and `gh pr merge` themselves (Claude's pushes
  and merges are denied by permission settings); Claude does everything
  else, including commits when asked.
- In plan mode, keep grilling until the user says done in their own words.
- Build protocol: parallel workers each get a managed git worktree
  (`pi-subagents` `worktree: true`, which needs a clean checkout). Workers
  do not commit; the parent applies each patch in dependency order, runs
  the full suite, and commits a checkpoint on the stage branch. The user's
  go-ahead for a stage covers those commits; the PR is squash merged. The
  roster, the wave loop, and the pace are in Build cycle.
- Each later stage gets its own plan pass against this document before
  code. Dogfood from stage 2: every change runs stewards' own ritual.

**Local environment facts.**
- Second machine (Artix Linux x86_64, checked 2026-11-11): cargo 1.99.1,
  git 2.56, pi 1.5.2 with `pi-subagents`, claude 1.0.128, `gh` logged in as
  wromansky; no `baml-cli`, `codex`, or `glab`. The facts below are from the
  macOS machine.
- Toolchain: cargo 1.96, `baml-cli` 0.221.0 (matches the `baml` crate to
  pin), `gh` logged in as wromansky, `glab` not checked.
- Pi 1.4.0 installed (`@earendil-works/pi-coding-agent`, at
  `/opt/homebrew/bin/pi`) with `pi-subagents` installed; Anthropic OAuth
  (subscription) credential in `~/.pi/agent/auth.json`. One-shot JSON mode
  verified: `pi --mode json --no-session "<prompt>"` returns usage and
  text on `anthropic/claude-opus-4-8`. The `pi-subagents` spawn test
  (`.pi/agents/echo.md` in `/private/tmp/pi-probe`) was started but
  stopped before completing: unverified. Note: on a fresh session
  `pi-subagents` exposes only a `subagents_enable` loader tool; set
  `toolActivation: "eager"` or pass `--exclude-tools subagents_enable` to
  have `subagent` active from the first request (eval driver detail).
- Codex CLI 0.154.0 under a ChatGPT-account login rejects `gpt-5.5` and
  the other pinned ids with 400 "not supported when using Codex with a
  ChatGPT account"; the account's offered model changed since June 2026.
  Fix: pick an offered model via interactive `/model`, or API-key auth for
  the eval runner. User said not to worry about it for now.

**Open questions:** none. **Known costs, accepted until telemetry says
otherwise:** a tester that writes tests makes its unit stale, so the maintainer
reviews those tests in one more round; lanes are reported, not enforced;
developers sharing a
compiled unit can see each other's half-written files in a build, which is
why the maintainer splits only independent work; one open job per (unit,
role), so a unit's bugs are judged one per round. Deferred ideas: a ZeroClaw renderer
(`[agents.<role>]` TOML), a documented one-time Borg `manage_roles` setup,
a Hermes eval driver if briefed-harness performance needs measuring.

## Facts

### Harness table (the contract, checked per harness; last verified 2026-10-21)

| harness | role-file format (rendered) | spawn tool / fresh child | model pin | parallel | env detection |
|---|---|---|---|---|---|
| Claude Code | `.claude/agents/*.md`, `~/.claude/agents/` | Agent tool | per file | yes (20) | `CLAUDE_CODE` |
| Cursor | reads `.claude/agents/`, `.codex/agents/`, `.cursor/agents/` | Task tool | per file | yes | UNVERIFIED |
| Codex CLI | `.codex/agents/*.toml`, `~/.codex/agents/` | spawn tool | per file | yes | `CODEX_*` |
| Copilot | `.github/agents/*.agent.md` (repo-level only) | `runSubagent` | per file | yes | UNVERIFIED |
| Gemini CLI | `.gemini/agents/*.md`, `~/.gemini/agents/` | `@name` | per file | yes | UNVERIFIED |
| OpenCode | `.opencode/agents/*.md`, `~/.config/opencode/agents/` | Task tool | per file | yes | UNVERIFIED |
| Pi + pi-subagents | `.pi/agents/*.md`, `~/.pi/agent/agents/` | `subagent` tool | per file | yes | `PI_*` |
| Hermes | none (briefed) | `delegate_task` | no | yes (10, background) | `HERMES_*` |
| ZeroClaw | none (briefed; stage 3 may render `[agents.<role>]` TOML) | `spawn_subagent`, `delegate` | per agent alias | `delegate` only | UNVERIFIED |
| Borg | none (briefed; roles in its database) | `spawn_agent` | per call | yes (3) | UNVERIFIED |

Delegation text per briefed harness, as `guide` prints it:

- Hermes: `delegate_task(tasks=[{goal: "Read <absolute brief path> and
  follow it", context: "<repo path>"}])`; children run in the background by
  default, wait for the completion message; `role` is ignored by the
  source.
- ZeroClaw: `spawn_subagent{prompt: "Read <absolute brief path> and follow
  it"}` (same agent, fresh session, blocks; the risk profile must not
  exclude it), or `delegate{agent, prompt, context}` to a same-profile agent
  in bounded mode.
- Borg: `spawn_agent(role="coder", blocking=true, message="Read <absolute
  brief path> and follow it", context="<task>")`, or a fresh `borg ask`
  session per brief; run inside the repo workspace (`-w` or `borg acp`),
  because `borg ask` defaults to the daemon's cwd.

The delegation paragraphs for the rendered harnesses are written in
`src/harness.rs`, which owns them.

Command gates: Hermes auto-denies dangerous child commands unless
`delegation.subagent_auto_approve: true`; Borg has a hardcoded deny list.
Instruction-file limits: Borg truncates project docs at 32 KiB across the
tree; Hermes takes only the first matching context file; ZeroClaw reads
`AGENTS.md` from its own agent workspace, not the repo.

### Per-harness detail

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
- Third-party subagent plugins (searched 2026-10-21, bar: ≥200 stars and
  maintained): none for Hermes or ZeroClaw. Hermes candidates are role prose
  or profiles, not runtime agents (`rlaope/oh-my-hermes` 3.5k stars,
  `witt3rd/oh-my-hermes` 370, `magnus919/hermes-profiles` 336,
  `Hermes-Bot-Mode` archived). ZeroClaw's official `zeroclaw-plugins` are
  WASM channel tools and `zeroclaw-skills` are prompt skills. Pi's
  `pi-subagents` remains the only plugin that adds file-defined agents.
- BAML Rust (spike 2026-11-12, crate and CLI 0.226.3, "BAML v0"; a v1 line
  exists): crate and CLI versions must match exactly; building the crate
  needs `protoc`; 136 crates, about 7.5 CPU-minutes cold against 25 s
  without. The crate `dlopen`s `libbaml_cffi` (51 MB on x86_64 Linux) on the
  first `parse`, `build_request`, or `call`, and by default downloads it
  from
  `https://github.com/boundaryml/baml/releases/download/<version>/libbaml_cffi-<target-triple>.so`
  with no published checksum, cached under `BAML_CACHE_DIR` or
  `~/.cache/baml/libs/<version>/`; `BAML_LIBRARY_PATH` and
  `BAML_LIBRARY_DISABLE_DOWNLOAD=true` override; a missing library panics
  in generated code. `parse()` is lenient (fuzzy enum match, type coercion,
  truncated JSON accepted) with one unstructured error string, so stewards
  does not use it. The generated types derive serde and validate strictly
  with no native library. The prompt and schema text come only from
  `build_request`, read out of a provider request body. A client with no
  `api_key` sends the literal `$OPENAI_API_KEY`. BAML logs the full prompt
  to stderr unless `BAML_LOG=off`. BAML rejects lowercase enum values, so
  enums carry aliases and the generator hook adds a serde rename; the hook
  also rewrites the client's crate path so it can live inside `llm.rs`.
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

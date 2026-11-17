# stewards — design record

The single record of what stewards is and why. Decisions here are settled;
reopen one by editing this file, not by drifting in code.

## Goals

Five, all primary, in the author's words:

1. Same behavior across harnesses. The whole contract with a harness is
   that its agent can start a subagent and give it one sentence; stewards
   never starts a session itself and holds no harness's command line, so
   it works the same in a harness it has never heard of. The block lives
   in `AGENTS.md` only; a harness that reads another file is pointed at
   `AGENTS.md` by the user, and the tool does not know which harness is
   in use.
2. Reviewed changes and a repository memory at a small, measured overhead:
   one review subagent per change, not a token saving.
3. Maximum code cleanliness.
4. Documentation generated from code.
5. Knowledge persists across sessions.

The saving was tested and not found: four thin-eval passes on 2026-11-12
(three on the fixture, one on this repository) cost `full` more tokens
than `bare` on every task; see "What the thin eval showed" in the Handoff.

Audience for year one: the author, across their own repos.

Feel: a team that already knows the repo. The agent makes the change and
hands its review to one subagent that already knows the area, with the one
sentence `stewards review` prints.

What persists: structure (areas, owners, anchors), contracts (interfaces and
expected behavior, with scenarios), and judgment (gaps, shortcuts, decisions).

## Settled decisions

### Shape

- **Harness contract.** stewards depends on three things a harness must
  provide: it reads the repo's `AGENTS.md`; its agent can run shell in the
  repo cwd; it can start a fresh-context child that can also run shell and
  write files, given one sentence as its task (the handoff). The tool never
  starts a session and knows no harness's command line: every role runs as
  the harness's own subagent, the same way.
  Two more are used when present: a per-call or per-file model pin (then
  roles run on the right tier), parallel children (then the review briefs of
  a round run at once). (A file format for a reusable role was a third, for
  `render`, which is cut from 0.1.0; see Build order.) Any
  harness meeting the required three is supported; the per-harness facts
  (paths, spawn tool, env detection, last-verified date) live in one
  `harness.rs` table and in Facts, never in the rules; a harness not in
  the table gets the generic guidance, not an error. The harnesses used
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
  from the usual env vars. Only `init --draft` honours it; no other command
  reads the flag, so `brief`, `submit`, `next` and `review` run in the
  handoff loop in every mode. It is the only mode that needs BAML's native
  library. A missing model entry, key, or library is an
  error, never a silent fallback, and the library is never downloaded
  unless `STEWARDS_BAML_DOWNLOAD` is set. When `BAML_LIBRARY_PATH` is unset
  and `<exe dir>/../lib/stewards/libbaml_cffi.<so|dylib>` exists (the layout
  of the release archive, the install script, and the AUR package), the tool
  uses that file. The tool never names a model itself.
- **Delivery:** the repo's `AGENTS.md` block is the one truth: the ritual,
  the handoff, the editing rules. Every harness reads it with
  no install. User-level instructions are optional accelerators (hooks are
  cut from 0.1.0; see Build order).
  A harness that reads instructions from its own workspace rather than the
  repo gets the same block there. `init` places the block at the top of
  `AGENTS.md`, because some harnesses truncate project docs or take only
  the first matching file. The block is written to `AGENTS.md` only, and no
  other instruction file is written or named by the tool. Some harnesses do
  not read `AGENTS.md` (checked 2026-11-13: one widely used harness version
  reads only its own file, and a one-line import of `AGENTS.md` in that file
  works). So `init` and `guide` say plainly that the block is in `AGENTS.md`
  and that such a harness needs one line in its own instruction file
  pointing at it (a line telling the agent to read `AGENTS.md`, or that
  harness's import syntax); the user lists that file in `instruction_files`
  of `repo.toml`, and `check` warns while it does not point at `AGENTS.md`.
- **Overhead:** each agent gets the exact context its job needs, no more,
  no less, and a change costs one review round (one brief per 48 KB of due
  units), not one job per unit and role.
  Eval runs and telemetry measure the overhead; nothing promises a saving.
- **License:** MIT, no attribution section. Clean room: no text from any
  earlier private project.

### Team

- **Roles, seven:** four per unit and three that work across them (the
  validator takes the bug's unit; the architect and the reviewer take none,
  so `pack`, `brief`, and `submit` omit the unit for them). Reviewer
  (reviews one change across every unit it touched: the maintainer's review
  of each unit's diff, its test command run, its card, scenarios, and docs
  kept true, never edits source), developer (implements one
  task of the maintainer's plan, writes only the files in its lane, never
  stamps), maintainer (explains, plans, splits a plan into developer tasks,
  reviews the diff for cleanliness and surgical scope, keeps the card
  current), tester (owns the contract and its scenarios, writes and runs
  tests, never edits source), documenter (writes the prose sections of the
  unit's card), validator (judges a bug from its reproduction, severity from
  evidence only), architect (plans changes that cross units, records flows
  and decisions, proposes unit boundaries).
- **Two flows:** the per-change flow is the reviewer: plain `next` folds
  every stamped unit due for a maintainer, tester, or documenter job into
  the reviewer jobs of one round (one per review brief, as many as the
  review budget needs), and `review` hands them off together. The per-role flow is for
  adoption (`init --draft`, then `review --roles` until it exits 0), for
  `review --roles` on request or before a release, and for plans
  (maintainer explain, developer tasks, architect). A unit's first review is always its maintainer's. A unit
  stamped through either flow is fresh for both.
- **First review records; it does not ask for changes:** a unit with no
  `[reviewed]` block has never been reviewed, so its maintainer review is a
  first review of existing code, not of a change. Its brief leads the role
  text with that (`brief::FIRST_REVIEW`): there is no diff; draft the card
  (purpose, interface, anchors) and record every problem as a gap in
  `card_patch.gaps` (`bug` with a repro when one can be shown, otherwise
  `gap`); the verdict is `approve`, with no findings. `submit` rejects a
  first review with verdict `changes` (field `verdict`) or any finding
  (`findings[i]`, naming its file and line), as `answer_parse_failed`, and
  the fix says to move each finding into `card_patch.gaps` and approve; it
  counts as any validation error. An accepted first review stamps as any
  approval does; later reviews of the unit may return `changes`. The
  reviewer is never given a never-stamped unit (`submit` refuses such a
  header with `no_brief`), so this holds for it too. Adopting stewards
  must never ask a user to rewrite existing code. (In evals, adoption
  changed the fixture's source on three of four harness and model pairs:
  first reviews returned `changes` about code that was already there, and
  `review --roles` then sent the agent to fix them.)
- **Team member, physically:** the same in every harness: a fresh-context
  child that reads a brief by path and runs shell. Every command that
  writes a brief prints its **handoff**, ``Read `<absolute brief path>`
  and follow it.`` (one function builds it, with the path in a
  single-backtick code span; a repo path with a backtick, a control or
  line separator character, or an invisible character is refused, so no
  directory name can end the span or add an instruction), and the agent
  passes that sentence unchanged as the whole task of a subagent that can
  run shell and write files (whichever agent type the harness has for
  that; the text names none, since Haiku on Pi read "general-purpose" as
  an agent name Pi does not have). The brief carries the role
  text, so every harness, known or not, gets the same guidance, word for
  word: `guide` names no harness's subagent tool (a name is wrong as soon as
  a harness renames it; checked 2026-11-13, one harness's tool had two names
  across versions). `render`, which would have written one file per role
  for a harness with a role-file format, is cut from 0.1.0 (see Build
  order); no guidance depends on it.
- **Model per role:** `~/.stewards/config.toml` maps role → model alias per
  harness; `--direct` builds a BAML `ClientRegistry` from the same table.
  Repo files never name a model.
- **Context:** pack by default, pull allowed. The tool computes the minimum
  pack for unit + role + task; the brief states a budget; the subagent may
  pull more by anchor and must report what it added in `pulled_anchors`.
- **Job loop:** `review` (as `next`) writes briefs to `.stewards/briefs/`
  and prints each one's handoff; the main agent starts one subagent per
  handoff with that sentence as its whole task; the subagent writes its
  answer as JSON to the path the brief names and runs `stewards submit
  <unit> --role <r> --file <f>`, fixing and retrying up to three times on a
  validation error, and replies with exactly what submit printed. Every
  role's accepted `submit` also writes what it printed to
  `.stewards/briefs/<job>.result.json`, and the next `review` reads,
  reports, and removes it, so the outcome does not depend on the
  subagent's reply: every field the file holds (the summary, a review's
  verdict per unit, each finding's file, line, `cut`, and
  `replace_with`, each non-blocking note, an explanation, unit proposals,
  warnings, and any other), each text on one line and capped. It is shown as what the
  subagent reported, not as something stewards verified. Every LLM step
  uses this loop, including `init --draft`.
- **Developers:** a maintainer has one developer by default and several
  when it splits its plan. Each task in the plan names its **lane**: the
  files it may write. `submit` accepts the plan only when every lane is
  owned by the unit and no two lanes share a file, so a unit's developers
  can run at once in one tree, as can developers of different units. The
  maintainer splits only work whose pieces build and check independently;
  anything else is one task. Reporting is by artifact, because most
  harnesses cap subagent depth at one: the maintainer's tasks become the
  developers' briefs, their combined diff is reviewed once (by the
  per-change reviewer, or with `--roles` the maintainer), and the main
  agent only starts a subagent per handoff. A developer is a fresh child
  per task, not a standing process; what makes it the unit's own is its
  pack. A change with no maintainer plan the main agent makes itself; the
  review is the same.
- **Lanes, checked:** a developer's answer does not list its files; at its
  `submit` the tool diffs the brief's snapshot against the tree and reports
  any changed file the unit owns that is in no current task's lane, as
  `out of lane` in the summary. It is a report, not a rejection: the write
  has happened, and the maintainer's review sees it as untraced.
- **Fix loop:** a review's blocking findings are the next task. A review's
  submit prints each finding's file, line, `cut`, and `replace_with`, and
  the next `review` reports them, so the main agent fixes them itself and
  runs `review` again. A finding marked `blocking: false` is a note: it is
  printed under `notes` by `submit` and `review` and counted in telemetry,
  never holds up approval, and makes no fix task.
- **Answers:** a finding the agent will not apply (it is wrong, or asks to
  remove deliberate behaviour) it answers instead of fixing: `review
  --answer "<unit>: <file>:<line>: <reason>"` (repeatable), the prefix
  copied from the one `review` prints beside each answerable finding
  (`<file>:<line>#<k>`, k from 1, when a unit has more than one finding at
  that file and line). One flag on the command the agent already runs,
  with no file to write, is what a weak model gets right most easily. A
  reviewer's `changes` submit stores the unit's blocking findings in the
  tool-owned `.stewards/briefs/<unit>-findings.json` beside its fix task
  (git-ignored, written through `Repo::write`, under the lost-update
  check, re-validated on every read as the task list is, each text under
  the prose cap); `--answer` records the reason there, after checking
  every answer of the call (an unknown finding, a unit with no such
  finding, a second answer, or an empty reason is `usage`, and none is
  written). The next review brief shows each answered finding with its
  reason under "Answered findings" in the unit's data section; the
  reviewer either accepts the reason (the finding is not raised again:
  it is kept as accepted, shown as such in later briefs, and named in the
  review summary as `accepted answers: <file>:<line>`) or contests it once
  in `contested[] {file, line, why}`, and a contested finding comes back
  as a blocking finding marked `contested` (with the why), in the fix
  task too. A finding is answered once: `--answer` for a contested one is
  rejected with a fix saying to apply it or change the task, and the
  `then` line names that state. An approval (a stamp) removes the file.
  The agent never stamps and cannot dismiss a finding without a reviewer
  seeing the reason. The per-role maintainer review neither shows nor
  settles answers; its approval removes the file too.
- **Task list:** an answer file stays
  writable after it is validated, so the tool does not read it again: at
  the maintainer's or reviewer's `submit` it writes the accepted task list to its own
  file, `.stewards/briefs/<unit>-tasks.json` (an explain answer's `tasks`,
  or a review's blocking and contested findings as one task whose lane is
  the files they name; an approval removes it), and `brief <unit> --role developer`
  re-validates that file every time it reads it. A developer that reports `blocked_on` sends the main agent
  back to the maintainer with that message as the task.
- **Tests, two writers in turn:** a developer writes the check for its task
  inside its lane; the tester writes scenario tests for the contract and
  may edit any test under the test roots. They never overlap: developers
  run in the changing step, the tester in a `review --roles` round after it.
  The per-change reviewer writes no tests: it runs the unit's test command
  and may upsert scenarios.
- **Job files:** one open job per (unit, role), or per task for developers,
  and one review round per repo. The brief is
  `.stewards/briefs/<unit>-<role>-<snap>.md` (`architect-<snap>.md` for
  the architect, `review-<n>-<snap>.md` for review brief `n` of a round,
  numbered from 1 even when the round has one,
  `<unit>-developer-<n>-<snap>.md` per task), `<snap>` the first 8 hex
  characters of the snapshot it was packed from; the answer is
  `<same>.answer.json`, what its accepted submit printed
  `<same>.result.json`. One function (`brief::rel_path`) builds these
  names and everything that reads one back (`submit`, `review`, the
  eval's handoff detection) checks it against that function. A handoff
  is therefore bound to its snapshot: one kept from an older round, or
  started before `review` wrote the brief, names a file that does not
  exist (twice a subagent was started with the fixed sentence before the
  brief was written, or in the same step as an edit). Writing a brief
  removes the job's stale answer and the briefs and answers of the same
  job at any other snapshot (for a review brief, of any review brief of
  any other round), through `Repo::remove`, and only files whose names
  `rel_path` gives for that job; writing a round also removes the review
  briefs numbered past its last; result files are left for `review` to
  report. So nothing accumulates. The briefs of one round are
  independent: a reviewer's `submit` writes only its own units' cards,
  test cards, task and findings lists, and pages (the units of a round's
  briefs are disjoint), plus the unchecked docs index; it stamps each unit
  with its fingerprint at its own header's snapshot, and the `.stewards/`
  and docs files it writes belong to no unit, so no other unit's
  fingerprint moves; the answer's `snapshot` is compared with its own
  brief's header, not with the tree; and it retires only its own brief.
  So the other briefs of the round submit as they would have, in either
  order. The brief's first line is a
  tool-owned header recording the
  mode, the snapshot it was packed from, the pack's bytes and anchor count,
  the bug title for a validator job, and the units a reviewer job covers;
  `submit` reads it, and refuses (`no_brief`) a header whose snapshot is
  not the one in the brief's name. `submit` finds the brief by the answer
  file's name when it is the job's `<same>.answer.json` (a developer's
  must be, as it carries the task number), so an answer to an older
  snapshot's brief finds none; an answer named otherwise takes the job's
  one brief on disk (for the reviewer, the one review brief of any
  number), and with more than one it is `no_brief`, never a guess. A successful
  `submit` removes the answer with the brief when `--file` is that
  `.answer.json` path; a failed one leaves both for a retry.
- **Stamp:** only an approval stamps: a maintainer review `submit` with
  `verdict: approve`, or a reviewer's unit with `verdict: approve`. It
  stamps the brief's snapshot, not the tree at submit time,
  so anything written since (a parallel tester's tests, an applied finding)
  is still stale and gets its own review. `approve` means no blocking
  finding (a contested finding is blocking) and `changes` means at least
  one; `submit` rejects either verdict otherwise (`check_review`, shared by
  the maintainer and the reviewer). An approval may carry notes.
  A reviewer cannot approve a unit whose test command it reports failing.
  Explain jobs, `changes` verdicts, and the other roles never stamp. `stamp <unit>` is the human
  override. A stamp records that one reviewer, shown this snapshot's diff,
  reported no blocking finding; it is not proof the code is right (two reviewers
  given the same brief, model, and code once returned an approval and a
  real finding), and `review --roles` is the deeper pass. `status` and
  `guide` say so.
- **Role marks:** the tester's and documenter's submits each record the
  snapshot they finished at, beside the stamp. Their jobs are due by what
  changed since their own mark, not since the stamp, so a fix round that
  does not touch their paths does not run them again, and a tester's own
  new tests do not call the tester back. With no mark, the stamp is used;
  when a new stamp replaces it, a role with no mark takes the old stamp as
  its mark, so a pending tester or documenter job survives the maintainer's
  approval. A reviewer's approval sets both marks to the stamp, since the
  reviewer answered for all three roles.
- **Triggers:** before changing, `route` the files; a change the main agent
  cannot plan from the unit pages goes to the owning maintainer to explain
  and plan, or, for two or more units or no owner, to the architect. To change, `brief <unit> --role developer` writes one
  brief per task of the maintainer's plan and prints one handoff each; a
  change with no plan the main agent makes itself. A step of an open
  architect plan that has not passed carries `todo` in `next.plans`: make
  the change yourself, or run `stewards brief <unit> --role maintainer
  --task "<change>"` to split it into developer tasks. Before reporting done, `review` lists what
  `next` finds: the units due, in the review briefs of one round, which
  together cover every stamped one, plus a maintainer first review for
  each never-stamped one; `review
  --roles` lists the per-role jobs of `next --roles` instead: maintainer review for every stale unit and every stamped one with a dead
  anchor; tester when a
  change since its mark touched an interface's lines or a file under
  `test_roots`; documenter when a change since its mark touched an
  interface's lines (a change only in a private helper an interface calls
  does not count; the maintainer's `tester_needed` covers it; outside the
  languages the line rule knows, any change to the interface's file
  counts); architect when
  two or more units changed since their stamps and no plan covers them;
  validator for a bug with no severity. Done means `review` exits 0: it
  lists no job and no finding is left to fix.
  `check` covers review only (stamps, anchors, scenario ids): it is what
  CI runs.
- **Modes:** `next` writes review briefs; `brief` writes explain briefs,
  with `--task` as the question. The header line carries the mode.
- **Review diff:** the maintainer's pack carries the diff from the unit's
  stamped tree to the current snapshot, so a re-review after a finding sees
  only what changed since. A unit never stamped, or whose stamped tree git
  has pruned, gets the full pack and no diff: a first review (only a never
  stamped one is held to the first-review rule above).
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
  the reviewer (its maintainer with `--roles`). Agents never hand-edit cards.
- **Staleness:** `[reviewed]` records the stamped snapshot tree, the unit's
  fingerprint in it (sha256 over the blobs of the files it owns), and the
  commit for display. `check` fails when a unit's current fingerprint
  differs from its stamp or it has no stamp, an anchor no longer resolves,
  or a scenario references an unknown interface id. Uncovered interfaces,
  overlapping globs, and files no unit owns warn, except those `repo.toml`'s
  `unowned_ok` globs list. Runs in the ritual and in CI.
- **Unowned changes:** a file no unit owns is shown to no reviewer. `next`
  and `review` list every file in the snapshot that no unit owns and
  `unowned_ok` does not list, and that changed or was added since the
  oldest stamp of its repo (a tree has no time, so: since any unit's
  stamp), as `unowned_changed`, with the line `These changed files belong
  to no unit and were not reviewed: <paths>. Add them to a unit in
  .stewards/units.toml, then run stewards review again.`; neither is clean
  while one is left. `unowned_ok` is for the files a repo means to leave
  unowned: root config and docs such as `Cargo.toml`, `README.md`,
  `.gitignore`, `AGENTS.md`, `LICENSE`, CI files. (Twice, source or test
  files added on a branch had no unit, so no reviewer saw them, and only a
  `check` warning said so.)
- **Anchors:** `path::name`. Resolved by text in stage 1 (file exists, whole
  word match). With tree-sitter (stage 3), `name` must be a definition per
  the grammar's `tags.scm`; a mere mention is dead. Static grammars (stage 3),
  by extension: rust `rs`; python `py`; typescript `ts mts cts`, tsx `tsx`;
  javascript `js jsx mjs cjs`; go `go`; bash `sh bash`; c `c h` (a C++
  header is parsed as C); cpp `cc cpp cxx hpp hh`; c-sharp `cs`; hcl `tf
  hcl`; yaml `yml yaml`. A definition is each `@name` capture inside a
  `@definition.*` capture of the grammar's own tags query (TypeScript adds
  its crate's query to JavaScript's), plus `src/anchors/*.scm`: Rust
  `const`, `static`, enum variants and struct fields (tags.scm omits them),
  YAML mapping keys, Bash functions, and HCL blocks named by their labels
  joined with `.` (`resource "aws_s3_bucket" "logs"` is
  `aws_s3_bucket.logs`, `variable "region"` is `region`). A definition's
  span is its node (a C declarator widened to its function, a decorated or
  exported item to the wrapper) and the comments and attributes directly
  above it. A qualified name (`a::b`, `a.b`) matches a definition named
  as a whole first (HCL, YAML keys with dots); else its last segment must
  be a definition inside a definition named by the segment before it, or
  inside a Rust `impl` of that type (so `Router::owner` is alive and
  `Other::owner` dead), or under that YAML key; only the immediate
  qualifier is checked. Runtime grammars: `<stewards home>/grammars/<lang>/`
  holds `lib.so` (`lib.dylib` on macOS) exporting `tree_sitter_<lang>` (`-`
  in the name read as `_`) and `tags.scm`, an optional `extensions` file
  listing file extensions one per line (without it, the directory name is
  the one extension). A static grammar wins over a runtime one for the same
  extension; of two directories claiming one extension the first by name
  wins. The library loads with `libloading` once per process, on the first
  file with its extension (an unfiltered `check`, and so `next` and
  `review`, loads every grammar directory to validate it), and stays loaded; it is refused, with a reason, when
  its language ABI is outside the range the linked `tree-sitter` accepts,
  it has no such symbol, `tags.scm` does not compile for it, or `lib.*` or
  `tags.scm` is not a regular file (links are followed: it is the user's
  own home). A runtime language then behaves as a static one; one that
  failed to load leaves its extension on the text rule, and `check` warns
  `grammar_unusable` (see Rules › check; `check <unit>` and `pack` do not
  repeat the warning). BAML, Markdown, TOML,
  Java, Ruby and every other language keep the text rule. (simplified: Go
  methods and C++ out-of-line members are not scoped by their receiver or
  class; `Type.Method` is dead there until a supplement query names them.)
- **Repo-level knowledge, no oracle agent:** `repo.toml` holds `[[flow]]`
  and `[[decision]]` next to repo-wide facts. `pack` includes the flows that
  touch the unit. The architect's pack is every unit's purpose and interface
  ids plus all flows and decisions; no anchors, no source.
- **Plans:** the architect's steps are state in `.stewards/plans/<id>.toml`;
  `next` runs each stale unit's step check through the allowlist, and again
  each step whose stored result is `fail` (so a step that failed and was then
  stamped can still pass), marks pass or fail in the step's `result`, and
  closes the plan when all pass and every unit it names is fresh again; a
  closed plan is deleted (git history keeps it). An open plan does not hold
  `review` back. A step whose unit never changes is never run, so its plan
  stays open: delete the plan file by hand.
- **Debt:** `simplified:` comments are computed on demand by `debt` and
  attached to the owning unit by route; never stored.
- **Bugs:** a tester's or reviewer's `bugs[]` and `validate --report` both
  land on the owning card as `[[gap]] kind = "bug"` with the repro, keyed by title
  (`validate --report` writes through the lost-update check); a
  tester's bug that names an interface takes that interface's anchor as its
  `anchor`, and one that names none keeps the anchor it had. The tester's
  and reviewer's packs list the unit's bug gaps under `Known bugs` (title,
  anchor, note, severity), saying to reuse the exact title when reporting
  the same defect. A bug with a new title whose anchor is that of a bug
  the card already has, judged or not, is a probable duplicate: `submit`
  does not add it and says so in its summary (`bug "<new>": possible
  duplicate of "<title>"; reuse that title to update it, or give a
  different anchor`), and the submit still succeeds. (The same defect was
  twice recorded under two titles by a tester and a reviewer.) The
  validator's submit sets `severity`, or removes a bug it judges not real;
  the maintainer closes a fixed one like any gap.
- **Docs:** `docs/units/<unit>.md` is rendered from the card; the
  documenter, or the reviewer through a unit's `doc`, writes only the prose
  sections; a reviewer's submit renders the page again for every unit whose
  card or test card it wrote and which has docs; `docs/units/README.md` is
  the human index with flows and decisions.
- **Memory:** `~/.stewards/memory.toml`, hand-edited one-liners with optional
  language or role filters; roles never write it; matching entries are
  packed into every brief under a byte budget.
- **Workspaces:** `stewards.toml` with `repos = [...]` in a parent folder.
  It is the user's own file and is trusted: its paths may be absolute or
  reach through links, so it belongs outside any repo it lists.
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
  it through the allowlist; a failing run (the command exited non-zero) is
  proof, a timeout or a command that could not start is not (`reproduced:
  false` with a `reason`); the validator assigns
  severity from evidence, and judges `real` by reasoning only when no
  runnable repro exists. `validate --report <file>` takes one strict JSON
  object (`{"unit", "title", "repro", "evidence", "interface"?}`, unknown or
  duplicate fields rejected, the answer size cap), checks it as a tester's bug
  (`unknown_unit`; `answer_parse_failed` with `errors[]` and the shape as the fix),
  records it, runs the repro, and writes the validator brief for that bug,
  printing `{recorded, duplicate?, repro_result, reproduced, brief_path,
  handoff, then}`; a probable duplicate is not added and writes no brief.
  Every validator brief that `next --roles` or `brief` writes runs the repro
  first; `next --no-brief` and `pack` run nothing. `--export` creates an issue on GitHub, GitLab,
  Jira, or Linear (`repo.toml` `[tracker]`; auth from `gh`, `glab`,
  `JIRA_URL` and `JIRA_TOKEN`, `LINEAR_API_KEY`; Jira and Linear through
  their REST and GraphQL APIs), once per bug: the issue URL is recorded on
  the gap. It files every bug gap with a `severity` and no `issue`, in
  units.toml then card order, and writes each issue's URL to its gap as
  soon as the issue exists (lost-update checked), so a later failure never
  loses a filed issue; it stops at the first failure. GitHub and GitLab go
  through `gh api` / `glab api` (`POST repos/<owner>/<repo>/issues`,
  `POST projects/<id>/issues`, the body as JSON on stdin, `--hostname` the
  `origin` host when origin is that forge and its path is the `[tracker]`
  project, else `github.com`/`gitlab.com`).
  Jira: `POST <JIRA_URL>/rest/api/2/issue` (issue type `Bug`), the URL
  `<JIRA_URL>/browse/<key>`; `JIRA_EMAIL` set → Basic `email:token`, else
  Bearer `JIRA_TOKEN`; `JIRA_URL` must be https with no user info, `?`,
  `#` or `\`. Linear:
  `POST https://api.linear.app/graphql`, the team by key, then
  `issueCreate`, values only as GraphQL variables. HTTP has a 30 s timeout
  and follows no redirect. A returned URL is stored only when it is
  printable ASCII with no space, at most 300 characters, and under the
  tracker's own issue address (`https://<host>/<owner>/<repo>/issues/`,
  `https://<host>/<path>/-/issues/`, `<JIRA_URL>/browse/`,
  `https://linear.app/`). The URL is written to the first bug gap with the
  same title, a `severity` and no `issue`. A missing login or variable, or
  HTTP 401/403, is `tracker_auth`; any other tracker failure is
  `tracker_failed`; a local failure keeps its own code. A failure after the
  issue exists names its URL and says to set it on the gap by hand.
  Prints `{exported[] {unit, title, url}, then}`; `then` is `done`, or,
  when bugs without a severity exist, says to judge them with
  `review --roles` first.
- **Git and PR:** (`init --hooks`, a pre-push hook running `check`, is cut
  from 0.1.0; see Build order.) `next --pr <n>` posts the report (the `--human` text, in a code fence
  longer than any backtick run in it, so card and file text cannot ping,
  link, or format) as one PR or MR comment updated in place;
  `review --pr <n>` is `review` plus posting what that call reported (each
  finding and note as `file:line: cut -> replace_with`, per unit verdicts, the
  jobs listed, and the `then` line) the same way; both add `pr_comment {url,
  updated}` to the report. The comment starts with a marker line
  (`<!-- stewards:next -->`, `<!-- stewards:review -->`); updating lists the
  PR's comments and edits the first whose author is the logged-in user and
  whose body starts with the marker, else creates one (two concurrent runs
  can each create one). The `next` text omits each job's handoff (a local
  path). A failed post does not hide the report or change the exit code: the
  report carries `pr_comment: null` and `pr_comment_error` (one line). The
  repo and host come from `origin` (`repos/<owner>/<repo>` or
  `projects/<encoded path>` and `--hostname`), not from `gh`/`glab`'s default
  remote. The forge is GitHub for exactly `github.com`, GitLab for exactly
  `gitlab.com`; any other host is the forge whose CLI reports a login to it
  (`gh auth status --hostname`, then `glab`), else a `usage` error that names
  host and path, never userinfo. `init --ci` writes
  `.github/workflows/stewards.yml` (GitHub Actions) or `.gitlab/stewards.yml`
  (GitLab CI; `.gitlab-ci.yml` is never touched, the `include:` lines are
  printed) for that forge. The GitHub template pins `actions/checkout` by tag,
  not SHA, and `install.sh`'s checksum comes from the same release as the
  binary. The GitLab template needs `glab` on the runner image, and a
  project access token (Reporter, `api` scope, short expiry) in the
  `GITLAB_TOKEN` variable, which every branch pipeline that can see the
  variable can read. The CI job installs the release
  matching this binary, runs `next --no-brief --no-run --pr <n>` on pull
  requests (allowed to fail), then `check`, which decides the job. CI has no
  stamped trees (they are local unreachable objects), so `next` there lists
  due units with every owned file as changed and no hunks; `check` needs none.
- **Telemetry:** `.stewards/telemetry/<yyyy-mm>.jsonl`, gitignored, one line
  per submit: unit, role, task, pack bytes and anchor count, pulled anchors,
  harness, timestamp, for a reviewer the number of units it covered, and
  for a review its number of notes (non-blocking findings). Token counts come from eval transcripts, never from
  what a subagent says about itself.
- **Evals:** `stewards eval` drives Pi, Claude Code, and Hermes over a
  purpose-built fixture, three arms, graders for the ritual gate, the
  overhead report, and two report-only checks (bug fixed, plan stored and
  closed); on demand, not in CI. Manual spot checks for the other harnesses.
  Not in 0.1.0: a Codex driver, the cleanliness and docs-accuracy LLM
  judges, and eval in CI on release tags (cut 2026-11-15).
- **Self-test:** Rust tests against fixture repos in `tests/fixtures/`; BAML
  `test` blocks run by `baml-cli test` in CI with a key.
- **Distribution:** `cargo install stewards`; GitHub releases with the
  matching `libbaml_cffi` beside the binary and an install script; an AUR
  package; shell completions and a man page. (A thin Claude Code plugin
  whose SessionStart hook ran `stewards guide` is cut from 0.1.0; see Build
  order.)
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
  schema, and submit instructions; a subagent gets it through a handoff.
- **submit**: the command that validates a typed answer and writes it.
- **snapshot**: a git tree of the working tree as it is now: tracked,
  modified, and untracked-but-not-ignored files.
- **stamp**: recording that a unit was reviewed at a snapshot: one reviewer,
  shown that snapshot's diff, reported no blocking finding. It is not proof
  the code is right; `review --roles` is the deeper pass.
- **fingerprint**: sha256 over the blobs of the files a unit owns in a
  snapshot; **stale**: the current fingerprint differs from the stamped one.
- **job**: one task `next` decides is due: a review brief of a round, which
  together cover every unit a change touched, or with `--roles` one per
  (unit, role).
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
instruction_files = []                   # other instruction files a harness reads, repo-relative
unowned_ok = ["Cargo.toml", "README.md"]  # optional; globs (as unit paths) of files that may belong to no unit
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
tester = "<snapshot tree sha>"           # role marks: where the tester and documenter last finished; a reviewer's approval sets both to its stamp
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
check = "pytest tests/api -k priority"   # must pass the allowlist for the step's unit
result = "pass"                          # pass | fail; written by next, absent until the check first runs
```

`<id>` is `<created>-<slug>`: `created` is the UTC date the architect's
answer was submitted, the slug the title lowercased with each run of
characters other than `a-z0-9` as one `-`, at most 40 characters (`plan` when
none is left), and `-2`, `-3`, ... on a clash. `submit` rejects a plan with
no step, a step whose unit is not in `units.toml`, or a check the allowlist
refuses; interface ids are not checked (a plan may name one the change adds).
`load_repo` reads every `plans/*.toml` through no link; a parse failure or a
step naming an unknown unit is `invalid_toml`.

`stewards.toml` (workspace, in a parent folder)

```toml
repos = ["backend", "frontend"]          # paths relative to this file
```

User-level files live in `$STEWARDS_HOME` when it is set, else `~/.stewards`:
`config.toml` (`[models.<harness>] <role> = "<alias>"`, and `[models.direct]
<role> = "<provider>/<model>"` for `--direct`, providers anthropic, openai,
google-ai), `memory.toml` (`[[entry]] text, languages?,
roles?`), `grammars/<lang>/{lib.so|lib.dylib, tags.scm, extensions?}` (see
Anchors).

Anchor grammar: `path` is the text before the first `::`, `name` is the rest
(Rust names contain `::`). A text match is an occurrence of `name` whose
neighbouring chars are not `[A-Za-z0-9_]`; case-sensitive; hand-rolled char
check, no regex crate. `name` is the bare identifier (`dispatch`, `Router::owner`), never
a keyword phrase such as `fn dispatch`: `submit` rejects a name with whitespace.

Snapshot: copy `.git/index` to a temporary file, then `git add -A` and `git
write-tree` with `GIT_INDEX_FILE` pointing at it. The tree covers modified
and untracked files, honors `.gitignore`, and leaves the real index, HEAD,
and refs untouched. The copy keeps the real index's modification time:
git re-reads a file only when its mtime is not older than the index's, so a
fresh copy would hide a same-size edit made in the same second as the last
`git add`. One snapshot per command.

Fingerprint: sha256 over the sorted `path blob` lines of `git ls-tree -r
<snapshot>` for the files the unit owns by route.

Unit names match `[a-z0-9][a-z0-9_-]*` (they become file names); anything
else is `invalid_toml`. `.stewards/**`, `docs_path/**`, and `.git/**` (any
path with a `.git` component) are tool-owned, compared ignoring ASCII case:
never routed, never reported as unowned, so no unit ever owns them.

## Command surface

| Command | Does |
|---|---|
| `guide [--harness h] [--all] [--errors] [--glossary]` | prints the ritual, the handoff, roles, editing rules, adoption, delegation text for the detected or named harness (the generic paragraph for one not in the table), error codes, glossary |
| `init [--draft] [--ci]` | step 1 proposes `units.toml` and `repo.toml` from facts, writes the AGENTS.md block, and says (`note`) that a harness not reading `AGENTS.md` needs a line pointing at it; `--draft` briefs the naming and card drafting (each first review's task: draft the card, record each problem as a gap, approve; existing code is not changed), prints each brief's handoff, and says to start those subagents, noting that a first review never asks for existing code to change, then run `review --roles` until it exits 0; `--ci` writes the CI template of the origin's forge, adds its path to `unowned_ok` unless a unit owns it, leaves an identical file, refuses a different one (`usage`), and prints `{written, unowned_ok_added, then}` |
| `status [--telemetry]` | units, freshness, coverage, gaps, across the workspace; uninitialized repos; `stamp_claim`, what a stamp claims |
| `route <path>...` | owning unit per path, `null` when none; no LLM: an unowned path goes to the architect |
| `pack [unit] --role <r> [--diff] [--task t]` | the context pack; no unit for the architect or the reviewer (the reviewer's is `groups[]`, one review pack per brief of the round, every due unit in exactly one) |
| `brief [unit] --role <r> [--task t] [--stdout]` | writes an explain brief (the review briefs of the round for the reviewer, listed under `briefs`, a `usage` error when no stamped unit is due; one brief per task for the developer), prints its path, size, and handoff |
| `submit [unit] --role <r> --file <f>` | validates a typed answer strictly (the BAML-generated types through serde) against the job's brief header, writes the update and what it printed to `<job>.result.json`; an approving maintainer review, or a reviewer's approved unit, stamps |
| `stamp <unit>` | records `[reviewed]` at the current snapshot; the human override |
| `next [--task t] [--no-brief] [--roles] [--no-run] [--pr n]` | finds the units due, runs `check` and (unless `--no-run`) plan step checks and flow checks, writes the review briefs of the round (with `--roles`, the per-role briefs), prints the jobs, each with its handoff and, for a reviewer job, the units it covers |
| `check` | failures: stale or never-stamped units, dead anchors, unknown interface ids; warnings: uncovered interfaces, overlapping globs, unowned files (outside `unowned_ok`), `instruction_files` entries that do not point at `AGENTS.md`, `grammar_unusable`: a directory under the stewards home's `grammars/` that could not be used, naming it and the reason, with `rebuild it against tree-sitter ABI <min>..<max>, or remove <dir>` (never a failure; `next` and `review` carry it with the other warnings) |
| `validate [--report <f>] [--export]` | `--report` takes one JSON object `{unit, title, repro, evidence, interface?}`, records the bug on its unit's card, runs the repro through the allowlist, writes the validator brief (neither flag: `usage`); `--export` files an issue for every judged bug that has none |
| `review [--task t] [--roles] [--answer a]...` | records each `--answer` (`<unit>: <file>:<line>: <reason>`, a reason not to apply a finding of the last review, for a reviewer to accept or contest), then reports, as what subagents reported (not verified), and removes the result files the last round's submits wrote, at most 20 per call, then lists what `next` (`next --roles`) lists for the task in force (`--task`, else the one the last `--task` stored), each job with its handoff, and one `then` line; never starts a session; exit 0 only when no job is listed, no finding is reported, and no result file is left |
| `review --pr <n>` | `review`, then posts what this call reported as one PR or MR comment updated in place (`src/forge.rs`: `gh` or `glab` by the origin host) |
| `debt` | `simplified:` comments attached to units |
| `docs [unit]` | renders `docs/units/<unit>.md` and the index |
| `eval [--harness h] [--suite s] [--arm a] [--task n]... [--runs n] [--max-cost usd] [--no-developers] [--model id] [--thinking level] [--setup-dir d] [--keep-work]` | runs the eval suite; exit 0 when the suite passes, 1 otherwise |
| `completions <shell>` | the completion script of `bash`, `zsh`, `fish`, `elvish`, or `powershell` on stdout (a bad shell is a `usage` error), one line on stderr saying where to save it |
| `man` | the roff man page on stdout, one line on stderr saying where to save it |

Global flags: `--human`, `--repo <name>`, `-C <dir>`, `--direct` (honoured by `init --draft` only). JSON by
default, markdown with `--human`. Exit 0 ok, 1 work is due (`check`
failures; for `next`, also any job, failed flow or plan step check, or
unowned changed file; for `review`, any job listed, finding reported, result
file left, failed flow or plan step check, or unowned changed file), 2
errors.

## Rules

### Pack

```
Pack { unit, role, task, budget, bytes, over_budget, purpose, paths[],
       interface[], anchors[{path, name, line, source, source_start}], scenarios[],
       test_command, primary_paths[], test_commands[], gaps[], known_bugs[], flows[],
       decisions[], units[{name, purpose, interface_ids, paths, files}], doc,
       changed_files[], hunks[{path, diff}], other_changed_files[], memory[],
       bug, dev_task, repro_result, dropped[] }
repro_result (validator only): the run's {command, exit_code, pass, timed_out, output}
  plus reproduced (exit_code non-zero) and, when not reproduced, reason; null = not run
  (`pack`, `next --no-brief`), rendered as "not run".
```

Absent sections are `null`. `--diff` sets `changed_files` the way `next`
does, restricts `anchors` to those whose path is in it, fills `hunks` with
the unit's own files' diff (`git diff -U3 <stamped tree> <snapshot>` per
file) and `other_changed_files` with what other stale units changed. With
no usable stamp (never stamped, or the stamped tree is gone) the
maintainer's `--diff` pack is the full pack, a first review, plus
`changed_files` set to every file the unit owns in the snapshot (sorted, as
`next` lists them) and no `hunks`: rendered as an `Owned files` list noting
the unit has not been reviewed yet, capped at 200 paths then `(+N more)`;
the JSON carries them all. `fit` drops this list after flows and before
hunks are truncated.

| role | sections |
|---|---|
| maintainer | purpose, paths, interface, anchors (+source), gaps, flows, hunks + other_changed_files with `--diff` (owned files with no usable stamp) |
| tester | paths, interface (+source at interface anchors), scenarios, test_command, primary_paths, known bugs, hunks with `--diff` |
| documenter | purpose, paths, interface, doc prose, flows |
| validator | purpose, paths, interface, primary_paths, the bug, test_command, commands.test, repro_result |
| architect | every unit's purpose, interface ids, paths, and owned-file count, all flows, all decisions; no anchors, no source |
| developer | purpose, paths, interface, its task (change, lane, check), anchors (+source) in its lane, gaps, flows |
| reviewer | per covered unit: why it is due, the maintainer's `--diff` sections (hunks from the oldest of its stamp and role marks; its bug gaps as known bugs, the other gaps as gaps) plus scenarios, test_command, commands.test, doc prose, and its answered findings; once for the brief, the files changed in units not due |

A reviewer's pack is one section per unit its brief covers, under one
budget of 48,000 bytes for them all (a constant, not a `[budgets]` key). A
round covers every due unit at once: units join a brief in `units.toml`
order while its total fits, and the first that does not starts the next
brief, so a round has as many briefs as the budget needs and every due
unit is in exactly one (`pack --role reviewer` prints them as `groups`).
A unit alone over the budget is shrunk as any pack is (below), then loses
its doc, then its scenarios, then has every fenced body left (purpose,
sources, hunks) cut to one length until it fits, each cut named on its
section's "Left out to fit" line, and no other unit joins its brief. (In
the 2026-11-14 eval a multi-unit change took one round per unit, the
rest deferred, and most agents quit before the last.) Each unit's section
opens with a "Due because" line: the reasons of its jobs, and, when the
agent answered findings of the unit's last review, an "Answered findings"
table (file and line, the finding, the agent's reason, and its state:
answered, accepted, or contested), which is never dropped. Its hunks run from the oldest of its stamp, tester mark, and
documenter mark whose tree exists (a tree has no time, so the oldest is the
one the most of its files changed since), so a unit due only because a
mark is behind its stamp shows the change that mark has not seen.
A source window is, in a grammar language, the definition's span (the node,
the comments and attributes directly above it) cut at 41 lines then one line
`… (<n> more lines)`; in any other file, 20 lines before and after the match.
`line` is the match's line and `source_start` the file line the source
begins at. Budget is bytes of the `--human` rendering. Shrink order:
halve every source window once (the match line stays, with ten lines above and below); drop windows last-listed first; drop gaps; drop known bugs;
drop flows; truncate hunks to their first 40 lines each. Purpose, interface,
scenarios, and `other_changed_files` are never dropped (but for the review
pack's last cuts above). If still over,
`over_budget = true` with the size and no further truncation. Every removal
is listed in `dropped`. Files matching `[pack] exclude` are named, never
inlined.

### Route

Repo-relative forward-slash paths matched with `globset`
(`literal_separator(true)`). Specificity: longest literal prefix, then fewest
`*`/`**`; remaining tie → first unit in `units.toml`, and `check` warns on
two units matching the same tracked file at equal specificity.

### check

Each `instruction_files` entry of `repo.toml` is a repo-relative path of
plain names (no absolute path, `..`, or `.`), refused on load with
`unsafe_path` otherwise. `check` adds a warning of kind
`instructions_unlinked`, never a failure, for each listed file that is
missing, is not a regular file or is a link (never followed), is over
1 MiB, or does not contain the text `AGENTS.md`; its message names the
file and says it should point at `AGENTS.md`. `next` and `review` carry it
with the rest of the check's warnings.

### next

One snapshot per repo per run, shared with the embedded `check`; each job
carries the snapshot of its repo. A unit is stale when its fingerprint in the snapshot
differs from `[reviewed].fingerprint` or it has no `[reviewed]`. Its changed
files are `git diff --name-only <stamped tree> <snapshot>` restricted to the
files it owns; with no stamp, or a stamped tree git has pruned, every owned
file counts as changed and the pack has no hunks. There is no base ref:
committed, staged, unstaged, and untracked changes are all just distance
from the stamp. The per-role jobs, which `next --roles` lists:

- every stale unit → maintainer review; so is a stamped unit, stale or not,
  whose card has an anchor `check` calls dead (reason `dead anchor:
  <path::name>`, one per anchor, after `stale` when both hold): only a
  reviewer's card patch fixes it, and the text rule's mentions die when the
  grammar judges them in code that did not change
- a change since the tester's mark touches an interface's lines, or a file
  owned by the unit and under `test_roots` changed → tester
- a change since the documenter's mark touches an interface's lines →
  documenter
- an interface's lines in a file: in a grammar language (see Anchors), the
  lines of every span of a definition the anchor's name resolves to (the
  definition's node and the comments and attributes directly above it, no
  blank line between); a mention gives none. In the languages
  `java kt rb php swift scala` the text rule: a block around every
  whole-word match of the anchor's name, and of its last segment when the
  name holds `::` or `.`. Down, the block runs from the match's line until
  a line whose previous line is blank, whose indent is at most the match
  line's (a tab is 8 columns), and before which every `(`, `[`, `{` opened
  since the match line is closed. Such a line that is a comment (it starts
  with `#`, `//`, `/*`, `*` or `--`) ends the block only if the next line
  that is neither blank nor a comment would too, or the file ends first.
  Trailing blanks are dropped. Up, it takes the non-blank lines directly
  above, then, if the nearest non-blank line above their blank lines starts
  with `/`, `*`, `#` or `@`, those blanks and the run above them, once.
  Items not parted by a blank line merge into one block. (simplified: this
  text rule can end a block early, at a blank line then a column-0 line
  inside a triple-quoted string; the upgrade is the language's grammar.)
- a hunk of `git diff -U0 <mark> <snapshot>` touches an interface when its
  new-side range or the line just before or after it is one of the
  interface's lines in the snapshot's file, or its old-side range or a
  neighbour is one of them in the file at the mark. The reason is
  `interface changed: <anchor>`. When the lines can't be found (a file in
  a language with no grammar or text rule such as Markdown or TOML, name
  not defined, file gone,
  no hunks for it, mark tree pruned, a blob unreadable, as for a file added
  since the mark) the file changing is enough:
  `interface file changed: <path>`. A change only in a private helper outside
  every span touches no interface, even when an interface calls it; the
  maintainer's `tester_needed` covers that.
- tester and documenter jobs do not need the unit to be stale; with no
  mark they measure from the stamp, and a never-stamped unit gets none
  until its first review stamps it (a validator job does not wait)
- 2+ units changed since their stamps (never-stamped units do not count)
  and no open plan of the repo naming all of them → architect
- a bug gap with no `severity` → validator, one per unit per round
- added lines in `git diff -U0 <stamped tree> <snapshot>` that match the debt
  rule → listed under the owning unit's maintainer job
- a flow with a `check` and a stale or never-stamped unit → the command
  runs through the repo's allowlist (`commands.test` only; unless
  `--no-run`) into `flow_checks[] {flow, command, pass, output}`; a timeout
  or a command the allowlist refuses is a failure whose output says so
- each open plan → `plans[] {id, title, closed, steps[] {unit, change,
  check, result, fresh, output?, todo?}}`: a step runs its `check` through
  the allowlist of its unit (unless `--no-run`) when the unit is stale or
  never stamped, or the step's stored `result` is `fail`; the result is
  written back to the plan file when it changed; `output` is there only for
  a check run in this call. A plan whose every step is `pass` and whose every
  step unit is fresh is deleted and reported `closed: true`. A step that is
  not `pass` carries `todo` (see Triggers). With `--no-run` nothing runs and
  no plan is written: the stored results are shown
- `check` failures are included; `next` exits 0 only with no jobs, a green
  `check`, no failed flow check or plan step `result`, and no
  `unowned_changed` (files no unit owns, outside `unowned_ok`, changed or
  added since the oldest stamp); otherwise 1

Plain `next` lists, per repo, the `reviewer` jobs of one round, folding
every stamped unit that has a maintainer (stale), tester, or documenter job
above: one job per review brief (see Pack), each with `units`, the units
its brief covers; its reasons are each of those units' jobs' reasons led
by the unit's name (`api: stale`, `api: interface changed:
src/api.rs::create_item`), and its changed files and debt are theirs. Job
`n`'s brief is `.stewards/briefs/review-<n>-<snap>.md` and its command
`stewards brief --role reviewer` (which writes the whole round). Each never-stamped unit
keeps its maintainer job (a first review). Nothing else: no architect,
tester, documenter, or validator job. A validator job for a bug the
reviewer reports is a `--roles` job.

Every job whose brief was written carries `handoff`, ``Read `<absolute
brief path>` and follow it.`` (`brief::handoff`, the one function every
command that writes a brief uses; it refuses a path `brief::check_root`
refuses).

### review

`review [--task t] [--roles] [--answer a]...` is `next` (`next --roles`)
plus recording answers, collecting results, and one `then` line. It never
starts a session and never waits for one.

- **Answer.** Each `--answer` is recorded first, as Answers (Team) says;
  any rejected answer is an error (exit 2) and records none.

- **Report.** Every `.stewards/briefs/*.result.json` in each repo, in
  name order, at most 20 per call (the rest are left for the next call,
  counted in `left`), is listed under `reported[]`: what a subagent
  reported, not something stewards verified. A file git tracks, or one
  that is not a regular file, is reported as not a result, its content
  not shown, and left in place. Any other is opened through no link and
  without waiting on a FIFO, checked to be a regular file on the open
  handle, read up to 256 KiB, and removed once the output is written; one
  over the cap or not a JSON object is reported as not a result. Each
  entry carries `file`, the role and unit its name gives (a unit only when
  the repo has it), `summary`, a review's `units[] {unit, verdict,
  stamped, check_reported}`, `findings[] {unit, file, line, cut,
  replace_with, contested?, answer?}` (the blocking ones), and `notes[]`
  (the non-blocking ones, same shape), and `other`, every other field of
  the file. A finding or note is kept only when it passes `submit`'s own
  checks (its file a plain path its unit owns; no hidden, control, or
  data-marker character; under its cap), a unit only when the repo has it
  and its verdict is `approve` or `changes`; the rest are counted in
  `invalid`. A finding's `answer`, the exact `--answer` prefix to copy, is
  never taken from the file: `review` computes it from the unit's stored
  findings, for each one not answered yet. At most 50 findings, 50 notes,
  and 50 units are shown per result (the rest counted in `not_shown`), and every
  text from a file or its name is on one line, invisible characters shown
  as `\u{XXXX}`, capped at 2,000 characters (findings at 300, as `submit`
  prints them). A result cannot make `review` pass: `done` and the exit
  code come from `next` alone.
- **Fix first.** When a reported result has a valid finding or a unit
  with verdict `changes`, nothing is listed (`next: null`) and `then` is
  `Fix the findings, then run stewards review again.` (`review --roles`
  with `--roles`, as in every `then` line), plus, when a finding has an
  answer prefix, `A finding you will not apply: run stewards review
  --answer "<its answer prefix><your reason>" instead; a reviewer sees the
  reason and accepts or contests it.`, and, when one is contested, `A
  contested finding cannot be answered again: apply it, or change the task
  with --task.` Every result of the call is reported, so findings in one
  brief of a round never hide another brief's result; notes alone do not
  hold the list back. When the findings are fixed or answered, the next
  `review` lists every still-due unit again in one round. With result files left and no
  finding: `Run stewards review again to read the N result files left.`
- **List.** Otherwise it writes and lists what `next` lists now (`next`
  with the task in force and the same `--roles`), each job with its
  handoff. A brief already on disk whose bytes are exactly the brief due
  now (same task, units, snapshot, and body; a regular file, read with a
  cap and without waiting on a FIFO) and no result is not written again:
  its job is marked `waiting`, with the same handoff. Anything else there
  is rewritten, except a link, which is refused (`unsafe_path`) as every
  link is.
- **Then.** With jobs listed: `Start one subagent per handoff below,
  giving it exactly that sentence as its whole task (you can start them
  together); when they have finished, run stewards review again.`, plus, when a job is waiting,
  that a subagent which replied without submitting did not finish and a
  new one gets the same handoff. With a `check` failure and no job: `Fix
  the check failures listed under next.check, then run stewards review
  again.` When `next` lists `unowned_changed`, its line (`These changed
  files belong to no unit and were not reviewed: <paths>. Add them to a
  unit in .stewards/units.toml, then run stewards review again.`) follows
  the `then` line above, or is the whole `then` line when there is no
  other. A failed flow check or plan step adds `Fix the failing checks
  listed under next.flow_checks and next.plans, then run stewards review
  again.` after the lines above, or alone when there is no other (before
  the unowned line). With nothing left: `done`; each open plan then adds
  `Plan <id> has steps left; see next.plans.` (`done. Plan ...`), still
  exit 0.
- **Exit** 0 only when no job is listed, no finding is left, no result
  file is left to read, no flow check or plan step has failed, and no
  `unowned_changed` file is left; 1 otherwise.
- **Task.** `--task` is stored in `.stewards/briefs/review-task.txt`
  (git-ignored with the briefs, written through `Repo::write`, capped at
  20,000 characters) and used by every later `review` without `--task`; a
  new `--task` replaces it, and `review` removes it once it reaches
  `done`. `task {text, from}` shows the task in force and where it came
  from: `given`, `stored`, or `none` (`next`'s fallback, the branch name
  and last commit subject, plus a hint to pass `--task`, since the
  reviewer checks every changed line against it).

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
5. Write `units.toml` with `purpose = ""` and `repo.toml` with
   `test_roots` and detected `commands.test`; append `.stewards/briefs/` and
   `.stewards/telemetry/` to `.gitignore`; list unrouted tracked files.
   Thresholds are constants; `units.toml` is the override.
6. Test commands, each run from the repo root: a directory's are, in this
   order, a Cargo manifest (`cargo test`, in a subdirectory `cargo test
   --manifest-path <dir>/Cargo.toml`), a Python manifest (`pytest [<dir>]`)
   or else `test*.py` files in it (`python3 -m unittest`, `python3 -m
   unittest discover -s <dir>`), a `package.json` whose `test` script is
   not npm's placeholder (`npm test [--prefix <dir>]`), a Makefile with a
   `test` target (`make [-C <dir>] test`), then `go.mod`, `pom.xml`,
   `build.gradle*`, `*.csproj`/`*.sln` (each with its own directory flag)
   and, at the root only, a Gemfile. `commands.test` is the repo root's
   own, every one found there, and nothing else. Each unit gets the first
   command each of its own `<dir>/**` paths implies (never a parent's or
   another unit's), on its test card's `test_command`, unless the root's
   commands cover that toolchain; paths implying different commands give
   none, and `init` says so for that unit. A unit with none is shown no
   command as its own. (On a mixed-language fixture, one repo-wide `cargo
   test` from a subdirectory's manifest was shown to a Python unit, a
   reviewer flagged the command it could not run, and the loop could not
   end.)

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
  excluded file through a link. An anchor is alive only when its path is a
  file in the repo's snapshot tree (tracked, modified, or untracked and not
  ignored), so `.git/` and ignored files are dead and never inlined. Native
  code is loaded only from the user's own stewards home (`grammars/<lang>/lib.*`,
  Anchors), never from a repository: a library in a clone would run code
  from it. The stewards home is `$STEWARDS_HOME` or `$HOME/.stewards` only
  when that path is absolute; otherwise there is none (no config, memory or
  grammars), so a relative path can never resolve into the repository.
  Accepted limits: loading a library runs its initializers, as the user,
  before the ABI, symbol and `tags.scm` checks, on every unfiltered `check`,
  `next` and `review`; a grammar is native C parsing untrusted repo files,
  so a memory bug in an installed grammar is reachable by a hostile repo;
  `grammars/` is trusted like shell config (no owner or mode check, links
  followed); a library is never unloaded; parsing has no time limit or file
  size cap. No unit
  ever owns `.git/**` (any path with a `.git` component, in any ASCII case),
  so it is never in a lane, a fingerprint, or a patch anchor. Card text printed on one line has its
  control characters replaced; purpose and prose in a rendered pack sit in
  fences longer than any backtick run they contain; rendered docs escape
  only the characters that could start markup, a link or an autolink in card
  text, and refuse a `docs_path` at or outside the repo root.
- **Execution.** Only commands whose argv starts with the whole argv of a
  `commands.test` entry from `repo.toml`, or for one unit's own jobs (its
  scenarios, bug repros, task checks) of that unit's test card
  `test_command`, ever run (`pytest` does not allow `pytest-x`): repro,
  plan step checks, flow checks, scenario commands. `commands.lint` is
  never run. No answer has a
  field that sets `test_command`, so only `init` or a person writes it, as
  for `repo.toml`. A validator brief runs the bug's repro through the same
  allowlist when it is written (`validate --report`, `next --roles`, `brief`).
  The words after an allowed prefix are not checked: a repro
  or check can pass flags that make the allowed program run any other
  program, as the user, with the inherited environment (tokens included),
  and with no sandbox. The allowlist limits which program starts, not what
  it does. CI runs `check`, which runs no repo command (it does load the user's
  grammar libraries when the stewards home has any). One module (`run`) owns the
  allowlist and the execution: argv split, no shell, from the repo root, a
  bare program name found only through the absolute entries of `PATH`,
  stdin null, inheriting env, in its own process group, which is killed
  whole at the matching entry's `timeout_seconds` (the longest matching
  entry; else `commands.timeout_seconds`) and after a normal exit, so
  nothing the command left behind keeps running; a process that leaves the
  group is not killed. The check itself refuses control, invisible and
  shell characters. A timeout or a program that cannot start is
  a failed result, not an error; the result keeps the last 4,000
  characters of stdout and stderr with control characters replaced.
  Nothing from an answer file, card, issue, or PR runs otherwise. A plan
  step's check is checked against its unit's allowlist when `submit` stores
  it and again when `next` runs it (a hand-edited plan or flow `check` the
  allowlist refuses is a failed result); a flow check gets `commands.test`
  only. The output is the command's text, attacker-controlled like card
  text: `next` and `review` carry it as a JSON string, `--human` prints it
  in a fence no backtick run inside it can close, and any brief that ever
  quotes it must fence it as data. A step's `todo` never puts the step's
  `change` on its command line.
- **Writes.** The tool writes only `.stewards/**`, `docs/units/**`, the
  marked block in `AGENTS.md`, two appended `.gitignore` lines, and CI
  files when asked by flag (`init --ci`, which also appends the file's path to `unowned_ok` in `repo.toml` with `toml_edit`, layout kept, unless a unit owns it). Never source or
  tests. Deletes only inside `.stewards/briefs` and `.stewards/telemetry`,
  and closed plans. `submit` writes a new plan; `next` (and so `review`)
  writes each step result that changed and deletes a closed plan, unless
  `--no-run`. Snapshots add unreachable objects to `.git/objects`
  (git prunes them) and never touch the index, HEAD, or any ref.
  Every write, append, and delete inside a repo goes through `Repo::write`,
  `append`, or `remove`: the path must be plain names, no existing
  component may be a symlink, and a write renames a fresh temporary file
  over the target, so a committed link is refused, never followed
  (`unsafe_path`). `load_repo` refuses the same way to read `.stewards/`,
  `.stewards/units`, or any file it loads through a link.
- **Secrets and external text.** `pack` names but never inlines files
  matching `[pack] exclude`. `review --pr` and `next --pr` never read the PR
  title or body, and of the comments only each one's id, body, and author login,
  to find their own (author is the logged-in user, body starts with the marker);
  nothing of any comment is kept or printed. The text they post goes inside a
  code fence longer than any backtick run in it, each line indented by one
  space so that no line starts a GitLab quick action (GitLab runs `/merge` and
  the like from API notes, fenced or not), invisible characters shown as
  `\u{XXXX}`, and is cut to stay under the comment size cap. The repo and
  host come from `origin`, not from gh/glab's default remote. Under
  `GITHUB_TOKEN`, `gh api user` is refused and the author is
  `github-actions[bot]`, shared with any other workflow using that token. A
  GitLab token in a CI variable is readable by every branch pipeline that can
  see the variable. A posting failure is reported in `pr_comment_error` and
  does not change the exit code. Telemetry stores
  sizes and counts. Tracker export sends only fields the tool composed:
  the title (the gap title, one line, capped at 120 characters) and a body
  of `Unit <unit>, severity <severity>.`, the anchor, repro and note (each
  in a code fence longer than any backtick run in it, every line indented
  one space, invisible characters shown; for Jira plain text in
  `{noformat}` blocks with every `{noformat...}` macro in the text
  removed), and `Filed by stewards from .stewards/units/<unit>.toml.`
  Nothing else from the repo is sent. Issues go to the `[tracker]` project
  the repo names, as the logged-in user, with a title and text taken from
  the card: check `[tracker]` before exporting from a repo you did not
  write. No token, header, or request body reaches output, an error,
  telemetry, or a file; a tracker error has the token and the whole
  `Authorization` value blanked out. An issue can be filed twice: after a
  crash, a timeout, or a dropped connection after the server created it;
  after a 2xx reply with no key or URL; or after a failed card write (the
  error names the URL to record by hand). Jira and Linear calls ignore
  `HTTPS_PROXY` (`gh` and `glab` honour it) and trust the bundled web
  roots, not the system store, so a Jira behind a required proxy or a
  private CA fails closed. Tracker replies are untrusted: error text is
  shown as one escaped line of at most 300 characters, and the returned
  URL must be ASCII and under the tracker's own address.
  No secrets in any stewards file; tracker auth from CLIs and env.
- **Sessions and results.** stewards never starts a session or a
  subagent and holds no harness's command line; the agent starts each
  subagent with the handoff, in its harness's own way and permissions.
  Back from a round it reads only `<job>.result.json` files, which its own
  `submit` writes through `Repo::write`: one git tracks is not read; any
  other is opened through no link and without blocking on a FIFO, must be
  a regular file of at most 256 KiB, has its findings re-checked as
  `submit` checks them, its texts printed on one line, invisible
  characters escaped, and capped, and it is removed through `Repo::remove`
  only after the output is written. A result is shown as reported, never
  as verified, and cannot make `review` pass. A result planted by a
  hostile subagent cannot be told apart from one `submit` wrote without a
  key kept outside the repository; that is a recorded limit. A waiting
  brief is kept only when its bytes equal the brief due now.
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
  "fix"}}` on stdout, exit 2. Exit 1 means work is due: `check` failures,
  jobs from `next`, or jobs listed, findings reported, or result files left
  by `review`.
- Every command that writes a brief (`next`, `brief`, `init --draft`,
  `review`) prints, per brief, `handoff`: exactly ``Read `<absolute brief
  path>` and follow it.``, the same for every role. The agent passes it
  whole and unchanged as a subagent's task; the brief tells the subagent
  that its reply is exactly what `submit` printed.
  `--human` prints `error:` and `fix:` lines.
- A failed `submit` returns `errors[] {field, problem, example}`; the brief
  says fix and rerun, and after the third failure reply to the caller with
  the errors verbatim. A rejection for length alone (`answer_too_long`)
  does not count toward the three: each of its entries says how far over
  the cap the field is and gives a target, 80% of the cap (`302
  characters; 2 over the cap of 300; cut to 240 or fewer`, the example `at most
  240 characters`), and the brief says to cut to that target, not to the
  cap. (A reviewer once spent all three tries on one field at 308, 303,
  and 302 characters against a 300 cap, and the review was lost.)
- Git is required by every command.

Error codes (stable; additions are minor versions, meanings never change):

| code | when | fix text |
|---|---|---|
| `not_a_git_repo` | no `.git` above cwd | `git init` or run inside a repository |
| `no_stewards_dir` | no `.stewards/` above cwd | `stewards init` |
| `schema_too_new` | `repo.toml` schema > tool's | upgrade stewards (`cargo install stewards`) |
| `invalid_toml` | parse failure, with file and line | fix the file at `<path>:<line>` |
| `unknown_unit` | unit name not in `units.toml` | one of the listed units |
| `unknown_role` | role not in the seven | one of developer, maintainer, tester, documenter, validator, architect, reviewer |
| `answer_parse_failed` | `submit` can't parse the file | the field errors listed; rerun submit |
| `answer_dead_anchor` | a patch names an anchor that doesn't resolve | correct the anchor or drop it |
| `answer_too_long` | field over cap | cut `<field>` to the target the error states (80% of its cap); this rejection does not count toward the three tries |
| `command_not_allowed` | command outside the allowlist | prefix with one of `commands.test` or the unit's test command, or add it to `repo.toml` |
| `command_timeout` | allowlisted command exceeded its timeout | raise `timeout_seconds` or fix the test |
| `workspace_repo_missing` | `stewards.toml` lists a path that doesn't exist | fix `repos` in `<path>` |
| `baml_runtime_unavailable` | `--direct` and the native lib is not found | set `BAML_LIBRARY_PATH` to the library, or set `STEWARDS_BAML_DOWNLOAD=1` to let stewards download it |
| `no_api_key` | `--direct` and the configured provider's key is unset or blank | export the variable named (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, or `GOOGLE_API_KEY`) |
| `tracker_auth` | `gh`/`glab` not logged in, Jira or Linear env missing, `JIRA_URL` not https or with user info, or the tracker refused the credentials (HTTP 401/403) | `gh auth login` / `glab auth login` / set `JIRA_URL`, `JIRA_TOKEN` (and `JIRA_EMAIL` for Jira Cloud) / set `LINEAR_API_KEY` |
| `tracker_failed` | `validate --export`: any other tracker failure; the message gives the kind, the HTTP status, the tracker's error on one line (at most 300 characters), and how many issues this call already filed and recorded | check [tracker] project and the tracker, then run stewards validate --export again; filed issues are recorded on the cards |
| `harness_unknown` | unsupported harness name | one of the listed harnesses |
| `usage` | bad arguments, a bad `-C`, or a role given with or without a unit wrongly | run `stewards <command> --help` |
| `not_in_repo` | the current directory is in no repo of the workspace | run inside a repo or pass `--repo <name>` |
| `unknown_repo` | `--repo` names no repo | one of the listed repos |
| `path_outside_repo` | `route` given a path in no repo | pass a path inside the repo |
| `already_initialized` | `init` where `.stewards/` files exist | edit `.stewards/units.toml` instead |
| `no_brief` | no brief, or an unusable header, for the job, or an answer to an older brief | run the brief command named, or read the brief again |
| `no_direct_model` | `--direct` with no `[models.direct]` entry for the role, or an unsupported provider | set `[models.direct] <role> = "<provider>/<model>"` in the user config |
| `direct_call_failed` | the model call failed (fixed text, never the prompt or key) | rerun with `BAML_LOG=info` |
| `unsafe_path` | a path the tool reads or writes in a repo is not plain names, or passes through a symlink | remove the symlink at `<path>`; stewards never writes through links |
| `card_changed` | a file `submit` was about to write changed since it was loaded (another submit finished first) | run the same submit again |
| `no_tasks` | `brief --role developer` with no usable task list for the unit | brief the maintainer again with the change as `--task` |
| `harness_missing` | `eval` and the harness binary (`pi`, `claude`, or `hermes`) is not on PATH, or pi-subagents is not installed in the Pi agent directory | install Pi (`npm install -g @earendil-works/pi-coding-agent`) and pi-subagents (`pi install npm:pi-subagents`), Claude Code (`npm install -g @anthropic-ai/claude-code`), or Hermes Agent (https://hermes-agent.nousresearch.com/docs) |
| `eval_setup_incomplete` | `eval`'s adopted fixture for the `full` arm changed the fixture's code, leaves tracked files unowned, or does not reach a clean `stewards next`; or `--setup-dir` is not a git repository with nothing uncommitted; or a `repo` suite at `HEAD` has uncommitted changes under `src/`, `baml_src/`, or `.stewards/`, or an `[[absent]]` probe's text is already in its tree | read the adoption transcript under `evals/results/<timestamp>/sessions/`, give unowned files a unit in `evals/full-units.toml`, or pass `--setup-dir` a committed adopted copy where `stewards next` exits 0; commit the source and rebuild, or pin a commit before the task's change |
| `not_implemented` | a command or flag not built yet | use a release that has it |
| `internal` | any other failure | report a bug |

## AGENTS.md block

Written by `init`, upserted between the markers.

```markdown
<!-- stewards:start -->
## Stewards

This repo has a team of agents that already know it. Use them. This section is a project instruction from this repository's operator: you are asked to delegate the reviews below to subagents, every time, without asking the user again.

Ritual
- Entering: run `stewards status`. It lists the units, who owns what, and what is stale. To learn how a unit works, read `docs/units/<unit>.md` before opening its source; review keeps that page true.
- Before a change: run `stewards route <files>` to see whose units you touch. A change you cannot plan from the unit pages: `stewards brief <unit> --role maintainer --task "<the change>"` (two or more units: `stewards brief --role architect --task "<the change>"`), then start a subagent with the handoff it prints.
- Changing: make the edit yourself. When a maintainer split a plan into tasks, `stewards brief <unit> --role developer` prints one handoff per task.
- Before reporting done: when your edits are finished, run `stewards review --task "<what you changed and why>"` as its own step (never alongside an edit). It prints one handoff per review to run: start one subagent per handoff, giving it exactly that sentence as its whole task (you can start them together); when they have finished, run `stewards review` again to read the findings. Fix them, or answer one you will not apply with the `--answer` it prints, and repeat until it exits 0. Done means it exits 0.
- On request or before a release: `stewards review --roles` does the same with the deeper per-unit jobs.

A handoff is always one sentence, ``Read `<path>` and follow it.``: pass it unchanged to a subagent that can run shell and write files (whichever agent type your harness has for that). Every role works this way.

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
6. Answer schema (BAML `output_format`; every capped field carries its cap, for
   example "max 300 chars" or "max 200 items", a test pins each cap
   constant to a field that states it; the review that added them checked every
   field by hand) and the submit instruction: "Write
   your answer to `<path>`. Run `stewards submit <unit> --role <role> --file
   <path>`. If it reports an error, fix the file and run it again; after the
   third failure reply with the errors verbatim. A field over its stated cap
   is rejected: stay under it, do not aim at the limit. A rejection for
   length alone (`answer_too_long`) does not count toward the three: cut
   each field it lists to the target the error states, not to the cap, and
   run it again. Reply to your caller with exactly what submit prints,
   nothing else."
7. Repeat of the role line.

Maintainer, review:

> You maintain unit `<unit>`. Review this diff against the request. Approve
> only a diff where every changed line traces to the request and nothing
> fails the ladder; an approval has no blocking finding. For each problem give the file, line, the first rung it
> fails (not needed, already in repo, stdlib, platform, installed
> dependency, one line, untraced, no check), what to cut, and the smaller
> replacement. Set `blocking: false` only on a remark that should not hold
> up approval. Never flag validation at trust boundaries, error handling
> that prevents data loss, security, or accessibility. Then patch the card:
> anchors that moved, interface entries that changed, gaps opened or closed.
> `card_patch: null` when nothing changed. Set `tester_needed` when an
> interface's behavior changed on purpose. With no diff in the pack this is
> a first review of existing code: draft the card from source, record each
> problem as a gap in the card patch, not a finding, and approve.

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
> bug: give a title, the exact repro command (it must start with the unit's test
> command or one of the repo's), and the evidence. You may write tests under the
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

Reviewer:

> You review one change across the units below. For each unit: read its
> hunks against the request and check cleanliness and scope on the ladder:
> approve only when every changed line traces to the request and nothing
> fails a rung; an approval has no blocking finding. For each problem give the
> file (a file of that unit), line, the first rung it fails, what to cut,
> and the smaller replacement. Set `blocking: false` only on a remark that
> should not hold up approval; the agent sees it as a note. Each answered
> finding in a unit's pack carries the agent's reason for not applying it:
> accept the reason and do not raise the finding again, or contest it once
> in `contested`, saying why the reason does not hold; a contested finding
> returns to the agent as blocking, so that unit's verdict is `changes`.
> Never raise a finding listed as accepted. Report every problem you can find in this
> pass: a later round should only confirm the fixes. Never flag validation at trust boundaries,
> error handling that prevents data loss, security, or accessibility. Run
> the unit's test command, or with none the repo's test commands, and
> report it in `check`; with no test command in the pack, `check` is null.
> A unit whose tests fail is not approved. Keep the card true. Update
> scenarios only if the contract changed. Write the doc when the unit has
> none; rewrite it only if what a section says is no longer true. Report a
> defect you will not fix as a bug. A bug's `repro` and a scenario's
> `command` must start with the unit's test command or one of the repo's. Never edit
> source. Open more only by anchor. One answer, one `submit`.

The reviewer's brief adds one line after the role text naming the units
under review, each to appear exactly once in `units` and no other, and one
before the submit instructions giving its snapshot, which the answer
copies.

Answer shapes: maintainer review `verdict approve|changes`, `findings[]
{file, line, rung, cut, replace_with, blocking}` (`blocking` required; the
schema text says `false` is for a remark that should not hold up
approval, such as wording, a redundant local, formatting, and that
anything about correctness, missing tests for changed behaviour, scope or
security is `true`), `card_patch {interface[], anchors[],
gaps[]} | null`, `tester_needed` (true makes `submit` write the tester brief
and print its path); maintainer explain `explanation`, `tasks[] {title, change, files[], check?}`
(empty for a pure question), `card_patch | null`; developer
`check {command, pass, output} | null`, `summary`, `blocked_on | null`; tester `scenario_patch[]`, `results[]
{scenario, pass, output}`, `bugs[] {title, repro, evidence, interface?}`,
`tests_written[]`; documenter `doc {overview, how_it_works, how_to_change}`;
validator `real, severity, reason, evidence[]`; architect `plan | null,
flows[], decisions[], unit_proposals[], explanation`; reviewer `snapshot`,
`units[] {unit, verdict approve|changes, findings[], card_patch | null,
check {command, pass, output} | null, scenario_patch[] | null, bugs[] |
null, doc | null, contested[] {file, line, why} | null}` (`contested` names
answered findings of the unit by file and line, each at most once; capped
as every list). All carry `pulled_anchors[]` and `notes`.

A reviewer's answer copies the brief's `snapshot`: one that differs is
`no_brief` (it answers an older review brief, which `next` replaced), and
the fix is to read the brief again. `submit` also refuses (`no_brief`) a
review brief whose header lists a unit that is unknown, never stamped, or
listed twice. The answer names every unit of its brief exactly once and no
other, and each unit's fields pass the checks and caps the maintainer's,
tester's, and documenter's do; a unit approved with a failing `check` is
rejected. `check` is required when the brief shows a test command for the
unit (its own or the repo's `commands.test`) and must be null when it shows
none, so a repo with no test command can still be approved. `doc` is
required when the unit, with its card patch applied, has an interface and
no docs (otherwise `next` would list it forever), and otherwise given only
when what a section says is no longer true. `submit` validates the whole answer first and writes nothing on
any error. Then per unit, through the same code paths: the card patch;
bugs as the tester's become bug gaps with no severity (a probable
duplicate is reported, not added); the doc as the
documenter's; scenarios as the tester's, into the test card; the page
rendered again for every unit whose card or test card was written and
which has docs; on `approve` the stamp at the brief's snapshot with both role
marks set there, and the unit's task and findings lists removed; on
`changes` no stamp, the blocking and contested findings as the unit's fix
task, and its findings list rewritten (see Answers). The lost-update check
covers every file it writes, and the brief is retired, as for any role.
The summary names each unit's verdict, card changes, notes, accepted
answers, and whether it was stamped; `findings[] {unit, file, line, cut,
replace_with, contested?, answer?}` (each text capped at 300 characters)
lists what to fix, and `notes[]` what was remarked without blocking.

Details the shapes leave open: enum values are lowercase snake case
(`approve|changes`, `critical|medium|low`, `gap|future|bug`, the rungs as
`not_needed` … `no_check`). A nullable field may be absent or null; every
other field and every list is required; unknown fields, duplicate fields,
and any text around the JSON are errors. `card_patch` has six optional
lists. `interface`, `anchors`, and `gaps` upsert by key (interface `id`,
anchor path and name, gap `title`): an entry replaces the one with its key
or is appended, and every entry the patch does not name is kept, in order.
Only `remove_interface` (ids), `remove_anchors` (`path::name`), and
`remove_gaps` (titles) remove, which is how a patch closes a gap; a removal
must name a key the card has and the patch does not also upsert, and a
removed anchor need not resolve. A
patch gap is `{kind, title, anchor?, note, repro?}`; severity and issue are
the tool's. `scenario_patch` upserts scenarios by id. A plan is `{title,
steps[] {unit, change, interfaces[], check}}`, stored as
`plans/<id>.toml` (see Schemas). A unit proposal is `{name,
purpose, paths[]}`. A validation error names the field path, or
`(document)`, and carries an example value.

`guide` prints the long form: the ritual, the handoff paragraph, the
roles, the editing rules, the hygiene rules, adoption (`stewards init`,
correct `units.toml`, `stewards init --draft` and one subagent per handoff
it prints, then `stewards review --roles` until it exits 0), and the
delegation paragraph, and the sentence `init` also prints (the block is in
`AGENTS.md`; a harness that does not read it needs one line in its own
instruction file pointing at it, listed as `instruction_files`);
`--harness <name>` names a harness instead of the
one detected from env, `--all` prints every variant. Every harness, known
or not (an unknown one is the normal case, not an error), gets the same
generic paragraph: start a fresh-context subagent that
can run shell and write files (whichever agent type your harness has for that), give it the handoff stewards printed
(``Read `<absolute brief path>` and follow it.``) unchanged as its whole
task, and wait for its reply: what `submit` printed. Nothing is added
for a harness in the `harness.rs` table.

## Docs templates

`docs/units/<unit>.md`: a banner ("generated by stewards from
`.stewards/units/<unit>.toml`; change prose through the documenter"), H1
unit name with purpose, Overview, How it works, How to change it, Interface
table (id, contract, anchor as a relative link with line), Scenarios table,
Flows this unit is part of, Gaps, last reviewed commit. `docs/units/README.md`:
one line per unit with a link, flows as ordered unit chains, decisions list.

## Evals

- **Suites:** `stewards eval --suite <name>` (default `fixture`) reads
  `evals/suites/<name>.toml`: where each arm's copy comes from, the task
  prompts' directory, the per-session wall-clock limit
  (`session_minutes`), and per task whether it is `read_only`, which task
  runs first as its `setup`, and which `bare` task its files opened are
  compared with (`files_opened_vs_bare`). `--task` takes the suite's task
  numbers; any other, or an unknown suite, is a `usage` error. The
  `fixture` suite is the small fixture below. The `self` suite exists
  because one pass on the fixture showed no delegation in `full` and more
  tokens than `bare`: at about 400 lines, reading is too cheap for
  delegation to pay. `self` is the stewards repository itself (about 8,500
  lines of Rust, twelve adopted units with cards and docs) at its manifest's
  `commit`, `HEAD`: the commit the checkout holding `evals/` is on when
  the eval starts, so the source the sessions read is the source the
  binary was built from (an agent once read a fixed pin's old source,
  where `review` was a stub taking `--pr`, and concluded `--task` did not
  exist). The runner refuses to start (`eval_setup_incomplete`) while that
  checkout has uncommitted changes under `src/`, `baml_src/`, or
  `.stewards/`, since the binary and the tree could then disagree, and
  when an edit task's change is already in the tree: the manifest's
  `[[absent]]` probes name, per task, a literal text the task would add
  and the file it must not yet occur in (task 1 `Debt {` and `Debt(` in
  `src/main.rs`, task 3 `owner` in `src/model.rs`). A literal sha in
  `commit` is still accepted, to rerun a past eval. Results from different
  commits are therefore not comparable run to run: the resolved sha the
  summary records is what identifies a run. The tree is cloned from that
  checkout (no network) without its remote. Its `full`
  arm is that tree with this binary's AGENTS.md block upserted (as `init`
  does), `AGENTS.md` and the driver's context file added to `unowned_ok`
  (both newer than every stamp, so `next` would ask for a unit for them),
  and committed, and no adoption session; the runner checks
  `stewards next --no-brief --no-run` exits 0 there before any run (else
  `eval_setup_incomplete`). `bare` is the tree without `.stewards/`,
  `docs/units/`, and `AGENTS.md`; `block-only` adds an `AGENTS.md` holding
  only the block. Every arm's history is one root commit, `initial`, on
  `main`, with no other ref and an expired reflog, so no session can
  recover `.stewards/`, `docs/units/`, or `AGENTS.md` (or later commits of
  the checkout) from history: `bare` and `block-only` are new repositories
  (`git init`), with no old object at all. `full` is an orphan commit in
  the clone and keeps the clone's object store, unpruned: a unit's stamp
  and its tester and documenter marks are tree hashes held only as
  unreachable objects there, and `next`, the review pack, and the
  line-level triggers diff from them; without them every owned file would
  count as changed and the review brief would carry whole files instead
  of hunks. A local clone carries those objects; the runner checks each
  one exists (`git cat-file -e`) and stops with `eval_setup_incomplete`
  naming the unit otherwise. The old commits stay reachable to `git fsck`
  and `git cat-file` in `full`, so those join the flagged history commands
  (see Runner). The summary records the commit git resolved. In this
  suite the `stewards` on `full`'s `PATH` is the eval's own binary, not
  the tree's code: a session that changes stewards itself runs
  the older or newer binary, not its edit. Its manifest's `build` (`cargo
  build --all-targets && cargo clippy --all-targets`, so every arm starts
  with a warm lint cache too) runs once per invocation in the clone with
  the user's environment; its `target/` is then moved out of the clone and
  copied (`cp -a --reflink=auto`, sharing blocks where the filesystem can)
  into every run copy of every arm, so no arm pays a cold build and no
  run's in-place writes (cargo rewrites fingerprints) reach the cache.
  Only `full` copies are cleaned (`git clean -fdX`, before the cache is
  copied in); `bare` and `block-only` copies are not, and need not be: the
  clone holds no ignored file once `target/` is moved out. The build's
  seconds are setup, reported outside every ratio. Its four tasks are under
  `evals/suites/self/tasks/`: (1) a `--unit` filter for `debt`; (2)
  explain how an approving review is stamped (read-only); (3) an optional
  unit `owner` end to end; (4) a second-session question about `owner`
  after task 3 (read-only, compared with `bare` task 2). A session has 60
  minutes. The work directory is `<tmp>`, so run it with `TMPDIR` on a disk
  with room for a target directory per run (a reflink copy shares blocks
  only on the cache's filesystem), since a small RAM-backed `/tmp` fills.
  The summary names the suite, its `commit` (`HEAD` or a sha), and the
  full sha it resolved to.
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
- **Runner:** `stewards eval --harness <pi|claude-code|hermes>` (default
  `pi`) drives `pi --mode json`, `claude -p --output-format stream-json
  --verbose`, or `hermes chat --oneshot --format stream-json` (all three
  built; a Codex driver, `codex exec --json`, is not in 0.1.0) behind one
  `Driver` trait (start a session on a prompt in a directory with a model
  and thinking level and return its transcript; parse it). The drivers
  under `src/eval/` are the only place stewards knows a harness's command
  line; a test keeps their launch flags (`--dangerously`, `--mode json`,
  `exec --json`, `--oneshot`) out of the rest of `src/`. Suites, arms,
  tasks, prompts, graders, the gate, and the summary are the same for all
  and never branch on the harness: each driver maps its transcript into the
  same `Run` and the same harness-neutral actions (a shell command, a
  subagent launch with each child's task). The runner parses each transcript
  to `Run {main_tokens? {input, output, cache_read, cache_write},
  total_tokens? {same}, cache_warming {same}, peak_main_context?,
  tool_calls[], files_opened[], files_opened_tree[], shell_commands[],
  ritual_steps{status, route, next, review, handoff, handoff_exact,
  reviewer_submit, check_green}, cost_usd?, harness_reported_cost_usd?, duration_s, models[], thinking_levels[],
  sessions_without_thinking[], subagent_sessions, uncounted_sessions[], reviews[],
  git_history_commands[], stop_reason, failed_children[],
  truncated_lines}`.
  Main tokens = the parent session only; total = parent + all children
  (the role subagents are children of the session, so the tree holds every
  token); the token ratios compare the sum of the four fields. For Pi the fields
  are summed over assistant-message usage plus `compaction` and
  `branch_summary` records (matched by record type), each record counted
  once across the tree by `(id, timestamp)` because a fork starts as a copy
  of its parent; `usage` records (cache refreshes, which do not enter
  context) go to `cache_warming` instead, their cost still in `cost_usd`.
  `cost_usd` is Pi's own `cost.total` (its price table, not a bill), `null`
  when the transcript has none. For Claude Code the transcript is the
  session file it persists (`$CLAUDE_CONFIG_DIR/projects/<cwd
  slug>/<session id>.jsonl`, copied into the run's sessions beside the
  stream as `stdout.log`): child records carry `isSidechain: true` and each
  child is the chain of them from one with no sidechain parent. A response
  is persisted once per content block with its usage so far: its input and
  cache counts are final, its output count can fall short (the smoke run's
  records summed 1,980 output tokens against the stream's 2,185), so main
  tokens are the stream's final `result` line's `usage` (the parent's
  alone) and total tokens the sum of its `modelUsage` (every request of
  the process, children included): both exact. A session that ended
  without that line (killed) has `main_tokens` and `total_tokens` `null`,
  never the records' short sum; `peak_main_context` still comes from the
  main records. On every driver a timed-out run's counts are partial, so
  the runner sets its `main_tokens` and `total_tokens` to `null` (its
  `cost_usd` stays): its task's token medians are then `null`, and the
  adoption ratio leaves it out, on every driver alike. Claude Code's `models`
  are the records' models plus every `modelUsage` key, which also names
  side requests (titles, summaries) made outside the parent and its
  children, so one on another model is flagged as any model mismatch.
  Claude Code's `total_cost_usd` prices a model id it does not know at
  another model's rates (1.0.128 prices `claude-haiku-5-5` at Sonnet's), so
  its runs record `cost_usd: null` and keep that figure as
  `harness_reported_cost_usd`; the summary says so, its cost ratios are
  `null`, and `--max-cost` with this harness is a `usage` error, since no
  cost can bound the spend. Its records carry no stop reason: a main
  session failed when the stream's `result` line says `is_error` or its last
  response is an API error (`isApiErrorMessage`), a child when its last
  response is one. Files opened are its `Read` tool's `file_path`, its
  `Grep` tool's `path` (a directory is dropped, as below), and the shell
  reads below; a relative path is resolved against the `cwd` of
  the record that made the call, since a `cd` in one `Bash` call carries
  over to the next, and every path is then taken relative to the main
  session's starting directory (a `..` steps back up). For Hermes the
  transcript is `hermes sessions export --format jsonl` of the session's
  own home, run after the session (`transcript.jsonl`, beside the stream as
  `stdout.log` and the home's `logs/agent.log` with its rotations
  `agent.log.<n>`): one row per session. The task's session is the
  first-started row with no `parent_session_id`; another such row (a
  `hermes` a session started itself with the inherited `HERMES_HOME`) and
  the rows under it are left out of every count and named in the run's
  `uncounted_sessions`, flagged in the summary, not failed (on Pi and
  Claude Code a nested session is simply not counted). Under the task's
  session a row is a child only when its `source` is `subagent`; a row
  with the parent's own `source` is the same session continued after
  Hermes compressed its context (a new row under the old one), so its
  tokens, tool calls, and API calls belong to that session (main or
  child), and a call it kept from the row before counts once. Hermes keeps
  each row's counts itself
  (`input_tokens` uncached, `output_tokens` with reasoning inside,
  `cache_read_tokens`, `cache_write_tokens`; per-call deltas): main tokens are the main
  session's rows' (equal to the stream's `result.tokens`, whose `input` includes
  the cache reads), total tokens add every child's. It persists no
  per-response usage, so its `peak_main_context` is the largest prompt
  (`in=`, cache included) of the main session's API calls in `agent.log*`,
  `null` without such a line. Its `estimated_cost_usd` prices
  `deepseek-flash` from Hermes's own table at DeepSeek's off-peak rates in
  every hour (its source notes peak hours, Mon-Fri 01-04 and 06-10 UTC,
  cost twice that), so its runs record `cost_usd: null`, keep the summed
  figure as `harness_reported_cost_usd`, and `--max-cost` with this
  harness is a `usage` error, as for Claude Code. Its `--model` is
  `provider/model` (`deepseek/deepseek-flash`), split at the first `/` into
  `--provider` and `-m` (without one, a `usage` error); each session's
  model is recorded as `billing_provider/model`. A `delegate_task` call is
  a subagent launch for each of its children that ran, as its result's
  `results` report them: a task they mark refused is left out, a result
  without `results` (Hermes refused the whole call with an `error`: a
  spent one-shot budget, the depth limit, a bad argument) launched
  nothing, and a call with no result (the session ended inside it) counts
  all its tasks. Its tasks are `tasks` (an array, or the JSON
  string of one, which Hermes accepts for small models), else the
  top-level `goal`, `context`, and `output_schema`, which Hermes reads only
  when `tasks` is absent or empty. Each child's task is the whole text it
  was given: its `goal` (its first message), plus, when there is any, the
  context Hermes puts in its system prompt (its `context`, with the output
  contract Hermes appends for an `output_schema`). Hermes's tool
  description asks the model to fill `context`, so a weak model may add
  text there that another harness's would not; the summary's setup note
  says so. Its shell commands are `terminal` calls; commands an
  `execute_code` script runs through its own `terminal()` are not seen (not
  in `shell_commands`, files opened, or the ritual steps), and the setup
  note says so. Its main session failed when the stream's
  `result` line has a nonzero `exit_code` or an `error`, else by its last
  response's `finish_reason`; a child failed when the `delegate_task` call
  that ran it reports a status other than `completed`, or its own last
  response stopped with `error`, `aborted`, or `length`. Its `duration_s`
  includes the start of the gateway daemon (8 to 10 s before the turn
  began in the smoke runs, of 47 s and 65 s), so it is longer than the same turn on another
  harness; the setup note says so. Hermes's auxiliary
  requests (titles, compression) are not in the export; on the
  verification machine they failed for want of a Nous login and spent
  nothing.
  `peak_main_context` is the largest input + cache read + cache write of
  any one main-session response.
  `files_opened` (main session) and `files_opened_tree` (every session)
  count distinct paths relative to each session's cwd, by one rule on every
  driver: a file counts when its content was read or searched, through a
  read tool (Pi's `read`, Claude Code's `Read`, Hermes's `read_file`), a
  content search given its path (Claude Code's `Grep`, Hermes's
  `search_files`), or a simply recognised shell read (a Hermes
  `terminal` call's reads start in its `workdir` when it gives one); a
  listing never counts, whatever produced it (`ls`, `find`, Claude Code's
  `Glob`, Hermes's `search_files` with `target: files` or `find`), nor do
  Hermes's `execute_code` scripts. The shell reads are `cat`, `head`, `tail`, `less`, `sed
  -n`, `grep`/`rg` with a path argument (separators inside quotes do not
  split, heredoc bodies (`<<WORD`, `<<'WORD'`, `<<-WORD` up to the line
  equal to `WORD`) are skipped, a relative path after a leading `cd <dir>`
  segment is taken under `<dir>`, and words with `$`, words starting with
  `!`, and option values such as `--glob x` or `-A 3` are not paths; see
  `shell_reads`). After the parse the runner keeps only the paths that are
  regular files in the run's directory after the session: a glob in the
  last path component is expanded against that directory, and
  directories and paths that do not exist are not counted. The ritual
  steps recorded per run are `status`, `route`, `next`, `review` (a shell
  command, in the main session or any child, whose argv starts with
  `stewards <cmd>`), `handoff` (a subagent launch with a handoff, ``Read
  `<absolute path of a brief under .stewards/briefs/>` and follow it.``, in
  one of its argument strings, the file named as `brief::rel_path` names a
  brief, `-<snap>` included, so a name an older stewards wrote does not
  count), `handoff_exact` (every such launch gives
  each child exactly the handoff as its task: the whole task text the
  child was given equals the handoff, nothing added; that text is each
  `task` string of a pi-subagents call, the `prompt` of a Claude Code
  `Task` call, and a Hermes child's `goal` plus the context Hermes adds to
  its system prompt, as above), `reviewer_submit` (`stewards submit ...
  --role reviewer` in a child session), and `check_green`, which comes
  from the runner: after each `full` (and `full-no-developers`) session it
  runs `stewards next --no-brief --no-run` in the run directory (since
  `next` runs flow and step checks, and the session's own `review` already
  ran them, the gate measures review coverage, not test results) and
  records the exit code and job count; green is exit 0. The runner also records
  whether the session `changed` the run directory: any path that differs
  from the session's starting commit (`git diff --name-only <start>`, so
  committed changes count) or is untracked and not ignored, outside
  `.stewards/` and `docs/units/`. The transcript steps are best-effort;
  the summary lists the known blind spots (`timeout`/`env` prefixes,
  `if`, `bash -c`, backticks). Each run records its count of subagent
  sessions. Each `full` run records its review outcome, report-only, per
  unit of each successful reviewer `submit` (from the JSON that submit
  printed in the child's transcript): the round, the verdict, the number
  of findings (blocking), of notes (non-blocking), and of contested
  findings, the bug gaps the unit's card gained during the run (cards
  read before and after), and whether the submitting child ran the
  unit's test command (its test card's `test_command`, else one of the
  repo's `commands.test`, as a substring of one of the child's shell
  commands); and per run the number of findings answered (`--answer`
  arguments of `stewards review` commands anywhere in the session tree).
  Any session whose shell commands run `git log`, `show`,
  `checkout`, `reflog`, `stash`, `fsck`, or `cat-file` is flagged, not
  failed, and listed in the summary. A session whose last main response,
  or any child's last response, stopped with `error`, `aborted`, or
  `length` is a failed run whatever the harness's exit code; the stop
  reason and the failed child are recorded. A Pi session directory with more
  than one top-level session file fails the parse (a second top-level
  Hermes session is left uncounted and flagged, as above); an unparseable line is
  tolerated only as the last line of a file from a failed or timed-out
  session (counted in `truncated_lines`). A run whose transcript cannot be
  parsed has an unknown cost: after one, `--max-cost` stops the eval, as it
  can no longer bound the spend, and the summary says so. Every arm gets
  the same model (`--model`, else the harness default) and thinking level
  (`--thinking`, default `medium`), the task file verbatim as the prompt,
  and the same parsing; every model used in a session tree is recorded and
  a run where any differs from the requested one (or the main session's) is
  flagged, not failed (the main agent may pass `model` per subagent).
  Claude Code 1.0.128 has no thinking level, only a token budget from
  `MAX_THINKING_TOKENS` (the one variable that sets one; the driver
  removes it from the session's environment) or from words in a user
  message (`think` alone gives 4,000 tokens, `think hard` 10,000,
  `ultrathink` 31,999). So its runs request `thinking: off`, `--thinking`
  does not apply, and each session's effective setting is recorded from
  its transcript: `on` when a response holds a non-empty `thinking` or a
  `redacted_thinking` block, `off` otherwise (Haiku 5.5 emits an empty
  `thinking` block with no budget), unknown for a session with no response
  but API failures; an `on` session is flagged as any other level. Because
  of those words, the runner reads every planned task prompt before any
  session and stops with a `usage` error naming the file when one contains
  `think` in any case (Hermes has no such words). Hermes has a level,
  `--reasoning`: the driver passes `--thinking` as it (`off` as `none`)
  and children inherit it, but its transcript records only a child's
  level, never the main session's, so each Hermes session's effective
  setting is recorded as for Claude Code: `on` when it counted reasoning
  tokens or a response holds non-blank `reasoning` or `reasoning_content`
  (Hermes stores a lone space with reasoning off), else `off`. A requested
  level other than `off` is then flagged as a thinking mismatch in every
  Hermes run, so Hermes runs meant for comparison use `--thinking off`.
  `--thinking` stays the user's control for Pi and Hermes, so a run meant
  to compare with Claude Code uses `--thinking off` on them; the
  summary says in one sentence, whenever the requested or any recorded
  level is not `off`, that two harnesses' results compare only at the same
  setting and to run each driver with thinking off. Its children run on the
  session's model: the driver sets `CLAUDE_CODE_SUBAGENT_MODEL` to
  `--model`, else to the user's own `model` setting (else its
  `general-purpose` children run on the harness's built-in default, a
  retired model here); a child on another model is flagged as for Pi.
  Children get the thinking level too: for Pi the eval's settings set
  pi-subagents' `subagents.defaultThinking` and `maxThinking` to it and an
  `agentOverrides.<name>.thinking` for each bundled agent (`defaultThinking`
  does not override an agent's own frontmatter, such as `worker`'s `high`,
  and `maxThinking` fails a launch above it rather than lowering it). Every
  session's level is recorded from its `thinking_level_change` records and
  each assistant message's `thinkingLevel`; a run where any differs from
  the requested one is flagged, not failed (the main agent may still ask a
  child for a lower level), and so is a session with neither.
  Sessions run without the user's personal harness setup: for Pi, one agent
  directory per invocation (`PI_CODING_AGENT_DIR`) holds a `settings.json`
  listing only `npm:pi-subagents` (plus the user's `defaultProvider`/
  `defaultModel`, and `~/.agents/skills` excluded), `auth.json` and the
  `npm/` package store linked to the user's (a linked `auth.json` shares
  OAuth refreshes instead of rotating the user's token away), and a copied
  `models-store.json`; no global `AGENTS.md`, other extensions, or skills
  beyond pi-subagents'; the summary records it. For Claude Code, one
  config directory per invocation (`CLAUDE_CONFIG_DIR`) holds a
  `settings.json` with only the user's `model` and `.credentials.json`
  linked to the user's (shared OAuth refreshes, as for Pi): no user
  `CLAUDE.md`, agents, commands, hooks, plugins, or MCP servers (those live
  in the user's `.claude.json`, not copied). Where the user's directory has
  no `.credentials.json` (macOS keeps the login in the Keychain, which a
  session with another config directory does not read), sessions run in
  the user's own directory with `--setting-sources project,local
  --strict-mcp-config`: no user settings, hooks, plugins, or MCP servers,
  but the user's `CLAUDE.md`, rules, skills, agents and commands are
  loaded in every arm, and the summary says so. Claude Code 1.0.128 does not
  read a project `AGENTS.md`, so for this driver every arm copy with an
  `AGENTS.md` (`block-only`, `full`, and the adoption copy before its
  session) also gets a one-line `CLAUDE.md`, `@AGENTS.md`, which imports
  it; the block text stays only in `AGENTS.md`; the file is folded into
  the copy's last commit (reflog expired), so the history keeps its shape
  and the run's `changed` does not see it, and adoption's unowned check
  accepts it. The summary says so. For Hermes, one template home per
  invocation (`<work>/a`) holds a `config.yaml` with only
  `delegation.subagent_auto_approve: true` (children run shell commands
  unprompted: the children's counterpart of `--yolo`) and
  `delegation.oneshot_max_children: 0` (unlimited: Hermes lets a one-shot
  session start 2 subagents by default and then refuses `delegate_task`,
  telling the model to review its own work inline, which would contradict
  the block; the eval lifts the cap so a run can delegate as on Pi and
  Claude Code, which have none; a user running stewards in a Hermes
  one-shot session meets that default), `.env` linked to
  the user's (the provider keys), and `installs/`, the Python dependency
  environment Hermes builds in every new home (424 MB, built once by
  `hermes sessions list`, no model call; a gateway daemon it may start for
  the template is stopped). Every session gets a fresh home
  `<work>/<n>/h` (`HERMES_HOME`) with that config, `.env` and `installs/`
  linked: no user config, memory, SOUL.md, skills, plugins, or earlier
  sessions (one home per eval would show later runs the earlier runs'
  sessions through `session_search`, and their memory); a fresh home holds
  Hermes's own default SOUL.md, bundled skills, and bundled plugins. A
  session starts only while `installs/` is unchanged since it was built
  (every entry's path, size, mtime, and mode, without the `.leases`
  directories and `*.lock` files Hermes touches on every use), else the
  eval stops. That catches a session that installed into the shared
  environment by accident; it is no guarantee: a `--yolo` session can
  write a file and restore its size and mtime, write under a `.leases`
  directory or a `*.lock` name, or change Hermes's own source tree
  (`~/.hermes/hermes-agent`, the daemon's code), which is not checked.
  **Key scrubbing, every driver:** each driver names the credential files
  it links (Pi the linked `auth.json`, Claude Code the linked
  `.credentials.json`, Hermes `.env`). After each session the runner reads
  them (so a token refreshed during the session counts) and collects every
  value that looks like a secret: a string value, at any depth of a JSON
  file, or a `NAME=value` line of any other, whose name holds `key`,
  `token`, `secret`, `password`, `access`, or `refresh` in any case, at
  least 8 characters. Every exact occurrence is replaced by `[redacted]`
  in every text file of the session directory, nested child sessions
  included, before it is kept, and in the run record; a value is never
  logged or printed. (Hermes blocks `read_file` on `.env` and redacts
  some tool output, but a shell `cat` can still print it; any harness's
  agent can print a credential file.) Hermes
  reads the project `AGENTS.md` itself, so it runs without
  `--ignore-rules` (which would skip it), and no copy holds another file
  it reads (`.hermes.md`, `.cursorrules`); the summary says so. Each session gets its own
  `CARGO_INSTALL_ROOT` (never on `PATH`), each `full` session its own
  `STEWARDS_HOME` (the other arms none, so the environment names no
  stewards), and every other harness env prefix is stripped (the Claude
  Code driver also drops `CLAUDECODE`, which an enclosing Claude Code sets
  for its shell outside the `CLAUDE_CODE` prefix, and
  `MAX_THINKING_TOKENS`; the Hermes driver drops `AI_AGENT`, which Hermes
  sets only when unset). Claude Code shows a session its git status and
  last five commit subjects unasked (Hermes its last commits too), so every arm commit the runner and
  `evals/arms.sh` make is subject `initial` or `update`, by `dev
  <dev@example.invalid>`, naming no eval, `stewards`, fixture, adoption,
  or block. Runs are sequential, each in a
  fresh copy (made by `evals/arms.sh` from the fixture suite's source
  directory) at `<tmp>/w-<random>/<n>/repo`, a
  path naming no arm, `stewards`, or `eval` (the run record keeps the
  mapping); `bare` and `block-only` run with `stewards` off `PATH` (each
  `PATH` directory holding it is replaced by a mirror without it), `full`
  with the running binary first on `PATH`. `full` copies one adopted
  fixture per invocation: `stewards init`, then hand-set units from
  `evals/full-units.toml` (`api`, `client`, `cli`; `init` proposes only the
  `cli` manifest root on the fixture; the root `.gitignore` and `AGENTS.md`
  stay unowned, anything else unowned stops the eval; the summary says so
  in plain words; those names, and the driver's context file, become the
  adopted copy's `unowned_ok`), each with its own test command on its test card from
  `[source.test_commands]` in `evals/suites/fixture.toml` (`python3 -m
  unittest discover -s api`, `node --test client/`, `cargo test
  --manifest-path cli/Cargo.toml`; the root has no toolchain, so no
  `commands.test`), then an adoption session on the same model told to run
  `stewards init --draft`, start one subagent per handoff it prints, then
  run `stewards review --roles` until it exits 0, recorded as a `setup`
  run. The runner then diffs the adopted state
  against its `initial` commit: a changed, deleted, or added file
  outside `.stewards/`, `docs/units/`, `AGENTS.md`, and `.gitignore` stops
  the eval with `eval_setup_incomplete` naming the files, except a newly
  added file under the test locations `tests` (`api/test_*.py`,
  `client/*.test.ts`, `cli/tests/**`, beside `unowned` in
  `evals/suites/fixture.toml`); the summary lists every added
  path, every gap of every kind on the adopted cards, and every flow
  adoption recorded (a task whose seeded bug or answer is listed started
  known), plus a report-only line: adoption's total tokens as a multiple
  of the median `bare` task total. The runner checks `stewards next`
  exits 0 there (else `eval_setup_incomplete`, before any `full` run),
  removes every gitignored file, and commits it; each `full` copy runs
  `git clean -fdX` after the copy. `--setup-dir` (fixture suites only, else `usage`) reuses an adopted copy; it
  must be a git repository with an empty `git status --porcelain` and a
  clean `next`, and gets the same diff check. `--no-developers` is `full`
  with the block's "Changing:" line cut to its first sentence, `- Changing:
  make the edit yourself.` (dropping the `stewards brief <unit> --role
  developer` sentence), labelled `full-no-developers`. A task with a `setup` runs in a new
  session in the directory its setup task left: each run of fixture task 6
  first runs task 5 there as a `setup` run. A fixture session has 30
  minutes, then the harness's process group is killed
  and the session recorded `timed_out`. After every session (normal exit,
  failure, timeout, or interrupt) the runner kills every process whose
  working directory is under that session's `<n>/` directory, or whose
  `HERMES_HOME` (no other variable, so a
  process that only visited the directory, with it in `OLDPWD`, is left
  alone) is a path under it (on Linux from `/proc/<pid>/cwd` and
  `/proc/<pid>/environ`; on macOS from `lsof` and `ps -E`, which shows no
  environment for a system binary; Hermes's own daemon kill reads `/proc`
  and so does nothing on macOS) (Pi starts shell commands and background subagents in their own process
  groups, Claude Code its shell commands, its subagents running inside its
  own process; the Hermes agent runs in a gateway daemon that `hermes
  chat` starts in its own session, with Hermes's source tree as its working
  directory, not the task directory, though its shell and file tools act
  in the `--in` directory: the driver kills the daemon's process group,
  named in `<home>/gateway.pid`, after the session, a timeout included,
  only while that pid's environment holds the session's `HERMES_HOME` (a
  stale pid may name another process), and the sweep finds anything else
  by the `HERMES_HOME` it carries; the summary says so),
  waits up to 10 s until none is left, and records how many it killed; a
  survivor stops the eval after the run is recorded. A SIGINT or SIGTERM
  to the runner (one handler, every driver) kills the running session's
  process group at once, lets the driver stop what it started (Hermes's
  daemon, whose turn would otherwise go on with the user's key) and the
  sweep run, records that session `failed` with `failure: interrupted`
  and its token counts `null` (partial, as for a timeout), starts no other
  session, writes the summary, and exits non-zero (`interrupted`); a
  second signal ends the runner at once. A failed or timed-out
  session is recorded and counted, never retried or dropped. `--max-cost`
  stops before the next session when the spend so far plus its mean per
  session would pass the ceiling; the first always runs. Results go to
  `evals/results/<timestamp>/` (gitignored): `runs/*.json` (the `Run`, arm,
  task, role, model, thinking, model and thinking mismatch flags, status,
  failure, leftovers, `next` check, whether tracked files changed,
  transcript path, review outcome), `sessions/` (each session's
  transcripts, copied there after the run), `summary.json`, and
  `summary.md`, with each task's verdict and gate count, its overhead
  ratios, their medians across tasks, the review outcomes, the flags, and
  costs to 4 decimals (the spend is `null`, not zero, when no run has a
  cost). Each run's work directory is deleted after the run unless
  `--keep-work`. pi-subagents also leaves run state outside both, under
  `/tmp/pi-subagents-uid-<uid>/` (`async-subagent-runs/`,
  `async-subagent-launches/`, `async-subagent-results/`,
  `nested-subagent-events/`, `run-fanout-budgets/`, `session-leases/`,
  `supervisor-channels/`); the eval does not clean it.
- **Pass criteria:** 3 runs per arm per task. The ritual is the gate, on
  edit tasks only (read-only tasks are `n/a`): a `full` run passes when
  the runner saw it change the run directory, a `stewards review` command
  ran in the session, a subagent was started whose task holds the handoff
  of a brief under `.stewards/briefs/` (`handoff`), and the runner's own
  `stewards next --no-brief --no-run` exits 0 after the session (which proves the
  review covered the final state: a stamp matches only the snapshot it
  was made at). `status`, `route`, `next`, `handoff_exact`, and
  `reviewer_submit` are recorded, not gated. The task passes when every
  `full` run passes, shown `k/n`, and fails otherwise, each failing run
  named with what it missed (`stewards review` never ran, no subagent was
  given a handoff, and so on).
  Overhead is reported, not judged: per task the `full`/`bare` ratios of
  the median main tokens, total tokens, cost, and peak main context (and,
  with `files_opened_vs_bare`, of main-session files opened against that
  `bare` task, fixture task 6 against 4), each ratio's median across the
  tasks that are not incomplete, and each run's count of subagent
  sessions. Thresholds on overhead will be set by the user after the first
  measurement with the reviewer role. A task with any compared run
  failed, timed out, without a transcript, or not started is
  `incomplete`: its medians are reported but given no verdict, and it
  never passes. Not in 0.1.0: the cleanliness and docs-accuracy LLM
  judges (a BAML judge, 3 votes, majority, over rung failures, untraced
  lines and missing checks; and no claim in `docs/units/api.md` contradicted
  by code after task 1). No run is graded for cleanliness or docs accuracy.
  Bug fixed (report-only, every
  arm): a task naming `repro = "<dir under evals/repros>"` (fixture task 2:
  `medium-api-pagination`) has that `repro.sh` run after each run, from the
  checkout with `FIXTURE_ROOT=<run dir>` and a 60 s timeout; `bug_fixed` is
  true when it exits 0, and the summary shows `k/n` per arm. Plan
  (report-only, `full` runs of a task with `expects_plan = true`, fixture
  task 5): `plan: {stored, closed}`, `stored` when a child's `stewards
  submit --role architect` printed `plan <id> stored`, `closed` when stored
  and no `.stewards/plans/*.toml` is left after the session; the summary
  shows `plan stored k/n, closed k/n`. Neither changes a verdict. The suite fails when any task fails, is
  incomplete when any task is, and passes otherwise: every edit task's
  gate passed and no run is incomplete (exit code 0 only then).
- **Cadence:** on demand, by hand: the `fixture` and `self` suites before a
  release (`RELEASING.md`, step 5). Not in CI: CI has no harness
  subscriptions, so no eval job runs on release tags. `--max-cost` ceiling.

## Release and versioning

Releases from tags via GitHub Actions; matrix `x86_64-unknown-linux-gnu`,
`aarch64-unknown-linux-gnu`, `aarch64-apple-darwin`, `x86_64-apple-darwin`;
each archive (`stewards-<version>-<target>.tar.gz`: `bin/`, `lib/stewards/`
with the matching `libbaml_cffi`, `share/` with completions and the man page,
`LICENSE`, `README.md`) is built from the tag, and the library is downloaded
and checked against a sha256 pinned in the workflow (the BAML release
publishes none). A job writes `SHA256SUMS` and creates the release with the
archives and `install.sh`, which installs under `${STEWARDS_PREFIX:-$HOME/.local}`
after verifying the archive against `SHA256SUMS`. The binary finds the
bundled library by its place (see LLM modes), so no script sets
`BAML_LIBRARY_PATH`. After the release, separate jobs run `cargo publish` and
bump the AUR PKGBUILD (builds from the crates.io source, pins the runtime lib
as a source entry). The workflow generates completions and the man page with
`stewards completions` and `stewards man` (`clap_complete`, `clap_mangen`).
Checklist in `RELEASING.md`.

## Build order

What happened: work landed as numbered PRs on topic branches; the number is
in brackets. Stages 3 and 4 were run as the ten tasks of the 0.1.0 plan
(`docs/build/stage3/PLAN.md`), and the tasks without a PR number landed as
merges of `feat/s3-*` branches.

1. **Core, no LLM** [#4, #5]: schemas, workspace resolution,
   `route`, `check`, `stamp`, `status`, `debt`, `pack`, `next` (with
   `brief_path: null`). Checkpoints: model + workspace + cli skeleton;
   git + check + stamp; route + debt + pack; next + workspace fixture + CI.
2. **Roles** [#6 to #9]: `llm.rs`, BAML classes, `brief` and
   `submit` for the maintainer end to end, then the other roles; `init` two
   steps on stewards itself (first committed `.stewards/`); `docs`,
   `guide`, memory, telemetry. Dogfood from here.
   **2b. Proof** [#10, then #11 to #18]: the eval fixture, the runner, the
   Pi driver, and the token and ritual graders, run on tasks 1, 3, 4, and 6;
   #11 to #18 are the `review` command, the Claude Code and Hermes drivers,
   and the fixes the evals and dogfooding showed.
   The 2026-11-12 decision replaced the token-saving gate (see Handoff,
   "What the thin eval showed"): stage 3 starts when an agent completes the
   one-job ritual (one review brief per change) unprompted in the eval and
   the user accepts its measured overhead.
3. **Depth** [#19, #21, #22, #23, and merges]: the allowlisted command runner
   [#19]; tree-sitter static grammars [#21]; stored architect plans and flow
   checks [#22]; runtime grammars [#23]; `validate --report`; the report-only
   eval graders; `validate --export` to GitHub, GitLab, Jira and Linear.
   `render` is cut; see below.
4. **Edges** [#20, and merges]: completions, man page, release workflow,
   install script, AUR package, `RELEASING.md`, PR and MR comments
   (`next --pr`, `review --pr`), and CI templates (`init --ci`). Cut from
   0.1.0: the cleanliness and docs-accuracy LLM judges, the Codex driver, and
   eval in CI on release tags; `RELEASING.md` has a manual eval pass instead.
   (Git hooks and the plugin shell are cut; see below.)

**0.1.0 scope (the user's decision, 2026-11-14).** `render`, hooks (`init
--hooks` and the user-level hook accelerators), and the plugin shell are
cut: each exists only to do something per harness, which Goal 1 rules out.
Plans, tree-sitter anchors, flow checks, `validate`, issue export to all
four trackers (GitHub, GitLab, Jira, Linear), a CI template, completions
and the man page, and the release pipeline stay. The `render` command and
the `--hooks` flag are removed from the CLI.

Work lands as numbered PRs on topic branches, one commit per checkpoint, CI
green at each commit, squash merge. Rough single-builder effort: 4, 6–7, 3 (2b), 6–8, 5–6 days. The BAML
spike (risk 1) runs beside stage 1 in a scratch directory, before `llm.rs`
is designed around it. If scope slips,
cut in this order: tracker export (the Codex eval driver is already cut).

Dogfood: stage 1 is verified against a hand-written, uncommitted
`.stewards/` for this repo; the committed one comes from `init` in stage 2,
tested against the known answer below. From stage 2 on, every change runs
stewards' own ritual, each handoff given to the harness's subagents (Pi
with `pi-subagents` on the Linux machine, Claude Code on the macOS one).
stewards itself is the real-repo eval target once the fixture eval passes.

## Build cycle

How the build itself is run: one orchestrating session and a roster of
subagents, tuned for wall-clock speed. Settled 2026-11-11.

### Roster

The roster in force (2026-11-14). Built-in `pi-subagents` agents with the
model set per call as `provider/model`; no agent files in the repo.

| who | model | used for |
|---|---|---|
| main session | the user's chosen model | orchestration: briefs, applying patches, gates, checkpoint commits, triage |
| contract-heavy writer | `anthropic/claude-opus-5-5`, thinking medium | contract-heavy or cross-cutting work only |
| writer and reviewer | `anthropic/claude-sonnet-5-5`, thinking high | the default writer and the default reviewer |
| mechanical writer, probes | `anthropic/claude-haiku-5-5`, thinking high | mechanical work and probes |
| cheapest | `deepseek/deepseek-flash` | the cheapest mechanical work and weak-model eval runs |

Routine review rounds run in a background workflow, not in the main
session. Every launch names its model: a child with no model inherits the
parent's, so an unpinned launch is a mistake, and `delegate` (which always
inherits) is not used. One resolved model per launch. On a quota or
availability failure the lane stops and is reported; the orchestrator
never swaps a model silently.

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
  runner, both tracker tasks, PR comments, the release pipeline.

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
| 3 | plan → allowlist runner and plans ‖ tree-sitter → validate ‖ runtime grammars ‖ flow checks → other trackers → gate (render cut) |
| 4 | plan → PR glue ‖ release → report-only eval graders and gate (hooks, plugin shell, LLM judges, Codex driver cut) |

Stage 2's first five tasks and the fixture appear in stage 1's row because
that is when they run; they still land on their own stage's branch.

## stewards' own units

| unit | paths | interface ids |
|---|---|---|
| `cli` | `src/main.rs`, `src/lib.rs`, `src/output.rs`, `src/completions.rs`, `src/review.rs`, `src/validate.rs`, `tests/validate.rs`, `src/forge.rs`, `tests/forge.rs`, `src/tracker.rs`, `tests/tracker.rs`, `tests/skeleton.rs`, `tests/common/**` | `json-envelope`, `cli-error`, `usage-errors`, `global-flags`, `not-implemented`, `review-command`, `completions-man`, `validate-report`, `forge-comment`, `validate-export` |
| `model` | `src/model.rs`, `src/workspace.rs`, `src/user.rs`, `tests/workspace.rs`, `tests/user.rs`, `tests/safety.rs`, `tests/fixtures/**` | `load-repo`, `unit-name`, `anchor-parse`, `loaded-unit`, `role-from-str`, `discover`, `repo-containing`, `current-repo`, `loaded-repos`, `display-name`, `safe-path`, `repo-write`, `repo-append`, `repo-remove`, `user-config`, `user-memory`, `memory-for`, `loaded-card`, `repo-read`, `plain-names`, `user-grammars-dir` |
| `git` | `src/git.rs`, `tests/git.rs` | `ls_files`, `snapshot`, `ls_tree`, `has_object`, `is_tree`, `diff_names`, `diff_file`, `diff_added_lines`, `head_commit`, `task_text`, `recent_commit_files`, `origin_url`, `diff_hunks`, `blob_text` |
| `analysis` | `src/anchors.rs`, `src/anchors/**`, `src/route.rs`, `src/check.rs`, `src/debt.rs`, `tests/anchors.rs`, `tests/route.rs`, `tests/route_workspace.rs`, `tests/check.rs`, `tests/debt.rs`, `tests/fixtures/grammars/**` | `route-owner`, `route-overlaps`, `route-command`, `anchor-resolve`, `anchor-source-window`, `check-fingerprint`, `check-stamp`, `check-report`, `debt-scan`, `anchor-interface-lines`, `check-instructions` |
| `pack` | `src/pack.rs`, `tests/pack.rs` | `pack`, `developer`, `render`, `fit`, `run`, `review`, `validator` |
| `next` | `src/next.rs`, `src/plan.rs`, `src/status.rs`, `tests/plans.rs`, `tests/next.rs`, `tests/status.rs` | `next`, `next_run`, `status`, `status_run`, `review_due` |
| `llm` | `src/llm.rs`, `baml_src/**`, `src/baml_client/**`, `src/llm_schema/**`, `tests/llm.rs` | `role-text`, `output-format`, `parse-answer`, `prepare-baml-env`, `direct-guards`, `direct-call`, `schema-freshness`, `schema-caps` |
| `jobs` | `src/brief.rs`, `src/submit.rs`, `src/telemetry.rs`, `tests/brief.rs`, `tests/submit.rs`, `tests/submit_roles.rs`, `tests/developer.rs`, `tests/review.rs` | `brief-header`, `brief-path`, `brief-write`, `brief-build`, `submit-file`, `anchor-name-bare`, `write-and-retire`, `telemetry-append`, `telemetry-summary`, `card-patch`, `tester-bug`, `documenter-index`, `review-brief`, `review-submit`, `brief-handoff`, `job-name`, `command-allowlist`, `answer-too-long`, `first-review`, `finding-answer`, `architect-plan`, `validator-repro` |
| `setup` | `src/init.rs`, `src/guide.rs`, `src/harness.rs`, `src/docs.rs`, `tests/init.rs`, `tests/init_draft.rs`, `tests/guide.rs`, `tests/docs.rs`, `tests/fixtures/docs/**` | `init`, `init-draft`, `agents-block`, `guide`, `harness-detect`, `harness-by-name`, `docs`, `init-ci` |
| `eval` | `src/eval/**`, `tests/eval.rs`, `tests/eval_interrupt.rs`, `tests/fixtures/transcripts/**`, `evals/**` | `eval_run`, `execute`, `driver`, `pi_parse`, `pi_agent_dir`, `suite_load`, `grade_task`, `gate_misses`, `suite_verdict`, `ritual_steps`, `stewards_commands`, `shell_reads`, `existing_files`, `reviewed_units`, `would_exceed`, `without_developers`, `path_without_stewards`, `next_check`, `kill_leftovers`, `claude_parse`, `claude_config_dir`, `task_prompt`, `pinned_commit`, `hermes_parse`, `hermes_template`, `eval_redact_secrets`, `answers_given`, `eval_bug_fixed`, `eval_plan_outcome` |
| `run` | `src/run.rs`, `tests/run.rs` | `run-allowed`, `run-check`, `run-run`, `run-shell-chars` |
| `dist` | `.github/workflows/release.yml`, `install.sh`, `packaging/**`, `RELEASING.md`, `tests/completions.rs`, `licenses/**`, `about.toml`, `about.hbs` | `release-workflow`, `install-sh`, `aur-bump`, `aur-pkgbuild`, `release-checklist`, `completions-tests` |


Flows: `next-to-submit` (`cli` → `next` → `analysis` → `pack` → `llm` →
`model`), `init` (`cli` → `model` → `git` → `llm` → `model`). Decisions:
facts in Rust, judgment in BAML; git through the CLI; text anchors first,
tree-sitter behind the same function.

## Module API (as built)

The code is the reference; this is the map. `src/lib.rs` declares every
module and `src/main.rs` is a thin binary over it: clap, then one `run` per
command returning `anyhow::Result<ExitCode>`. A `CliError` anywhere becomes
the JSON error; anything else is `internal`.

```rust
// anchors.rs
pub fn interface_lines(path: &str, text: &str, name: &str) -> Vec<u32>
pub fn resolve(root: &Path, a: &Anchor) -> Resolution
pub fn source_window(root: &Path, a: &Anchor, before: usize, after: usize) -> Option<(u32, String)>

// anchors/runtime.rs
pub fn unusable() -> Vec<(&'static Path, &'static str)>

// brief.rs
pub fn read_header(path: &Path) -> Result<Header>
pub fn rel_path(unit: Option<&str>, role: Role, n: Option<usize>, snapshot: &str) -> String
pub fn job_of(name: &str) -> Option<JobName>
pub fn find(root: &Path, unit: Option<&str>, role: Role) -> Result<(String, Option<usize>)>
pub fn answer_rel(brief_rel: &str) -> String
pub fn result_rel(brief_rel: &str) -> String
pub fn handoff(path: &Path) -> Result<String>
pub fn build(repo: &Repo, unit: Option<&str>, role: Role, mode: Mode, task: &str, snapshot: &str) -> Result<String>
pub fn build_validator(repo: &Repo, unit: &str, bug: Option<&str>, task: &str, snapshot: &str, result: Option<serde_json::Value>) -> Result<String>
pub fn repro_result(repo: &Repo, unit: &str, repro: &str) -> Result<serde_json::Value>
pub fn build_review(repo: &Repo, due: &[next::DueUnit], task: &str, snapshot: &str) -> Result<Vec<(String, ReviewPack)>>
pub fn write_round(repo: &Repo, snapshot: &str, texts: &[&str]) -> Result<Vec<PathBuf>>
pub fn write_text(repo: &Repo, unit: Option<&str>, role: Role, n: Option<usize>, snapshot: &str, text: &str) -> Result<PathBuf>
pub fn write(repo: &Repo, unit: Option<&str>, role: Role, mode: Mode, task: &str, snapshot: &str) -> Result<(PathBuf, usize)>
pub fn run(g: &Global, unit: Option<&str>, role: Role, task: Option<&str>, stdout: bool) -> anyhow::Result<ExitCode>

// check.rs
pub fn one_line(s: &str) -> String
pub fn invisible(c: char) -> bool
pub fn escape_invisible(text: &str) -> String
pub fn unowned_ok(repo: &RepoConfig) -> Result<GlobSet>
pub fn is_stale(reviewed: Option<&Reviewed>, current: &str) -> bool
pub fn fingerprint(tree: &[(String, String)], unit: &str, router: &Router) -> String
pub fn reviewed_at(repo: &Repo, unit: &str, tree: &str, marks: Option<&Reviewed>) -> Result<Reviewed>
pub fn stamp(repo: &Repo, unit: &str, tree: &str) -> Result<Reviewed>
pub fn in_snapshot(entries: &[(String, String)], path: &str) -> bool
pub fn changed_since(root: &Path, snapshot: &str, entries: &[(String, String)], router: &Router, unit: &str, tree: Option<&str>) -> Result<Vec<String>>
pub fn stale_files(root: &Path, snapshot: &str, entries: &[(String, String)], router: &Router, unit: &str, reviewed: Option<&Reviewed>) -> Result<Option<Vec<String>>>
pub fn dead_anchors(root: &Path, entries: &[(String, String)], card: Option<&UnitCard>) -> Vec<String>
pub fn check(ws: &Workspace, filter: Option<&str>) -> Result<CheckReport>
pub fn check_at(ws: &Workspace, filter: Option<&str>, snapshots: &[String]) -> Result<CheckReport>
pub fn run(g: &Global) -> Result<ExitCode>
pub fn run_stamp(g: &Global, unit: &str) -> Result<ExitCode>

// completions.rs
pub fn run(shell: Shell, mut cli: clap::Command) -> anyhow::Result<ExitCode>
pub fn run_man(cli: clap::Command) -> anyhow::Result<ExitCode>

// debt.rs
pub fn is_debt_line(line: &str) -> bool
pub fn scan(repo: &Repo) -> Result<BTreeMap<Option<String>, Vec<DebtItem>>>
pub fn run(g: &Global) -> Result<ExitCode>

// docs.rs
pub fn run(g: &Global, unit: Option<&str>) -> anyhow::Result<ExitCode>
pub fn pages(loaded: &Loaded, unit: Option<&str>) -> anyhow::Result<Vec<(String, String)>>

// eval/mod.rs
pub fn as_str(self) -> &'static str  // Arm
pub fn is_full(self) -> bool  // Arm
pub fn as_str(self) -> &'static str  // Status
pub fn sum(&self) -> u64  // Tokens
pub fn context(&self) -> u64  // Tokens
pub fn add(&mut self, other: Tokens)  // Tokens
pub fn reviewed_units(output: &str) -> Option<Vec<ReviewedUnit>>
pub fn is_handoff(text: &str) -> bool
pub fn holds_handoff(text: &str) -> bool
pub fn answers_given(shell_commands: &[String]) -> usize
pub fn is_reviewer_submit(cmd: &[&str]) -> bool
pub fn is_architect_submit(cmd: &[&str]) -> bool
pub fn plan_stored(output: &str) -> bool
pub fn ritual_steps(actions: &[Action]) -> RitualSteps
pub fn reads_git_history(command: &str) -> bool
pub fn stewards_commands(command: &str) -> Vec<Vec<&str>>
pub fn shell_reads(command: &str) -> Vec<String>
pub fn existing_files(dir: &Path, paths: &[String]) -> Vec<String>
pub fn credential_secrets(files: &[PathBuf]) -> Vec<String>
pub fn redact_text(text: &str, secrets: &[String]) -> String
pub fn redact_secrets(driver: &dyn Driver, dir: &Path) -> Result<Vec<String>>
pub fn fresh_session_env(cmd: &mut Command, env: &[(&str, OsString)])
pub fn plan_outcome(dir: &Path, stored: bool) -> PlanOutcome
pub fn would_exceed(max: f64, costs: &[f64]) -> bool
pub fn execute(driver: &dyn Driver, plan: &Plan) -> Result<Summary>
pub fn task_prompt(dir: &Path, task: u32) -> Result<String>
pub fn adopted_knowledge(dir: &Path) -> Result<(Vec<String>, Vec<String>)>
pub fn next_check(stewards: &Path, dir: &Path) -> Result<NextCheck>
pub fn kill_leftovers(dir: &Path) -> Result<Option<Leftovers>>
pub fn without_developers(agents_md: &str) -> Result<String>
pub fn path_without_stewards(path: &OsStr, mirrors: &Path) -> Result<OsString>
pub fn neutral_work_dir(tmp: &Path) -> PathBuf
pub fn run(g: &Global, f: Flags) -> Result<ExitCode>

// eval/claude.rs
pub fn find(path: &OsStr, limit: Duration) -> Result<ClaudeCode>  // ClaudeCode
pub fn isolate(&mut self, dir: &Path) -> Result<String>  // ClaudeCode
pub fn config_dir(user: &Path, dir: &Path) -> Result<(String, Option<String>)>
pub fn parse(transcript: &Path, duration_s: f64, truncation_ok: bool) -> Result<Run>

// eval/grade.rs
pub fn median(xs: &[f64]) -> Option<f64>
pub fn arm_stats(arm: Arm, runs: &[&RunRecord]) -> ArmStats
pub fn gate_misses(r: &RunRecord) -> Vec<&'static str>
pub fn grade_task(suite: &Suite, task: u32, records: &[RunRecord], arms: &[Arm], runs: u32) -> TaskGrade
pub fn suite_verdict(grades: &[TaskGrade]) -> Verdict
pub fn overhead_medians(grades: &[TaskGrade]) -> Vec<OverheadMedian>
pub fn summarize(plan: &Plan, records: &[RunRecord], stopped: bool, unknown_cost: &[String], setup: SetupNotes) -> Summary
pub fn render(s: &Summary) -> String

// eval/hermes.rs
pub fn find(path: &OsStr, limit: Duration) -> Result<Hermes>  // Hermes
pub fn isolate(&mut self, dir: &Path) -> Result<String>  // Hermes
pub fn template(bin: &Path, user: &Path, dir: &Path) -> Result<(String, Vec<String>)>
pub fn fingerprint(dir: &Path) -> Result<Vec<String>>
pub fn parse(transcript: &Path, duration_s: f64, truncation_ok: bool) -> Result<Run>

// eval/pi.rs
pub fn find(path: &OsStr, limit: Duration) -> Result<Pi>  // Pi
pub fn isolate(&mut self, dir: &Path, thinking: &str) -> Result<String>  // Pi
pub fn agent_dir(user: &Path, home: &Path, dir: &Path, thinking: &str) -> Result<String>
pub fn parse(transcript: &Path, duration_s: f64, truncation_ok: bool) -> Result<Run>

// eval/suite.rs
pub fn load(evals: &Path, name: &str) -> Result<Suite>
pub fn task(&self, number: u32) -> Option<&SuiteTask>  // Suite
pub fn numbers(&self) -> Vec<u32>  // Suite
pub fn check_tasks(&self, tasks: &[u32]) -> Result<()>  // Suite
pub fn commit(&self) -> Option<&str>  // Suite

// forge.rs
pub fn of_origin(cli: &dyn Cli, origin: &str) -> Result<Forge>  // Forge
pub fn ok(stdout: &str) -> Out  // Out
pub fn new(rules: Vec<(&str, Out)>) -> Fake  // Fake
pub fn post_comment(cli: &dyn Cli, forge: Forge, origin: &str, c: &Comment, github_actions: Option<&str>) -> Result<PrComment>
pub fn post(root: &Path, pr: u64, marker: &str, title: &str, text: &str) -> Result<PrComment>
pub fn error_line(e: &anyhow::Error) -> String

// git.rs
pub fn ls_files(root: &Path) -> Result<Vec<String>>
pub fn snapshot(root: &Path) -> Result<String>
pub fn ls_tree(root: &Path, tree: &str) -> Result<Vec<(String, String)>>
pub fn has_object(root: &Path, sha: &str) -> bool
pub fn is_tree(root: &Path, sha: &str) -> bool
pub fn diff_names(root: &Path, from: &str, to: &str) -> Result<Vec<String>>
pub fn diff_file(root: &Path, from: &str, to: &str, path: &str) -> Result<String>
pub fn diff_added_lines(root: &Path, from: &str, to: &str) -> Result<Vec<(String, u32, String)>>
pub fn diff_hunks(root: &Path, from: &str, to: &str) -> Result<Vec<Hunk>>
pub fn blob_text(root: &Path, sha: &str) -> Result<String>
pub fn head_commit(root: &Path) -> Result<Option<String>>
pub fn origin_url(root: &Path) -> Option<String>
pub fn task_text(root: &Path) -> Result<String>
pub fn recent_commit_files(root: &Path, n: usize) -> Result<Vec<Vec<String>>>

// guide.rs
pub fn block() -> &'static str
pub fn upsert_block(repo: &Repo) -> anyhow::Result<bool>
pub fn run(g: &Global, harness: Option<&str>, all: bool, errors: bool, glossary: bool) -> anyhow::Result<ExitCode>

// harness.rs
pub fn detect() -> Option<&'static Harness>
pub fn by_name(name: &str) -> anyhow::Result<&'static Harness>

// init.rs
pub fn run(g: &Global, draft: bool, ci: bool) -> Result<ExitCode>

// llm.rs
pub fn role_text(role: Role, mode: Mode) -> &'static str
pub fn output_format(role: Role, mode: Mode) -> &'static str
pub fn parse(role: Role, mode: Mode, text: &str) -> Result<Answer, Vec<FieldError>>
pub fn direct_guards(role: Role) -> Result<(&'static str, String, String)>
pub fn direct(role: Role, mode: Mode, unit: &str, brief_text: &str) -> Result<String>

// model.rs
pub fn parse(s: &str) -> Result<Anchor>  // Anchor
pub fn as_str(&self) -> &'static str  // Role
pub fn unit(&self, name: &str) -> Result<&UnitDef>  // Loaded
pub fn card(&self, unit: &str) -> UnitCard  // Loaded
pub fn load_repo(root: &Path) -> Result<Loaded>

// next.rs
pub fn checks_failed(&self) -> bool  // NextReport
pub fn unowned_line(&self) -> Option<String>  // NextReport
pub fn next(ws: &Workspace, task: Option<&str>, briefs: Briefs, roles: bool, run_checks: bool) -> Result<NextReport>
pub fn review_due(repo: &Repo, task: Option<&str>) -> Result<(String, String, Vec<DueUnit>)>
pub fn run(g: &Global, task: Option<&str>, no_brief: bool, roles: bool, no_run: bool, pr: Option<u64>) -> Result<ExitCode>

// output.rs
pub fn emit<T: Serialize + Human>(v: &T, human: bool) -> bool
pub fn new(code: &'static str, message: impl Into<String>, fix: impl Into<String>) -> Self  // CliError
pub fn emit(&self, human: bool)  // CliError
pub fn with_note(e: anyhow::Error, note: String) -> anyhow::Error
pub fn not_implemented(command: &str) -> anyhow::Result<std::process::ExitCode>

// pack.rs
pub fn pack(repo: &Repo, unit: Option<&str>, role: Role, task: &str, diff: bool, snapshot: &str) -> Result<Pack>
pub fn validator(repo: &Repo, unit: &str, task: &str, snapshot: &str, bug: Option<&str>, repro: Option<serde_json::Value>) -> Result<Pack>
pub fn developer(repo: &Repo, unit: &str, task: &str, n: usize, dev: &Task, snapshot: &str) -> Result<Pack>
pub fn review(repo: &Repo, due: &[DueUnit], task: &str, snapshot: &str) -> Result<ReviewRound>
pub fn render(&self) -> String  // ReviewPack
pub fn render(&self) -> String  // Pack
pub fn fit(&mut self, budget: usize)  // Pack
pub fn run(g: &Global, unit: Option<&str>, role: Role, diff: bool, task: Option<&str>) -> Result<ExitCode>

// plan.rs
pub fn plans_and_flows(ws: &Workspace, repo: &Repo, loaded: &Loaded, snapshot: &str, run_checks: bool) -> Result<(Vec<PlanReport>, Vec<FlowCheck>)>

// review.rs
pub fn run(g: &Global, task: Option<&str>, roles: bool, pr: Option<u64>, answers: &[String]) -> Result<ExitCode>

// route.rs
pub fn new(units: &[UnitDef], docs_path: &str) -> Result<Router>  // Router
pub fn tool_owned(&self, path: &str) -> bool  // Router
pub fn owner(&self, path: &str) -> Option<&str>  // Router
pub fn overlaps(&self, files: &[String]) -> Vec<(String, Vec<String>)>  // Router
pub fn run(g: &Global, paths: &[String]) -> Result<ExitCode>

// run.rs
pub fn allowed(loaded: &Loaded, unit: Option<&str>) -> Vec<Allowed>
pub fn check<'a>(cmd: &str, allowed: &'a [Allowed]) -> Result<&'a Allowed, String>
pub fn run(root: &Path, allowed: &[Allowed], cmd: &str) -> Result<RunResult>

// status.rs
pub fn status(ws: &Workspace) -> Result<StatusReport>
pub fn run(g: &Global, telemetry: bool) -> Result<ExitCode>

// submit.rs
pub fn run(g: &Global, unit: Option<&str>, role: Role, file: &Path) -> Result<ExitCode>
pub fn documenter(repo: &Repo, unit: &str, header: &Header, text: &str, before: &Before) -> Result<Submitted>
pub fn reviewer(repo: &Repo, _unit: &str, header: &Header, text: &str, before: &Before) -> Result<Submitted>
pub fn record(root: &Path, rels: &[String]) -> Result<Before>
pub fn write_and_retire(repo: &Repo, before: &Before, writes: &[(String, String)], brief_rel: &str) -> Result<()>
pub fn write_remove_and_retire(repo: &Repo, before: &Before, checked: &[&str], changes: &[(String, Option<String>)], brief_rel: &str) -> Result<()>
pub fn pending(&self) -> bool  // StoredFinding
pub fn answerable(&self) -> bool  // StoredFinding

// telemetry.rs
pub fn append(repo: &Repo, record: &Record) -> Result<()>
pub fn rfc3339(t: SystemTime) -> String
pub fn summary(root: &Path) -> Value
pub fn human(summary: &Value) -> Vec<String>

// tracker.rs
pub fn new(responses: Vec<(u16, &str)>) -> FakeHttp  // FakeHttp
pub fn backend<'a>(t: &Tracker, env: &dyn Fn(&str) -> Option<String>, cli: &'a dyn Cli, origin: Option<&str>, http: &'a dyn Transport) -> Result<Box<dyn Issues + 'a>>
pub fn body(kind: TrackerKind, unit: &str, severity: &str, gap: &crate::model::Gap) -> String
pub fn export<'a>(repo: &Repo, open: impl FnOnce(&Tracker) -> Result<Box<dyn Issues + 'a>>) -> Result<Exported>

// user.rs
pub fn model(&self, harness: &str, role: &str) -> Option<&str>  // Config
pub fn grammars_dir() -> Option<PathBuf>
pub fn load_config() -> Result<Config>
pub fn load_memory() -> Result<Vec<Entry>>
pub fn load_config_from(dir: &Path) -> Result<Config>
pub fn load_memory_from(dir: &Path) -> Result<Vec<Entry>>
pub fn memory_for<'a>(entries: &'a [Entry], role: &str, languages: &[String], budget_bytes: usize) -> Vec<&'a Entry>

// validate.rs
pub fn run(g: &Global, report: Option<&Path>, export: bool) -> Result<ExitCode>

// workspace.rs
pub fn discover(cwd: &Path, only: Option<&str>) -> anyhow::Result<Workspace>
pub fn here(g: &Global) -> anyhow::Result<(PathBuf, Workspace)>
pub fn loaded(&self) -> anyhow::Result<&Loaded>  // Repo
pub fn repo_containing(&self, path: &Path) -> Option<&Repo>  // Workspace
pub fn current_repo(&self, cwd: &Path) -> anyhow::Result<&Repo>  // Workspace
pub fn loaded_repos(&self) -> anyhow::Result<Vec<(&Repo, &Loaded)>>  // Workspace
pub fn display_name(&self, repo: &Repo, unit: &str) -> String  // Workspace
pub fn write(&self, rel: &str, contents: &[u8]) -> anyhow::Result<()>  // Repo
pub fn append(&self, rel: &str, contents: &[u8]) -> anyhow::Result<()>  // Repo
pub fn read(&self, rel: &str, cap: u64) -> anyhow::Result<Vec<u8>>  // Repo
pub fn remove(&self, rel: &str) -> anyhow::Result<()>  // Repo
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
| 3 | Subagents ignore the pack | `pulled_anchors` large; tokens per job far above pack size | Pull rule stated twice; budget shown; token grader | Replaced by the 2026-11-12 decision, which dropped the main-context saving criterion: the stage-3 gate is now an agent completing the one-job ritual unprompted in the eval, with a measured overhead the user accepts; missing it → reopen delegation before stage 3 starts |
| 4 | Cards rot despite `check` | Many `card_patch: null` while anchors drift; prose contradicts code | Fingerprint staleness; dead-anchor check; `next` re-reviews every stale unit | A dead anchor, or a docs page out of date with its card, at a release → regen-only cards |
| 5 | Prompt injection through cards or reports | A brief makes a subagent act outside its role | Data fencing, caps, allowlist, writes boundary; a hostile fixture card in the Rust tests (fence breakout, control characters, over-cap fields) | Any confirmed escape → stop-ship until fixed |
| 6 | Scope for one builder | Stages slip; no dogfood by stage 2 | Each stage usable alone; cut order above | Stage 2 not dogfooding in its window → freeze at stages 1–2 |
| 7 | Eval cost or flakiness | Runs exceed estimate; graders disagree | Report-only code graders (the LLM judges are not in 0.1.0); medians; `--max-cost` | Suite flakes on an unchanged tree twice → graders redesigned first |
| 8 | Codex driver untestable locally (the driver is not in 0.1.0) | `codex exec` 400s under the ChatGPT account | API-key auth for the runner, or Codex arm in CI only | Revisit when a Codex driver is built |

## Cost estimate (list prices, 2026-10; assumptions stated)

Opus 5.5 $4 in / $20 out per MTok, cache reads $0.20; Sonnet 5.5 $2 / $10,
cache reads $0.20; Haiku 5.5 $0.10 / $0.50 (prompts ≤ 100K). Claude Code,
Pi, and Hermes under subscriptions cost no API dollars; `--direct` does.

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

Every JSON output also carries a `stewards` object (`version`, `schema`),
left out below. Shapes and key names are what the commands print on
`tests/fixtures/single` (2026-11-16); the unit names are the example's.

```
> stewards status
{ units: [{ unit: "api", purpose, paths, freshness, interfaces, covered, gaps, bugs, debt }, ...],
  uninitialized: [], stamp_claim: "A stamp records that one reviewer, ..." }
> cat docs/units/api.md                                                   # how api works, before its source
> stewards route api/items.py
{ routes: [{ path: "api/items.py", unit: "api" }] }
[Agent] edits api/items.py, api/store.py, api/tests/test_items.py itself  # plannable from the unit page
> stewards review --task "add a count endpoint to api"
{ task: { text: "add a count endpoint to api", from: "given" }, reported: [],
  next: { jobs: [ { unit: "", role: reviewer, brief_path: "/work/repo/.stewards/briefs/review-1-3f9a1c2e.md",
                    handoff: "Read `/work/repo/.stewards/briefs/review-1-3f9a1c2e.md` and follow it.",
                    units: ["api"], reasons: ["api: stale", "api: interface changed: api/items.py::list_items"] } ], ... },
  then: "Start one subagent per handoff below, giving it exactly that sentence as its whole task (you can start them together); when they have finished, run stewards review again." }   exit 1
[Agent] starts a subagent with the task "Read `/work/repo/.stewards/briefs/review-1-3f9a1c2e.md` and follow it."
  └ subagent: reads the hunks, runs `python -m unittest` in api, writes the answer,
    runs `stewards submit --role reviewer --file ...`, replies with what it printed
> stewards review --task "add a count endpoint to api"
{ reported: [ { file: "review-1-3f9a1c2e.result.json", role: reviewer, unit: null,
                 summary: "reviewer: api changes (1 finding; card: +interface count-items; 1 scenario upserted; not stamped)",
                 units: [ { unit: "api", verdict: "changes", stamped: false, check_reported: true } ],
                 findings: [ { unit: "api", file: "api/store.py", line: 41,
                               cut: "the hand-rolled loop", replace_with: "len()",
                               answer: "api: api/store.py:41: " } ],
                 notes: [], other: {} } ],
  next: null, then: "Fix the findings, then run stewards review again. A finding you will not apply: run stewards review --answer \"<its answer prefix><your reason>\" instead; ..." }   exit 1
[Agent] fixes api/store.py:41 itself
> stewards review --task "add a count endpoint to api"
{ reported: [], next: { jobs: [ { role: reviewer, reasons: ["api: stale"], handoff: "Read `...` and follow it." } ] }, then: "Start one subagent ..." }   exit 1
[Agent] subagent with the handoff: "reviewer: api approve (card unchanged; stamped 8b1e044)"   # sets the tester and documenter marks too
> stewards review --task "add a count endpoint to api"
{ reported: [ { role: reviewer, units: [ { unit: "api", verdict: "approve", stamped: true, check_reported: true } ], ... } ],
  next: { jobs: [], architect: false, flow_checks: [], plans: [],
          check: { failures: [], warnings: [...] }, unowned_changed: [] },
  then: "done" }   exit 0
```

The main agent never reads a brief or a card; every handoff is one
sentence it passes unchanged; it makes the change and the fix itself, and
one subagent per review brief, all of a round started together, reviews
every unit the change touched; done is a
`review` that exits 0.

## Handoff (for a new session)

Read this section, then the Build order, then start.

**State (2026-11-16, the 0.1.0 plan's last task).** Every stage is built, and
the plan's ten tasks are on the branch this text ships with (Build order):
the allowlisted runner, tree-sitter anchors for the eleven static grammars
and runtime grammars, stored plans with step and flow checks, `validate`
(`--report`, `--export`), `next --pr`, `review --pr`, `init --ci`, the
report-only eval graders, completions, the man page, the release workflow,
the install script, the AUR package, and `RELEASING.md`. `render`, hooks, and
the plugin shell are cut. Stewards is adopted on this repository: twelve
units (the table under "stewards' own units"), their cards and test cards,
`docs/units/`, and the `AGENTS.md` block are committed, written through the
tool by role subagents. Repo `wromansky/stewards` (public since 2026-11-12),
auto-delete of merged branches on.

**Next action.** Release 0.1.0 by `RELEASING.md`, in its order: the one-time
repository setup, then each release's steps. Before the tag, run `stewards review` until it exits 0. Then
the manual eval pass on the `fixture` and `self` suites, the history scrub,
the tag, and the release workflow. The task list of the 0.1.0 plan
(`docs/build/stage3/PLAN.md`) is the record of what was built and why;
Build cycle says how waves run.

**What the thin eval showed (2026-11-12).** Four passes, one run per arm
per task, Pi on `anthropic/claude-sonnet-5-5` at medium thinking: three on
the fixture (tasks 1, 3, 4, 6) and one on this repository's `self` suite
(tasks 1 to 4). `full`/`bare` ratios of median main-session tokens (equal
to total tokens: no `full` task session started a subagent), then of cost:

| pass | task 1 | task 3 | task 4 | task 6 | cost, tasks 1/3/4/6 |
|---|---|---|---|---|---|
| fixture 1 | 1.14 | 1.36 | 1.17 | 1.39 | 1.02, 1.23, 1.13, 0.89 |
| fixture 2 | 1.14 | 1.87 | 1.23 | 1.60 | 1.14, 1.52, 1.24, 1.23 |
| fixture 3 | 1.25 | 1.32 | 1.34 | 1.56 | 1.25, 1.54, 1.40, 1.16 |

| pass | task 1 | task 2 | task 3 | task 4 | cost, tasks 1/2/3/4 |
|---|---|---|---|---|---|
| self 1 | 1.04 | 1.04 | 1.50 | 1.04 | 0.82, 1.19, 1.26, 0.92 |

- No pass saved tokens on any task: `full` ran 4% to 87% over `bare`.
- An agent following the block ran the cheap steps (`status`, `route`,
  `next`) and then did not dispatch the review jobs, because a small change
  listed 7 to 13 of them (maintainer, tester, and documenter per unit, plus
  the architect). On the change tasks no `full` run showed more than 3 of
  the 6 ritual steps.
- Asked directly, as in adoption, the same roles worked and found real
  bugs: after fixture pass 1's adoption the cards held four of the five
  seeded bugs as gaps.
- Decision (the user's): the per-change ritual is one review job (the
  reviewer); the per-role jobs stay for adoption and on request (`next
  --roles`); the token-saving goal is dropped in favour of reviewed changes
  and a repository memory at a small, measured overhead (Goals).

**What dogfooding stage 2 showed (2026-11-12).** About eighty role jobs
ran on this repository.
- The loop works from a one-sentence handoff: architect, first reviews,
  documenters, review rounds on real diffs, a finding turned into a
  developer fix, a tester's bug judged by a validator.
- It caught real defects in changes the build had already merged: the docs
  escaping had dropped `$`, and `origin_url` misreads an empty remote URL.
- `init` proposes one unit for a crate whose modules sit flat in `src/`;
  the units here were drawn by hand from the table below.
- On the eval fixture too, `init` proposes only manifest roots: one unit,
  `cli`, with `api/` and `client/` unowned. The eval's `full` arm sets its
  units by hand from `evals/full-units.toml`.
- Triggers are coarse. A tester and a documenter are due whenever a file
  holding an interface anchor changes, and maintainers record most public
  functions as interfaces, so a small edit in each of eight units cost 33
  role jobs over two rounds. Changed: they now trigger only when a hunk
  touches an interface's own lines (see next).
- The architect job returns every round in which two stamped units
  changed, until plans exist (stage 3). Changed: an open plan naming every
  moved unit now covers them (see next).
- A model learns a length cap only by being rejected; the caps belong in
  the schema text.
- Two roles can record one bug under two titles.
- A fix task's lane is the files the findings name, which leaves out the
  test file a finding asks for; the developer's out-of-lane write is
  reported, as designed.
- About one job in fifty did the work and never ran `submit`.

**What the one-job eval showed, and the review decision (2026-11-13).**
One pass each on the fixture (tasks 1, 3, 4, 6) and the `self` suite
(tasks 1 to 4), one run per arm per task, Pi on
`anthropic/claude-sonnet-5-5` at medium thinking, with one review job per
change (`evals/results/2026-11-13T04-15-23Z` and `2026-11-13T04-22-20Z`):

- The gate passed in 0 of the 4 edit-task `full` runs (fixture 1 and 3,
  self 1 and 3), and no `full` run of any task ended with a reviewer
  `submit`.
- Two runs saw the job and did not dispatch it. One dispatched it with its
  own handoff sentence ("...return a short summary of findings"), so the
  reviewer answered in prose and never ran `submit`, and the brief had no
  description of the request because `next` ran without `--task`. One ran
  `next` in parallel with its edit. Only fixture task 1's `full` run
  started a subagent.
- Overhead medians, `full`/`bare`: fixture main tokens 1.26, total 1.26,
  cost 1.23, peak main context 1.28; self main 1.43, total 1.43, cost
  1.20, peak main context 1.17. Fixture task 1's dispatched review took
  its `full` run to 113,730 total tokens against `bare`'s 24,797 (4.59).
- Across five passes agents ran every `stewards` command the block names
  and did not reliably delegate.
- A tool-run launcher (`stewards review` starting each harness's own
  headless session from a launch line per harness) was built the same day
  and withdrawn the same day: it tied the tool to the harnesses it knew.
  The non-delegation result comes from one harness, Pi, whose subagent
  extension tells the model to delegate only when the operator asked; it
  is not yet checked on a second harness.
- Decision (the user's): roles always run as the harness's own subagents,
  every role the same way, and the only contract with a harness is that
  its agent can start a subagent and give it one sentence. The tool prints
  that sentence, the handoff, for every brief; `stewards review` collects
  what the last round submitted, lists the jobs with their handoffs, and
  says in one line what to do next; the block states that it is the
  operator's instruction to delegate the reviews, every time.

**What the 2026-11-14 eval showed, and three review-loop decisions.**
Eight passes, one run per arm per task, thinking off, on weak models:
the fixture and `self` suites, edit tasks 1 and 3 each, on three
harnesses (`evals/results/2026-11-14T11-58-12Z` through
`2026-11-14T15-45-16Z`). The gate per harness and model, `full` runs
passed of the edit-task runs:

| harness | model | fixture (tasks 1, 3) | self (tasks 1, 3) |
|---|---|---|---|
| Claude Code | `claude-haiku-5-5` | 2/2 | 0/2 |
| Hermes | `deepseek/deepseek-flash` | 1/2 | 0/2 |
| Pi | `anthropic/claude-haiku-5-5` | 1/2 | 1/2 |
| Pi | `deepseek/deepseek-flash` | 2/2 | 1/2 |

- 8 of 16 edit-task `full` runs passed. Agents now start the review
  almost every time: of the 8 that failed, one never ran `stewards review`
  (Pi, Haiku, self task 1), one started no subagent with a handoff
  (Hermes, fixture task 3), and 6 started the loop and quit before
  `stewards next --no-brief` exited 0. (The Claude Code fixture pass also
  ran task 6, which was incomplete: two children of its task-5 setup run
  stopped with `error`.)
- A multi-unit change took one round per unit: the review pack's budget
  deferred the rest to a "next round", and the loop outlasted the agent. A
  finding that was wrong, or asked to remove deliberate behaviour, left
  the loop stuck. Each small finding cost a full extra review.
- Decisions (the user's): every due unit is reviewed in one round, in as
  many briefs as the 48 KB budget needs, listed together; the agent may
  answer a finding with a reason (`review --answer`) that the next
  reviewer accepts or contests once, never dismissing a finding unseen; a
  finding carries `blocking`, and a non-blocking one is a note that never
  holds up approval.

**Deferred, known:** workspace loads every listed repo before applying
`--repo`, so a broken sibling blocks `--repo good`; the harness table has no
`parallel` field; an anchor's symlink check runs before the read, not with
it; the lost-update check and the write are not one atomic step; briefs for
jobs no longer due stay on disk until overwritten.

**How we work.**
- Clean room: no text from any earlier private project, ever.
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
role), so a unit's bugs are judged one per round. Deferred ideas (a ZeroClaw renderer went with
`render`, cut from 0.1.0): a documented one-time Borg `manage_roles` setup.

## Facts

### Harness table (the contract, checked per harness; last verified 2026-10-21)

| harness | role-file format | spawn tool / fresh child | model pin | parallel | env detection |
|---|---|---|---|---|---|
| Claude Code | `.claude/agents/*.md`, `~/.claude/agents/` | Agent tool | per file | yes (20) | `CLAUDE_CODE` |
| Cursor | reads `.claude/agents/`, `.codex/agents/`, `.cursor/agents/` | Task tool | per file | yes | UNVERIFIED |
| Codex CLI | `.codex/agents/*.toml`, `~/.codex/agents/` | spawn tool | per file | yes | `CODEX_*` |
| Copilot | `.github/agents/*.agent.md` (repo-level only) | `runSubagent` | per file | yes | UNVERIFIED |
| Gemini CLI | `.gemini/agents/*.md`, `~/.gemini/agents/` | `@name` | per file | yes | UNVERIFIED |
| OpenCode | `.opencode/agents/*.md`, `~/.config/opencode/agents/` | Task tool | per file | yes | UNVERIFIED |
| Pi + pi-subagents | `.pi/agents/*.md`, `~/.pi/agent/agents/` | `subagent` tool | per file | yes | `PI_*` |
| Hermes | none (briefed) | `delegate_task` | no | yes (10, background) | `HERMES_*` |
| ZeroClaw | none (briefed) | `spawn_subagent`, `delegate` | per agent alias | `delegate` only | UNVERIFIED |
| Borg | none (briefed; roles in its database) | `spawn_agent` | per call | yes (3) | UNVERIFIED |

`guide` prints the same generic delegation paragraph for every harness
(see `guide` under Agent-facing contracts), in or out of this table, the
latter the normal case rather than an error. No
harness gets a template sentence of its own, extra arguments, or a step
that depends on a role file. Facts for the agent's own judgment, not
printed: Hermes children run in the background by default (wait for the
completion message); ZeroClaw's `spawn_subagent` blocks and needs a risk
profile that allows it; `borg ask` defaults to the daemon's cwd. The
table holds no launch line: the tool never starts a session.

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
- Hermes (Python; source read 2026-10-21):
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
- ZeroClaw (Rust; source read 2026-10-21):
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
- Borg (checked 2026-10-21): a daemon with a `borg` CLI. Headless:
  `borg ask "<msg>" --json`, `borg acp`. Reads `AGENTS.md` then `CLAUDE.md`
  per directory from the session workspace up to the git root, 32 KiB total
  with truncation. Subagents through `spawn_agent` (fresh context by
  default, built-in role `coder` has shell and patch tools), depth 1, 3
  concurrent; custom roles live in its database, with no file format. Shell
  runs in the session workspace behind a fixed deny list. `borg ask` has no
  cwd flag: the workspace is the daemon's cwd unless `-w <worktree>` or
  `borg acp`. Does not read `.claude/agents/`.
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
- tree-sitter crates (2026-11-15): `tree-sitter` 0.27.2 with
  `tree-sitter-rust` 0.24.2, `-python` 0.25.0, `-typescript` 0.23.2,
  `-javascript` 0.25.0, `-go` 0.25.0, `-bash` 0.25.1, `-c` 0.24.2, `-cpp`
  0.23.4, `-c-sharp` 0.23.5, `-hcl` 1.1.0, `-yaml` 0.7.2 build together
  (each grammar hands over a `tree-sitter-language` 0.1.9 `LanguageFn`, so
  one ABI); they are dependencies because neither std nor the tree has a
  parser for these languages. No `tree-sitter-baml`; `tree-sitter-loader` compiles
  grammars from source at runtime (needs a C compiler); prebuilt grammar
  libraries load with `libloading` 0.8.9 (already in the tree through
  `baml-sys`); the runtime loader takes the `LanguageFn` from
  `tree-sitter-language` 0.1.9, also already locked.
- Drivers (verified 2026-10-20 on claude 2.1.305 and codex-cli 0.154.0; Pi
  and Claude Code 1.0.128 as dated below): Claude Code 2.1.305 `claude -p
  "<prompt>" --output-format stream-json
  --verbose --model <m> --max-turns N --max-budget-usd X
  --no-session-persistence` (final `result` line carries `usage`,
  `modelUsage`, `total_cost_usd`, `subagent_stats`; `--bare` skips hooks,
  CLAUDE.md, plugins). Claude Code 1.0.128 (verified 2026-11-13 on
  `claude-haiku-5-5`, the id `--model` takes; aliases not tried, since they
  name other models): the eval runs `claude -p --output-format stream-json
  --verbose --dangerously-skip-permissions [--model <id>] -- "<prompt>"`,
  stdin closed, in the session's cwd (no cwd flag), with
  `CLAUDE_CONFIG_DIR` set to the eval's config directory and
  `CLAUDE_CODE_SUBAGENT_MODEL` to the session's model. This version has no
  `--bare`, no `--no-session-persistence`, and no thinking flag (a budget
  only, from `MAX_THINKING_TOKENS` or prompt words). It does not read a
  project `AGENTS.md` (no such name in its code; a session with one ignored
  it); a `CLAUDE.md` holding `@AGENTS.md` imports it. Its subagent tool is
  `Task`, input `{description, prompt, subagent_type}`, the child's task in
  `prompt` (2.1.305's `Agent`, which `src/harness.rs` names). A
  `general-purpose` child runs on the built-in default model
  (`claude-sonnet-4-20250514`, retired: 404) unless
  `CLAUDE_CODE_SUBAGENT_MODEL` is set; background title requests go to
  `claude-3-5-haiku-20241022` and also 404, with no tokens or cost.
  `CLAUDE_CONFIG_DIR` is honoured: a directory holding only
  `settings.json` and a linked `.credentials.json` authenticates (OAuth),
  with no MCP servers and only the built-in agents. The stream emits one
  `assistant` event per content block with partial usage, streams a
  child's events with `parent_tool_use_id` but not its final message, and
  ends with a `result` line: `usage` the parent's alone, `modelUsage` the
  parent's plus the children's, `total_cost_usd` both at its price table,
  which prices `claude-haiku-5-5` at Sonnet's rates. The persisted session
  file (`$CLAUDE_CONFIG_DIR/projects/<cwd slug>/<session id>.jsonl`) holds
  every record with `cwd`, `timestamp`, `uuid`/`parentUuid`, children as
  `isSidechain: true`, each response once per content block with its usage
  so far; `stop_reason` is null in every record and an API failure is an
  assistant record with `isApiErrorMessage: true` and model
  `<synthetic>`; a response's records carry its final input and cache
  counts but not always its final output count, which only the stream's
  `result` line has. An enclosing session gives its shell `CLAUDECODE=1` and
  `CLAUDE_CODE_ENTRYPOINT` (and `CLAUDE_CONFIG_DIR` when set); this version
  never reads `CLAUDECODE`, keeps an inherited `CLAUDE_CODE_ENTRYPOINT`
  (a label in its user agent and telemetry only), and honours
  `CLAUDE_CONFIG_DIR`, which the eval's isolation relies on. Its `Bash`
  commands run detached, each in its own process group; its subagents run
  inside its own process. Codex `codex exec --json --skip-git-repo-check -C
  <dir> -s workspace-write -m <model> "<prompt>"` (`turn.completed{usage}`
  summed across turns; rollout under `~/.codex/sessions/`); this machine's
  ChatGPT-account Codex rejects the configured model ids with 400. Pi
  (verified 2026-11-12 on `@earendil-works/pi-coding-agent` 1.5.2 with
  pi-subagents 0.88.0): `pi --mode json --session-dir <d> --exclude-tools
  subagents_enable --thinking <level> [--model <provider/id>] --
  "<prompt>"`, stdin closed, `PI_CODING_AGENT_DIR` set to the eval's agent
  directory (checked 2026-11-12 on `anthropic/claude-haiku-5-5`: it
  authenticates, the first request's tools are `read`, `bash`, `edit`,
  `write`, `subagent`, `contact_supervisor`, `bg_wait`,
  `subagent_supervisor`, its skills only pi-subagents' `council-mode` and
  `pi-subagents`, and no context file; Pi always scans `~/.agents/skills`,
  so the eval's settings exclude it);
  no cwd flag, the process cwd is the session's; AGENTS.md in the cwd is
  loaded. JSON events go to stdout, but the transcript is the session file
  `<d>/<timestamp>_<id>.jsonl` (`session` header with `cwd`, `model_change`,
  `message` records with per-response `usage {input, output, cacheRead,
  cacheWrite, cost.total}` and `stopReason`, `usage` records for cache
  refreshes). In `--mode json` Pi exits 0 even when the final response
  stopped with `error`, `aborted`, or `length` (only text mode sets exit
  1), so the runner reads the stop reason from the transcript. Subagent children are
  `<d>/<timestamp>_<id>/<runId>/run-0/session.jsonl` with `parentSession`,
  forks `<d>/<timestamp>_<id>/forks/<id>/*.jsonl` (a copy of the parent's
  records, same ids and timestamps, then the fork's own);
  `<d>/subagent-artifacts/` holds copies, not sessions. Excluding
  `subagents_enable` keeps `subagent` active from the first request. In
  non-interactive mode subagents launch async and Pi waits for them
  before exiting. A `PI_*` environment from an enclosing Pi session
  (`PI_SUBAGENT_CHILD`) makes a nested `pi` start without `subagent`, so
  the runner strips every harness's env prefix, then sets
  `PI_CODING_AGENT_DIR` itself. Hermes Agent (verified 2026-11-13 on
  v0.23.1, 2026.9.24, source `3b42157858`, with `deepseek/deepseek-flash`;
  five short runs, one with a subagent; on 2026-11-14 the one-shot child
  cap, compression rows, `delegate_task` shapes, refusals, log rotation,
  and `workdir` re-read from that source, and three more runs: a `bare`
  smoke run, and two one-shot sessions in a repo whose `AGENTS.md` set a
  codeword, told to start three children in three calls: both answered
  with the codeword (the main row's exported `system_prompt` holds the
  `AGENTS.md` under "Project Context"), with `oneshot_max_children: 0`
  all three children ran, at the default the third call returned the
  budget error and started nothing (recorded as the `refused` fixture),
  and in both the model filled each task's `context` unasked): the eval runs `hermes chat
  --query=<prompt> --oneshot --yolo --in <dir> --format stream-json
  --reasoning <level> --provider <p> -m <m>`, stdin closed, with
  `HERMES_HOME` the session's own home. `hermes chat` is only a client:
  it starts (`start_new_session`, cwd the Hermes source tree) or attaches
  to a gateway daemon for that home (`gateway run --idle-exit`, idle for
  600 s before it exits), and the agent runs in the daemon; killing the
  client leaves the turn running ("accepted work continues at the
  gateway"), and a `process_supervisor.sh` reaps the daemon's registered
  process groups when the daemon dies. `<home>/gateway.pid` (JSON, `pid`)
  names the daemon. `--in` sets the session's cwd: `pwd` printed it and a
  relative `write_file` landed there. The stream holds `system/init`,
  `tool_use`/`tool_result` and `text` events of the main session only,
  then a `result` line with `exit_code`, `error` (on failure), and
  `tokens {input (cache reads included), output, total, cache_read,
  cache_write}` of the main session alone; `Session: <id>` goes to
  stderr. `hermes sessions export --format jsonl --yes <file>` writes one
  line per session of the home (main and children, `parent_session_id`,
  per-session counts, `model`, `billing_provider`, `estimated_cost_usd`,
  `cwd`, and `messages` with `tool_calls[].function.{name, arguments (a
  JSON string)}`, tool results as JSON strings, `finish_reason`,
  `reasoning`/`reasoning_content`); it works after the daemon is killed
  and starts none. Per-response usage is only in `logs/agent.log`
  (`[<session id>] … API call #N: … in=<prompt> out=…`). A new home builds
  `installs/` (the dependency environment, 424 MB, minutes) on its first
  command, and a home linking another's `installs/` builds nothing; a
  fresh home seeds Hermes's default SOUL.md, its bundled skills (listed in
  the system prompt), and enables its bundled plugins. The project
  `AGENTS.md` loads into the system prompt ("# Project Context") without
  `--ignore-rules`, which skips it along with SOUL.md, memory, and
  preloaded skills; a "Workspace" block also lists the last commits.
  `delegate_task` takes `tasks[] {goal, context, output_schema, images}`
  (`tasks` may also be the JSON string of that array; a legacy top-level
  `goal`/`context`/`output_schema` is read only when `tasks` is absent or
  empty); a child's first message is
  its `goal`, its `context` goes into its system prompt (after
  `CONTEXT:`, with an output contract appended for an `output_schema`)
  with the repo's `AGENTS.md`. A finite one-shot session (`hermes chat
  -q`/`--oneshot`, `hermes -z`) may start `delegation.oneshot_max_children`
  subagents in all, 2 by default (0 is unlimited); past it `delegate_task`
  returns `{"error": "Delegation budget for this one-shot run is exhausted
  ... reviewing your own diff and running the tests inline is expected
  here, not a delegated review."}` and starts nothing. Interactive
  sessions and messaging-gateway sessions are never charged
  (`_oneshot_spawn_budget`); a one-shot turn the local gateway daemon runs
  is, since its launch policy's source is `oneshot`.
  `delegation.max_spawn_depth` is 1 by default: children cannot delegate.
  A task its backend cannot route is refused alone, its result entry
  `status: error`, `api_calls: 0`, `route.refused: true`. In a one-shot session the call runs its children
  synchronously ("async delivery unsupported on this session runtime"),
  so the session waits for them, and returns their summaries, statuses
  (`completed`, `failed`, `error`, `timeout`, `interrupted`), and token
  counts. Children inherit the parent's reasoning level (their session's
  `model_config.reasoning_config`); the main session's level is only in
  the gateway's own policy. `--reasoning none` and `low` left the main
  session's `reasoning_content` a lone space with 0 reasoning tokens; the
  default left reasoning text. Hermes has no prompt words that set
  thinking. Children auto-deny "dangerous" shell commands unless
  `delegation.subagent_auto_approve: true`. Tools: `terminal {command}`,
  `read_file {path}`, `search_files {pattern, target content|files, path}`
  (`files` lists paths, `.git` internals included), `write_file`,
  `patch`, `execute_code`; `terminal` also takes `workdir`, and
  `search_files` maps `target: grep` to `content` and `find` to `files`.
  An `execute_code` script can call `terminal()` itself; those commands
  are not `terminal` calls in the export. `read_file` refuses `.env`
  files ("not a security boundary; the terminal tool can still bypass").
  Compressing a session's context saves the rest of the conversation as a
  new session row whose `parent_session_id` is the old row and whose
  `source` is the old row's (`agent/conversation_compression.py`), at 75%
  of the window for windows under 512K; token counts are written as
  per-call deltas to the row in use. `logs/agent.log` rotates at 5 MB,
  three backups (`agent.log.1`..`.3`). The client's start of the gateway
  daemon precedes the turn (8 to 10 s in the smoke runs). Hermes sets `AI_AGENT=hermes-agent` and
  `HERMES_AGENT=true` only when unset, and `HERMES_*` variables; the
  daemon inherits the client's environment (its shell commands get it
  minus a secret blocklist), and the client's `PATH` reaches them with
  Hermes's tool directories prepended. Its price table has
  `deepseek-flash` at $0.15 / $0.60 / $0.003 per million input, output,
  and cache-read tokens, DeepSeek's off-peak rates (models.dev agrees),
  applied in every hour.
- `claude plugin eval` exists (cases under `evals/<case>/`, six fixed grader
  types, with/without ablation, `--json`) but has no custom-code graders and
  starts in an empty workspace; stewards uses its own runner.

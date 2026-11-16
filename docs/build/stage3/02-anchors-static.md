# Task 02: tree-sitter anchors, static grammars

Repo: stewards (Rust), branch off `main`. Follow `docs/build/common.md`.
Model: Sonnet 5.5, thinking high.

Read DESIGN.md: Knowledge › Anchors (line ~365), Schemas › "Anchor grammar", Rules › Pack ("Source
windows …" and the shrink order), Rules › next (the "interface's lines" rule and its
`simplified` limits), Trust and safety › Git and file reads, Facts › "tree-sitter crates", and the
`[[decision]]` "Tester and documenter jobs trigger on an interface's lines" in `.stewards/repo.toml`.
Then `src/anchors.rs`, every caller (`check.rs`, `pack.rs` `window`/`halve`, `submit.rs`
`anchor_alive` and the architect flow entry, `docs.rs`, `next.rs` `interfaces_touched` and
`BLOCK_LANGUAGES`), `tests/anchors.rs`, `tests/next.rs` (interface-line tests).

## Why
DESIGN.md: "With tree-sitter (stage 3), `name` must be a definition per the grammar's `tags.scm`; a
mere mention is dead." Today any whole-word mention keeps an anchor alive, source windows are ±20
lines, and the interface-line rule is a blank-line heuristic with known misses. Exact definitions
and spans make anchors honest and tester/documenter triggers precise.

## Build
1. **Dependencies.** `tree-sitter` plus the grammar crates for rust, python, typescript (TS and TSX),
   javascript, go, bash, c, cpp, c-sharp, hcl, yaml, at versions that share one ABI. Pick the newest
   set that builds together (`cargo build --offline` first; fetch if needed). One line per crate in
   Cargo.toml; the dependency reason (no parser in std or the tree) goes in the Facts update. No
   other new dependency.
2. **Languages by extension.** `rs`; `py`; `ts mts cts`; `tsx`; `js jsx mjs cjs`; `go`; `sh bash`;
   `c h` (DECIDED BY PLANNER: `.h` is C); `cc cpp cxx hpp hh`; `cs`; `tf hcl`; `yml yaml`. Anything
   else keeps the text rule (BAML, Markdown, TOML, Java, Ruby, …).
3. **Definitions.** Each grammar's own tags query (`TAGS_QUERY` where the crate ships one; else the
   grammar repository's `queries/tags.scm` copied into `src/anchors/<lang>.scm` with its license
   header): every `@name` capture inside a `@definition.*` capture is a definition name with that
   node's span. Custom queries in `src/anchors/`: YAML mapping keys, HCL blocks (name = the block's
   labels joined with `.`, so `resource "aws_s3_bucket" "logs"` is `aws_s3_bucket.logs` and
   `variable "region"` is `region`), Bash functions. DECIDED BY PLANNER: a Rust supplement adds
   `const_item`, `static_item`, `enum_variant`, and struct `field_declaration` (Rust's tags.scm omits
   them, and cards anchor such items, e.g. `src/harness.rs::EnvRule::Unverified`).
4. **Resolution** (`resolve`, same signature and `Resolution`): for a grammar language, `Found` when
   `name` is a definition. A qualified name (`a::b`, `a.b`, as the text rule splits it): DECIDED BY
   PLANNER: the whole name matches first (HCL, YAML keys with dots); otherwise the last segment must be
   a definition whose span lies inside a container named by the segment before it: a definition, or
   for Rust an `impl_item` whose type is that name (so `Router::owner` is alive, `Other::owner`
   dead), for YAML the parent key. Only the immediate qualifier is checked. `line` is the definition
   name's line. The plain-path, no-symlink, in-snapshot rules are unchanged (they live in `find`).
5. **Spans** (`source_window`): for a grammar language, the definition's node span, extended up over
   directly preceding comment and attribute siblings, capped at 41 lines (2×20+1) then one line
   `… (<n> more lines)` (DECIDED BY PLANNER: keeps packs bounded as today). Text languages keep ±20
   lines. `pack.rs` `halve` must keep the match line and halve the line count for both kinds; if it
   needs the window's first line, add it to `PackAnchor` and to the Pack line in DESIGN.md.
6. **Interface lines** (`interface_lines`, now also given the path): for a grammar language, the
   lines of every span (step 5, uncapped) of a definition named `name`; a mention gives none. Other
   languages: today's text rule, only for `BLOCK_LANGUAGES`; `next.rs` `interfaces_touched` asks
   anchors whether lines can be found instead of checking `BLOCK_LANGUAGES` itself. The fallback
   (`interface file changed`) is unchanged.
7. **Dead anchors make a unit due.** DECIDED BY PLANNER (the switch kills mention anchors in code that
   did not change, and today nothing then lists the unit, so `review` could never exit 0): in
   `next.rs` `role_jobs`, a stamped unit whose card has an anchor `check` would call dead gets a
   maintainer job with reason `dead anchor: <path::name>` even when its fingerprint is unchanged, so
   plain `next` folds it into the reviewer round and the reviewer's card patch fixes it. Verify the
   reviewer pack for a unit with no hunks renders and says why it is due.
8. **DESIGN.md**: Anchors, Pack "Source windows", the next interface-lines rule (drop the text
   rule's `simplified` caveats for grammar languages; keep them for the rest), the dead-anchor
   reason in next's job list, Facts tree-sitter line (versions, date). Remove the `simplified:`
   comment on `interface_lines` only for what tree-sitter now covers.

## Tests (no network)
- per static language, a small source file: a definition is `Found` at its line, a mention of the
  same name in a comment or a call is `DeadName`;
- Rust `Type::method` alive, `Other::method` dead, `Enum::Variant` alive, a `const` alive;
- YAML nested key `a.b`; HCL `aws_s3_bucket.logs` and a variable; Bash function; a `.baml` and a
  `.md` file still resolve by text;
- spans: a Python function whose docstring has a blank line then a column-0 line, and a Rust
  function with a raw string holding a lone `}` then a blank line, are each one span (the text
  rule's known misses); the doc comment above is included; a 200-line function is cut at 41 lines;
- `next`: an edit inside that Python docstring gives `interface changed`; an edit to a line that
  only mentions the name gives no tester or documenter job;
- `next`: a stamped, unchanged unit whose card anchors a mention lists a reviewer job with reason
  `dead anchor: …`; after a card fix and stamp, nothing is due;
- pack: halving keeps the match line for a span and for a text window.
Also run, read-only, `cargo run -- check` in this repository before and after, and report every
anchor of this repo's cards that dies (expected at least `src/init.rs::not_implemented`,
`src/guide.rs::BLOCK.split`). Do not fix the cards. Report `cargo build --release` time and binary
size before and after.

## Files
Own: `src/anchors.rs`, `src/anchors/*.scm` (new), `src/pack.rs` (`window`, `halve`, `PackAnchor`
only), `src/next.rs` (`interfaces_touched`, `BLOCK_LANGUAGES`, the dead-anchor reason),
`tests/anchors.rs`, `tests/pack.rs`, `tests/next.rs`; shared append: Cargo.toml, DESIGN.md sections
above. Do not touch `check.rs`, `submit.rs`, `docs.rs` (they keep calling `resolve`), runtime
grammar loading (task 05), `.stewards/`. New files: `src/anchors/*.scm`; unit `analysis` needs the
path `src/anchors/**`.

# Task 05: runtime-loaded tree-sitter grammars

Repo: stewards (Rust), branch off `main` after task 02 is merged. Follow
`docs/build/common.md`. Model: Sonnet 5.5, thinking high. Security
review of the loading code before merge.

Read DESIGN.md: Knowledge › Anchors ("runtime extras from `~/.stewards/grammars/<lang>/` via
`libloading`"), Schemas › user-level files (`$STEWARDS_HOME`, `grammars/<lang>/{lib.so|.dylib,
tags.scm}`), Rules › check, Facts › tree-sitter crates. Then `src/anchors.rs` and `src/anchors/`
as task 02 left them, `src/user.rs` (how `$STEWARDS_HOME` / `~/.stewards` is found), `src/check.rs`
(warnings).

## Why
The static set covers eleven languages. A user with Lua, Ruby, or Elixir code gets exact
definitions by dropping a compiled grammar and its `tags.scm` into their own stewards directory,
without a new stewards build.

## Build
1. **Layout.** `<stewards home>/grammars/<lang>/` holds `lib.so` (Linux) or `lib.dylib` (macOS)
   and `tags.scm`. DECIDED BY PLANNER (the spec does not map files to grammars): an optional
   `extensions` file lists file extensions, one per line; without it the directory name is the one
   extension. The library exports `tree_sitter_<lang>` (`-` in the name read as `_`). A static
   grammar wins over a runtime one for the same extension. Grammars are read only from the user's
   stewards home, never from a repository (a repo-supplied library would run code from a clone);
   say so in Trust and safety.
2. **Loading.** `libloading` (already in the tree through `baml-sys`; add it as a direct dependency at
   the locked version, so nothing new compiles). Load once per process, lazily, on the first file
   with that extension. Refuse, with a reason, a library whose language ABI is outside the range the
   linked `tree-sitter` accepts (`Language::abi_version` against `LANGUAGE_VERSION` /
   `MIN_COMPATIBLE_LANGUAGE_VERSION`), a missing symbol, a `tags.scm` that does not compile for that
   language, or a `lib.*` or `tags.scm` that is not a regular file once links are followed (links
   are allowed here: it is the user's own home, not a repository). The library is kept loaded for the process (no unload).
   One `unsafe` block with a `// SAFETY:` comment naming what is trusted: the user put the library
   there.
3. **Use.** A runtime language behaves exactly as a static one in `resolve`, `source_window`, and
   `interface_lines` (definitions from its `tags.scm`, the same qualified-name and span rules). A
   grammar that failed to load leaves that extension on the text rule.
4. **Telling the user.** `check` adds a warning of kind `grammar_unusable` (never a failure) per
   grammar directory that could not be used, naming the directory and the reason, with what to do:
   `rebuild it against tree-sitter ABI <min>..<max>, or remove <dir>`. `next` and `review` carry it
   with the other warnings, as they do today.
5. **DESIGN.md**: Anchors (the layout, the `extensions` file, the precedence, the warning), Rules ›
   check (the new warning), Trust and safety (libraries only from the user's stewards home).

## Tests (no network)
`tests/fixtures/grammars/json/`: the `parser.c` of a small public grammar (DECIDED BY PLANNER:
tree-sitter-json's, MIT; keep its LICENSE beside it) and a `tags.scm` you write that tags object
keys as definitions. The test compiles it with `cc -shared -fPIC` (`.so`; on macOS `-dynamiclib`)
into a temp `STEWARDS_HOME/grammars/json/lib.so`; when `cc` is missing the test fails with a
message saying so (CI and this machine have gcc). Then in a temp repo: a key is `Found`, a string
value with the same text is `DeadName`; an `extensions` file mapping `jsonc`; a library built from
a non-grammar C file (no symbol) and a directory without `tags.scm` each give a
`grammar_unusable` warning from `check` and fall back to text; a static extension (`rs`) is not
overridden by a runtime `rs` directory. Verify on macOS: not possible here; say so.

## Files
Own: `src/anchors.rs` (loading and lookup; a new `src/anchors/runtime.rs` is fine), `src/check.rs`
(the warning only), `src/user.rs` (a path helper only, if one is missing), `tests/anchors.rs`,
`tests/fixtures/grammars/**` (new); shared append: Cargo.toml (one line), DESIGN.md sections
above. Do not touch `next.rs`, `pack.rs`, `model.rs`. New files: `tests/fixtures/grammars/**` →
unit `analysis` (add the path).

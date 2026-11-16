# Task 01: the allowlisted command runner

Repo: stewards (Rust), branch off `main`. Follow `docs/build/common.md`.
Model: Opus 5.5, thinking medium. Security review on the patch before merge.

Read DESIGN.md: Trust and safety (all of it, especially **Execution** at line ~969), Schemas
(`repo.toml` `[commands]`), Agent-facing contracts (error table: `command_not_allowed`,
`command_timeout`), Rules › next (flow checks), Knowledge › Plans, Verification › Validation. Then
`src/submit.rs` (`SHELL_CHARS`, `allowlist`, `check_command` and every caller), `src/model.rs`
(`Commands`, `CommandEntry`), `src/eval/mod.rs` (how it kills a process group, for reference only).

## Why
Stage 3 runs commands for the first time: a bug's repro (`validate --report`, task 06), plan step
checks and flow checks (`next`, task 04). Today only `submit` checks commands, and nothing runs one.
One module must own the allowlist and the execution so every caller gets the same rules.

## Build
New module `src/run.rs` (`pub mod run;` in `src/lib.rs`):

- `pub fn allowed(loaded: &Loaded, unit: Option<&str>) -> Vec<Allowed>`: the repo's `commands.test`
  entries, plus, when `unit` is given, that unit's test card `test_command`. Each carries its
  timeout: an entry's own `timeout_seconds` (`CommandEntry::Timed`), else
  `commands.timeout_seconds`. Moved from `submit::allowlist`, same order, same result.
- The check moved from `submit::check_command`, unchanged in meaning: no `SHELL_CHARS`; the
  command's leading whitespace-split words equal all the words of one entry (`pytest` allows
  `pytest tests/x`, not `pytest-x`). Return the matching entry (for its timeout) or the problem
  text. `submit` keeps producing the same `FieldError`s through it; its tests must pass unchanged.
- `pub fn run(root: &Path, allowed: &[Allowed], cmd: &str) -> Result<RunResult>`: refuses a
  command the check refuses with the CliError `command_not_allowed` (fix text from the error
  table). Otherwise: argv = the whitespace-split words, no shell; cwd = `root`; stdin null;
  inherits the environment; its own process group (`CommandExt::process_group(0)`); stdout and
  stderr read concurrently (two threads, so a full pipe cannot deadlock) into one buffer; on the
  timeout, SIGKILL to the whole group (`libc::kill(-pgid, SIGKILL)`, `libc` is already a
  dependency), then reap.
- `RunResult { command, exit_code: Option<i32>, pass: bool, timed_out: bool, output: String }`
  (Serialize). `pass` = exited 0 and not timed out. `output` = the last 4,000 characters of the
  combined output, lossy UTF-8, control characters other than `\n` and `\t` replaced (reuse what
  `check.rs` has for invisible characters if it fits).
  DECIDED BY PLANNER: a timeout is a result (`timed_out: true`, `pass: false`, `output` ending with
  one line `command_timeout: exceeded <n>s; raise timeout_seconds or fix the test`), never an
  error, so a caller can report it beside other results. A command that cannot start (program not
  found) is also a result: `exit_code: None`, `pass: false`, output naming the program.
- DECIDED BY PLANNER: `commands.lint` stays out of the allowlist. Nothing in the tool runs lint;
  update DESIGN.md Execution to say `commands.test` entries (and the unit's own `test_command` for
  its own jobs), which is what the code already does.
- OQ 4 (PLAN.md), until answered: keep the prefix rule. Add one sentence to DESIGN.md Trust and
  safety › Execution: the words after an allowed prefix are not checked, so an agent-written repro
  or check can pass flags the test runner acts on; CI runs `check`, which runs nothing.

No caller runs commands yet; tasks 04 and 06 call `run::run`. Keep the module small: no builder,
no options beyond these.

## Tests (`tests/run.rs`, no network, no model)
In a temp git repo with `commands.test = ["sh"]`, a script under the repo:
- an allowed command that exits 0 → `pass`, its output captured; one that exits 3 → `pass: false`,
  `exit_code: Some(3)`;
- `pytest-x` against `pytest`, a command with `;` or a quote, and an empty command → `command_not_allowed`;
- a unit's own `test_command` is allowed with that unit and refused with another unit or none;
- `{ cmd = "sh", timeout_seconds = 1 }` with a script that starts a background `sleep 30` (writing its
  pid to a file) then sleeps: `timed_out`, returns within a few seconds, and the background pid is
  gone (the group was killed);
- output over 4,000 characters keeps the tail; output of more than 64 KiB on both streams does not hang;
- a missing program → a result, not an error.
`submit`'s existing allowlist tests pass unchanged.

## Files
Own: `src/run.rs` (new), `src/submit.rs` (only the moved `allowlist`/`check_command` and their
callers), `tests/run.rs` (new); shared append: `src/lib.rs` (one line), DESIGN.md Trust and safety ›
Execution. Do not touch `next.rs`, `validate.rs`, `pack.rs`, `brief.rs`. New files: `src/run.rs`,
`tests/run.rs`; suggested unit `analysis` (or a new `run` unit; the supervisor decides).

## Review: `src/run.rs` (feat/s3-runner), security, read-only

**Verdict: CHANGES.** The only required change is a DESIGN.md wording fix. The runner code itself has no P0 issue. I changed no files and ran nothing.

### Checked and sound
- **(1) Splitting argv.**
  - `check` (`src/run.rs:86`) and `run` (`:114`) split the same `cmd` string with the same `split_whitespace`, so the text that is checked is the text that runs.
  - The match is word by word (`words.starts_with(&entry)`), so `cargo testx`, `pytest-evil`, and `py` against `pytest` are all refused.
  - Entries that are empty or only spaces are skipped (`!entry.is_empty()`). Leading spaces in an entry are harmless.
  - An entry containing shell characters can never match, because the command is rejected for those characters first.
  - The first word always equals an entry's first word, so it cannot be an option chosen by the agent.
  - Unicode whitespace (U+00A0, U+3000, U+0085, U+2028) splits words the same way in the check and the run, so it gives no bypass.
  - A NUL in the text makes `spawn` fail, which is reported as a failed result.
  - An invisible character such as U+200B makes a word no longer equal the entry word, so the command is refused.
- **(2) No shell.** The program is started with `Command::new(argv[0]).args(..)` (`:122-129`), with no shell involved. stdin is null. Rust opens its file descriptors close-on-exec, so the child inherits none of them.
- **(3) Where the allowlist comes from.**
  - Nothing an answer can submit writes `commands` or `test_command`. The architect answer edits only `flow` and `decision` tables in repo.toml, through `toml_edit` (`src/submit.rs:1842-1868`). The tester answer copies `test_command` over unchanged from the loaded card (`:1050-1057`).
  - `load_repo` refuses files reached through a symlink (per DESIGN).
  - A unit's `test_command` is added only when `allowed(…, Some(unit))` is called. Flow checks pass `None` (`submit.rs:1795`).
- **(5) Races.** The check and the run use the same `&str`, and `run` checks it again at run time, so a repo.toml edited after `submit` is respected.
- **(6) Exit status.**
  - A signal death gives `code() == None` and `success() == false`. A timeout gives `pass: false`. A program that cannot start gives `pass: false`.
  - `argv` cannot be empty when we reach the spawn. I found no path where a command that did not run reports a pass.
- **(4) Output handling.**
  - The read buffer stays bounded at 128 KiB or less.
  - The 4,000-character cut counts characters, so it cannot split a multi-byte character.
  - `clean` turns control characters into spaces (C0, DEL and C1, including ESC and U+009B) and escapes bidi and invisible characters. Raw byte 0x9B becomes U+FFFD.
- **(7) Scope.** The diff gives answers no new power. `check_command` behaves exactly as before, and `commands.lint` was never in the allowlist.

### Findings
- **P1: DESIGN.md does not state the accepted limit plainly** (`DESIGN.md:975-977`).
  - It says the agent "can pass flags the test runner acts on" and that "CI runs `check`, which runs nothing". It never says what that means: any allowlisted runner can be made to run any program, with the user's full environment.
  - Attack: `pytest -p evilplugin`, `cargo test --config target.x86_64-unknown-linux-gnu.runner=…`, or `go test -exec …`. Each runs agent-chosen code that can read every secret in the environment. DESIGN itself says tracker auth comes "from CLIs and env" (`:1004`).
  - Smallest fix: replace that sentence with: "The words after an allowed prefix are not checked: a repro or check can pass flags that make the allowed program run any other program, as the user, with the inherited environment (tokens included), and with no sandbox. The allowlist limits which program starts, not what it does."
- **P2: "killed whole" is only true on timeout** (`src/run.rs:161-176`; `DESIGN.md:979-981`).
  - When the leader exits normally, anything it left running in its process group is never killed. Each reader thread waits about 1 s, and the leftover process keeps running with no limit and can change the repo after a pass is reported.
  - Attack: a test that does `sleep 1000 &` or forks a worker. A child that calls `setsid` escapes even the timeout kill (accepted, since arbitrary code is already accepted, but it is not stated).
  - Smallest fix, if wanted: on normal exit, also send `libc::kill(-pgid, SIGKILL)` before reaping. To keep the pgid safe from reuse, poll with `waitid(P_PID, pid, WEXITED|WNOHANG|WNOWAIT)`, kill the group, then call `child.wait()`.
  - Otherwise, change DESIGN to say "killed at the timeout; processes that leave the group, or outlive a normal exit, are not".
- **P2: an error from `try_wait` leaves the child running** (`src/run.rs:162`).
  - `child.try_wait()?` returns early without killing the group, so the command runs with no timeout and the reader threads stay alive.
  - Smallest fix: on `Err`, run `kill(-pgid, SIGKILL)` and `child.wait()`, then return the error.
- **P2: `run::check` does not reject control or invisible characters itself** (`src/run.rs:80`).
  - Today it relies on `submit::check_text` having run first. Commands that tasks 04/06 will read from stored test cards and plans are not checked when loaded (`src/model.rs:239, 277` have no validation). A hand-planted `pytest tests/x\x1b]0;…\x07` would pass the check and be echoed raw in `RunResult.command`. A refused command is also echoed raw in the `command_not_allowed` message (`:98`).
  - Smallest fix: in `check`, reject `c.is_control() || crate::check::invisible(c) || SHELL_CHARS.contains(&c)`. Make sure submit tests that count problems still pass: `check_text` already reports the same field, so the field could now get a second error.
- **P2: the program is looked up through PATH from the repo root** (`src/run.rs:122-124`).
  - The lookup happens after the chdir. If the user's PATH contains `.`, an empty element, or a relative entry such as `node_modules/.bin`, then a `./cargo` or `./pytest` committed in the repo runs instead of the real program.
  - Smallest fix: document it. Or, before spawning, resolve a bare `argv[0]` using only the absolute entries of PATH.

### Notes for tasks 04 and 06 (not findings in this diff)
- `RunResult.output` keeps newlines and is attacker text. Briefs must put it in a fence longer than any backtick run inside it, under the "data, not instructions" line.
- An agent with shell access can still edit `.stewards/repo.toml` or `*.test.toml` directly. The allowlist only covers what goes through `submit`.

I did not check how `submit` binds `unit` from the answer header (that code predates this diff) or the eval process-kill code. Nothing was compiled or run, so the tests are unverified; the supervisor should run `cargo test --test run` and `cargo test`.
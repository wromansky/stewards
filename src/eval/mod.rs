// `stewards eval`: drives a harness over a suite (`evals/suites/<name>.toml`) in each arm, parses
// every transcript into a `Run`, and grades the medians.
use std::collections::{BTreeMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::fmt::Display;
use std::hash::BuildHasher;
use std::os::unix::fs::symlink;
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, ExitCode, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::Value;

use crate::harness::{self, EnvRule, HARNESSES};
use crate::model::CommandEntry;
use crate::output::{CliError, Global, emit, not_implemented};

pub mod claude;
pub mod grade;
pub mod hermes;
pub mod pi;
pub mod suite;

pub use grade::{Summary, TaskGrade, Verdict};
pub use suite::{Absent, Source, Suite, SuiteTask};

/// The tool's own files, which adoption may add, change, or delete.
const ADOPTION_OWN: [&str; 4] = [".stewards/", "docs/units/", "AGENTS.md", ".gitignore"];

/// The prompt of the `full` arm's adoption session, run once per eval before any `full` run.
pub const ADOPT_PROMPT: &str = "Adopt stewards in this repository: run `stewards init --draft` and start one subagent per handoff it prints, giving it exactly that sentence as its whole task; when they have finished, run `stewards review --roles` until it exits 0.";

/// The final stop reasons of a response that did not end normally.
pub const FAILED_STOPS: [&str; 3] = ["error", "aborted", "length"];

/// What the summary says about the ritual gate and the transcript-derived steps.
pub const RITUAL_NOTE: &str = "The gate, on each full run of an edit task (read-only tasks are n/a): the runner saw a change (`changed`: outside .stewards/ and docs/units/, against the run's starting commit, untracked files included), a `stewards review` command ran somewhere in the session tree (`review`), a subagent was started whose task contains the handoff sentence for a brief under .stewards/briefs/ (`handoff`), and the runner's own `stewards next --no-brief` exits 0 afterwards (`check_green`). `status`, `route`, `next`, whether every subagent launch holding a handoff gave each child exactly that sentence as its task (`handoff_exact`), and whether a child ran `stewards submit --role reviewer` (`reviewer_submit`) are recorded, not gated. Steps are best-effort from the transcripts: a `stewards` command is seen only as the program of a `;`, `&`, `|`, `(`, `)`, or newline separated segment (after `VAR=value` words), so `timeout`/`env`/`time` prefixes, `if stewards ...`, `{ ...; }`, `bash -c '...'`, and backticks are misread or missed.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Arm {
    Bare,
    BlockOnly,
    Full,
    /// `full` with the AGENTS.md block's developer lines replaced (`--no-developers`).
    FullNoDevelopers,
}

impl Arm {
    pub fn as_str(self) -> &'static str {
        match self {
            Arm::Bare => "bare",
            Arm::BlockOnly => "block-only",
            Arm::Full => "full",
            Arm::FullNoDevelopers => "full-no-developers",
        }
    }

    /// The arms built from the adopted state.
    pub fn is_full(self) -> bool {
        matches!(self, Arm::Full | Arm::FullNoDevelopers)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Ok,
    Failed,
    TimedOut,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Failed => "failed",
            Status::TimedOut => "timed_out",
        }
    }
}

/// `task` runs are graded; `setup` runs (the adoption session, a task's `setup` task) are not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunRole {
    Task,
    Setup,
}

/// Token counts. Every count is the sum of the harness's per-response `input`, `output`,
/// `cache_read`, and `cache_write` usage fields; `sum` adds all four, and that is the number the
/// token ratios compare, in every arm.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl Tokens {
    pub fn sum(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }

    /// What one response put in context: `input + cache_read + cache_write`.
    pub fn context(&self) -> u64 {
        self.input + self.cache_read + self.cache_write
    }

    pub fn add(&mut self, other: Tokens) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
    }
}

/// The recorded ritual steps, in order; `review`, `handoff`, and `check_green` are gated (see
/// `RITUAL_NOTE`).
pub const RITUAL_STEPS: [&str; 8] = [
    "status",
    "route",
    "next",
    "review",
    "handoff",
    "handoff_exact",
    "reviewer_submit",
    "check_green",
];

/// Each of `RITUAL_STEPS` and whether the run showed it.
pub type RitualSteps = BTreeMap<&'static str, bool>;

/// One session tree parsed from its transcript.
#[derive(Debug, Clone, Serialize)]
pub struct Run {
    /// The parent session only; `None` when the transcript cannot measure it (a Claude Code
    /// session that ended without its stream's `result` line).
    pub main_tokens: Option<Tokens>,
    /// The parent plus every child session; `None` as for `main_tokens`.
    pub total_tokens: Option<Tokens>,
    /// Cache refreshes over the tree, which do not enter model context: not in the token sums
    /// above; their cost is in `cost_usd`.
    pub cache_warming: Tokens,
    /// The largest `Tokens::context` of any one main-session response; `None` when the
    /// transcript has no per-response usage (a Hermes session without its `agent.log` lines).
    pub peak_main_context: Option<u64>,
    /// Tool names called in the main session, in order.
    pub tool_calls: Vec<String>,
    /// Distinct repo-relative paths the main session opened: a file counts when its content was
    /// read or searched (a read tool, a content search given its path, a `shell_reads` command);
    /// a listing never counts, whatever produced it (`ls`, `find`, Claude Code's `Glob`, Hermes's
    /// `search_files` with `target: files`). Each driver's `files_opened` names its tools; the
    /// runner then keeps only `existing_files`.
    pub files_opened: Vec<String>,
    /// The same over the whole session tree.
    pub files_opened_tree: Vec<String>,
    /// Shell commands the main session ran, in order.
    pub shell_commands: Vec<String>,
    /// From the transcripts; `check_green` is set by the runner (`false` until then).
    pub ritual_steps: RitualSteps,
    /// The harness's own cost figure summed over the tree; `None` when the transcript has none
    /// or the driver does not trust it (see `harness_reported_cost_usd`).
    pub cost_usd: Option<f64>,
    /// The harness's figure when the driver does not trust it as `cost_usd` (Claude Code 1.0.128
    /// prices an unknown model id at another model's rates); `None` otherwise.
    pub harness_reported_cost_usd: Option<f64>,
    pub duration_s: f64,
    /// `provider/model` of the main session, as the transcript names it.
    pub model: Option<String>,
    /// Every `provider/model` used anywhere in the session tree, sorted.
    pub models: Vec<String>,
    /// Every thinking level any session of the tree was set to or ran at, sorted.
    pub thinking_levels: Vec<String>,
    /// Sessions of the tree (`main`, or the child file relative to the tree) with no thinking
    /// level recorded anywhere (flagged, not failed).
    pub sessions_without_thinking: Vec<String>,
    /// Child sessions in the tree.
    pub subagent_sessions: usize,
    /// Top-level sessions in the transcript besides the task's own (a harness a session started
    /// itself), left out of every count (flagged, not failed).
    pub uncounted_sessions: Vec<String>,
    /// Each successful `stewards submit --role reviewer` a child ran, in order.
    pub reviews: Vec<ReviewSubmit>,
    /// Shell commands anywhere in the tree that read git history (see `reads_git_history`).
    pub git_history_commands: Vec<String>,
    /// The stop reason of the main session's last response.
    pub stop_reason: Option<String>,
    /// Child session files (relative to the tree) whose last response stopped with one of
    /// `FAILED_STOPS`, with that reason.
    pub failed_children: Vec<String>,
    /// Unparseable last lines tolerated in the files of a failed or timed-out session.
    pub truncated_lines: usize,
}

/// One reviewer `submit` from its output: each unit with its verdict and findings.
#[derive(Debug, Clone, Serialize)]
pub struct ReviewSubmit {
    pub units: Vec<ReviewedUnit>,
    /// The shell commands of the child that ran it.
    pub commands: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewedUnit {
    pub unit: String,
    /// `approve` or `changes`.
    pub verdict: String,
    pub findings: usize,
}

/// The units of a reviewer `submit`'s JSON output: `summary` is `reviewer: <unit> <verdict>
/// (<parts>), ...` and each of `findings` names its unit. `None` for any other output (an error,
/// another role).
/// simplified: `--human` output is not read; upgrade if reviewers start passing it.
pub fn reviewed_units(output: &str) -> Option<Vec<ReviewedUnit>> {
    let v: serde_json::Value = serde_json::Deserializer::from_str(&output[output.find('{')?..])
        .into_iter()
        .next()?
        .ok()?;
    let summary = v["summary"].as_str()?.strip_prefix("reviewer: ")?;
    let findings = v["findings"].as_array().map_or(&[][..], Vec::as_slice);
    Some(
        summary
            .split("), ")
            .filter_map(|part| {
                let mut words = part.split_whitespace();
                let (unit, verdict) = (words.next()?, words.next()?);
                Some(ReviewedUnit {
                    unit: unit.to_string(),
                    verdict: verdict.to_string(),
                    findings: findings.iter().filter(|f| f["unit"] == unit).count(),
                })
            })
            .collect(),
    )
}

/// One thing a session did, in harness-neutral form, for `ritual_steps`.
pub enum Action {
    Shell {
        command: String,
        /// Run by a child session, not the main one.
        child: bool,
    },
    /// A subagent launch; `tasks` are what each child is given as its task.
    Dispatch { tasks: Vec<String> },
}

/// Whether `text` is exactly a handoff (`brief::handoff`) of a brief under `.stewards/briefs/`,
/// named as `brief::rel_path` names one (`review-<snap>.md`, `<unit>-<role>-<snap>.md`, ...).
pub fn is_handoff(text: &str) -> bool {
    text.strip_prefix("Read `")
        .and_then(|r| r.strip_suffix("` and follow it."))
        .filter(|p| p.starts_with('/'))
        .and_then(|p| p.rsplit_once("/.stewards/briefs/"))
        .is_some_and(|(dir, name)| {
            !dir.contains('`') && name.ends_with(".md") && crate::brief::job_of(name).is_some()
        })
}

/// Whether `text` holds a handoff anywhere in it.
pub fn holds_handoff(text: &str) -> bool {
    const END: &str = "` and follow it.";
    text.match_indices("Read `").any(|(i, _)| {
        text[i..]
            .find(END)
            .is_some_and(|j| is_handoff(&text[i..i + j + END.len()]))
    })
}

/// Whether a `stewards` command (see `stewards_commands`) is `submit --role reviewer`.
pub fn is_reviewer_submit(cmd: &[&str]) -> bool {
    cmd.first() == Some(&"submit")
        && (cmd.contains(&"--role=reviewer") || cmd.windows(2).any(|w| w == ["--role", "reviewer"]))
}

/// The transcript-derived ritual steps: `status`, `route`, `next`, `review` are shell commands,
/// in any session, whose argv starts with `stewards <cmd>`; `handoff` is a subagent launched with
/// a handoff in one of its argument strings, and `handoff_exact` that every such launch gives
/// each child exactly a handoff as its task; `reviewer_submit` is `is_reviewer_submit` in a
/// child. `check_green` is left `false` for the runner.
pub fn ritual_steps(actions: &[Action]) -> RitualSteps {
    let mut steps: RitualSteps = RITUAL_STEPS.iter().map(|&s| (s, false)).collect();
    let mut exact = true;
    for a in actions {
        match a {
            Action::Shell { command, child } => {
                for cmd in stewards_commands(command) {
                    if let Some(step) = ["status", "route", "next", "review"]
                        .into_iter()
                        .find(|s| *s == cmd[0])
                    {
                        steps.insert(step, true);
                    }
                    if *child && is_reviewer_submit(&cmd) {
                        steps.insert("reviewer_submit", true);
                    }
                }
            }
            Action::Dispatch { tasks } => {
                if tasks.iter().any(|t| holds_handoff(t)) {
                    steps.insert("handoff", true);
                    exact &= tasks.iter().all(|t| is_handoff(t));
                }
            }
        }
    }
    steps.insert("handoff_exact", steps["handoff"] && exact);
    steps
}

/// Git subcommands that read history a run's tree should not offer (see `reads_git_history`).
const GIT_HISTORY: [&str; 7] = [
    "log", "show", "checkout", "reflog", "stash", "fsck", "cat-file",
];

/// Whether a shell command line runs `git` with one of `GIT_HISTORY` (after `-C dir`/`-c k=v`).
pub fn reads_git_history(command: &str) -> bool {
    segments(command).into_iter().any(|words| {
        let mut words = words.into_iter();
        if words.next().map(program) != Some("git") {
            return false;
        }
        while let Some(w) = words.next() {
            match w {
                "-C" | "-c" => {
                    words.next();
                }
                _ if w.starts_with('-') => {}
                _ => return GIT_HISTORY.contains(&w),
            }
        }
        false
    })
}

/// The delimiter of a heredoc whose `<<` was just before `rest` (`<<WORD`, `<<'WORD'`,
/// `<<"WORD"`, `<<-WORD`), and whether it was `<<-`.
fn heredoc_delimiter(rest: &str) -> Option<(&str, bool)> {
    let dash = rest.starts_with('-');
    let rest = rest[usize::from(dash)..].trim_start_matches([' ', '\t']);
    let word = match rest.chars().next()? {
        q @ ('\'' | '"') => rest[1..].split(q).next()?,
        _ => rest
            .split(|c: char| c.is_whitespace() || ";&|()<>".contains(c))
            .next()?,
    };
    let word = word.trim_start_matches('\\');
    (!word.is_empty()).then_some((word, dash))
}

/// The offset just past the heredoc bodies starting at `from`, one per delimiter in order: each
/// runs to the line equal to its word (leading tabs dropped after `<<-`), or to the end.
fn heredoc_end(command: &str, from: usize, delimiters: &[(&str, bool)]) -> usize {
    let mut at = from;
    for &(word, dash) in delimiters {
        while at < command.len() {
            let end = command[at..]
                .find('\n')
                .map_or(command.len(), |n| at + n + 1);
            let line = command[at..end].trim_end_matches('\n');
            at = end;
            if (if dash {
                line.trim_start_matches('\t')
            } else {
                line
            }) == word
            {
                break;
            }
        }
    }
    at
}

/// Each `;`, `&`, `|`, `(`, `)`, or newline separated segment of a command line as words, quotes
/// trimmed, leading `VAR=value` words skipped. A separator or space inside `'...'` or `"..."`, or
/// after a backslash outside single quotes, does not split. Heredoc bodies are skipped.
fn segments(command: &str) -> Vec<Vec<&str>> {
    let (mut segments, mut words) = (vec![], vec![]);
    let (mut start, mut quote, mut escaped) = (None, None, false);
    // delimiters of heredocs opened on the current line; their bodies end at `skip_to`
    let (mut heredocs, mut skip_to) = (vec![], 0);
    for (i, c) in command.char_indices() {
        if i < skip_to {
            continue;
        }
        let rest = &command[i..];
        if quote.is_none()
            && !escaped
            && rest.starts_with("<<")
            && !rest.starts_with("<<<")
            && !command[..i].ends_with('<')
        {
            heredocs.extend(heredoc_delimiter(&rest[2..]));
        }
        // Some(true) ends the segment, Some(false) only the word
        let split = match (escaped, quote, c) {
            (true, ..) => {
                escaped = false;
                None
            }
            (_, Some(q), _) if c == q => {
                quote = None;
                None
            }
            (_, Some('\''), _) => None,
            (.., '\\') => {
                escaped = true;
                None
            }
            (_, Some(_), _) => None,
            (.., '"' | '\'') => {
                quote = Some(c);
                None
            }
            (.., '\n' | ';' | '&' | '|' | '(' | ')') => Some(true),
            _ if c.is_whitespace() => Some(false),
            _ => None,
        };
        match split {
            None => {
                start.get_or_insert(i);
            }
            Some(segment_end) => {
                if let Some(s) = start.take() {
                    words.push(&command[s..i]);
                }
                if segment_end {
                    segments.push(std::mem::take(&mut words));
                }
                if c == '\n' && !heredocs.is_empty() {
                    skip_to = heredoc_end(command, i + 1, &std::mem::take(&mut heredocs));
                }
            }
        }
    }
    words.extend(start.map(|s| &command[s..]));
    segments.push(words);
    segments
        .into_iter()
        .map(|words| {
            words
                .into_iter()
                .map(|w| w.trim_matches(['"', '\'']))
                .skip_while(|w| w.contains('='))
                .collect()
        })
        .collect()
}

fn program(word: &str) -> &str {
    Path::new(word)
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
}

/// The `stewards` commands a shell command line runs: each segment (see `segments`) whose
/// program is `stewards`, as its words from the subcommand on, global flags skipped.
pub fn stewards_commands(command: &str) -> Vec<Vec<&str>> {
    segments(command)
        .into_iter()
        .filter_map(|words| {
            let mut words = words.into_iter();
            if program(words.next()?) != "stewards" {
                return None;
            }
            while let Some(w) = words.next() {
                match w {
                    "-C" | "--repo" => {
                        words.next();
                    }
                    _ if w.starts_with('-') => {}
                    _ => return Some(std::iter::once(w).chain(words).collect()),
                }
            }
            None
        })
        .collect()
}

/// `grep`/`rg` options whose value is the next word.
const GREP_VALUE_FLAGS: [&str; 18] = [
    "-e",
    "-f",
    "-g",
    "-m",
    "-t",
    "-A",
    "-B",
    "-C",
    "--regexp",
    "--file",
    "--glob",
    "--iglob",
    "--type",
    "--type-not",
    "--max-count",
    "--context",
    "--after-context",
    "--before-context",
];

/// The paths a shell command line reads, recognised simply: in each segment (see `segments`)
/// whose program is `cat`, `head`, `tail`, `less`, `sed` with `-n`, `grep`, or `rg`, every word
/// after it that is not a flag, an option value, a number, a word with `$` or starting with `!`,
/// or a redirection (a lone `>`/`<` operator also drops the word after it). Option values:
/// `-n`/`-c` of `head`/`tail`, `-e`/`-f` of `sed`, `GREP_VALUE_FLAGS` of `grep`/`rg`. `sed -n`
/// drops its script and `grep`/`rg` their pattern (the first such word) unless `-e`/`-f` gave it.
/// Globs and directories are returned as written (`existing_files` expands and drops them). After
/// a leading `cd <dir>` segment, a relative path is joined to `<dir>`.
/// simplified: a later `cd`, or a `for` list, is misread; upgrade if `files_opened` disagrees with
/// the transcripts.
pub fn shell_reads(command: &str) -> Vec<String> {
    let segments = segments(command);
    let cd = match segments.first().map(Vec::as_slice) {
        Some(["cd", dir]) if !dir.starts_with(['-', '~']) && !dir.contains('$') => Some(*dir),
        _ => None,
    };
    let mut out = vec![];
    for words in segments {
        let Some((first, args)) = words.split_first() else {
            continue;
        };
        let (drop_first, value_flags): (bool, &[&str]) = match program(first) {
            "cat" | "less" => (false, &[]),
            "head" | "tail" => (false, &["-n", "-c"]),
            "sed" if args.contains(&"-n") => (true, &["-e", "-f"]),
            "grep" | "rg" => (true, &GREP_VALUE_FLAGS),
            _ => continue,
        };
        let mut pattern_given = false;
        let mut operands = vec![];
        let mut it = args.iter();
        while let Some(&w) = it.next() {
            let operator = w.trim_start_matches(|c: char| c.is_ascii_digit());
            if value_flags.contains(&w) {
                pattern_given |= matches!(w, "-e" | "-f" | "--regexp" | "--file");
                it.next();
            } else if !operator.is_empty() && operator.chars().all(|c| matches!(c, '<' | '>')) {
                it.next();
            } else if !(w.starts_with(['-', '!'])
                || w.contains(['<', '>', '$'])
                || w.chars().all(|c| c.is_ascii_digit()))
            {
                operands.push(w);
            }
        }
        out.extend(
            operands
                .into_iter()
                .skip(usize::from(drop_first && !pattern_given))
                .map(|p| match cd {
                    Some(dir) => Path::new(dir).join(p).display().to_string(),
                    None => p.to_string(),
                }),
        );
    }
    out
}

/// The regular files among a session's opened `paths` (relative to `dir`, the run's directory
/// after the session), in order: a glob (`*` or `?`, in the last component only) is expanded
/// against `dir`'s listing, a name starting with `.` matched only by a pattern that does;
/// directories and paths that do not exist are dropped.
pub fn existing_files(dir: &Path, paths: &[String]) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for p in paths {
        let found = if p.contains(['*', '?']) {
            let (parent, name) = p.rsplit_once('/').unwrap_or(("", p));
            let Ok(glob) = globset::Glob::new(name).map(|g| g.compile_matcher()) else {
                continue;
            };
            if parent.contains(['*', '?']) {
                continue;
            }
            let mut names: Vec<String> = std::fs::read_dir(dir.join(parent))
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| glob.is_match(n) && (name.starts_with('.') || !n.starts_with('.')))
                .map(|n| {
                    if parent.is_empty() {
                        n
                    } else {
                        format!("{parent}/{n}")
                    }
                })
                .collect();
            names.sort();
            names
        } else {
            vec![p.clone()]
        };
        for f in found {
            if dir.join(&f).is_file() && !out.contains(&f) {
                out.push(f);
            }
        }
    }
    out
}

/// What a driver's session left behind.
pub struct Session {
    /// From the harness's exit; the runner downgrades `Ok` when the transcript shows a failed stop.
    pub status: Status,
    /// The main session's transcript; `None` when the harness wrote none.
    pub transcript: Option<PathBuf>,
    pub duration_s: f64,
}

/// One harness. Graders see only the `Run` that `parse` returns.
pub trait Driver {
    /// Runs one session on `prompt` in `dir` (on `model`, else the harness default, at the
    /// `thinking` level) with `env` set on top of a fresh environment (see `fresh_session_env`),
    /// keeps its transcript under `sessions`, and waits for it to end or time out.
    fn start(
        &self,
        dir: &Path,
        prompt: &str,
        model: Option<&str>,
        thinking: &str,
        sessions: &Path,
        env: &[(&str, OsString)],
    ) -> Result<Session>;

    /// Parses a transcript; `truncation_ok` (a failed or timed-out session) tolerates an
    /// unparseable last line in each file.
    fn parse(&self, transcript: &Path, duration_s: f64, truncation_ok: bool) -> Result<Run>;

    /// A file (path, contents) this harness needs beside `AGENTS.md` to load it: the runner
    /// commits it into every arm copy that has an `AGENTS.md` (see `add_context_file`).
    fn context_file(&self) -> Option<(&'static str, &'static str)> {
        None
    }

    /// The credential files this harness's sessions are linked to (Pi's `auth.json`, Claude
    /// Code's `.credentials.json`, Hermes's `.env`): `redact_secrets` keeps their values out of
    /// the results.
    fn credential_files(&self) -> Vec<PathBuf> {
        vec![]
    }
}

/// Words that mark a credential entry's value as a secret, matched in its name in any case.
const SECRET_NAMES: [&str; 6] = ["key", "token", "secret", "password", "access", "refresh"];
/// The shortest value taken as a secret: shorter ones (a region, a flag) are left in the results.
const SECRET_MIN: usize = 8;

/// Every value in the credential `files` that looks like a secret: in a JSON file each string,
/// at any depth, under a name holding one of `SECRET_NAMES`; in any other (a dotenv file) each
/// `NAME=value` line's value under such a name, quotes and `export ` removed; at least
/// `SECRET_MIN` characters. A file that cannot be read gives none. No value is ever printed.
pub fn credential_secrets(files: &[PathBuf]) -> Vec<String> {
    fn secret_name(name: &str) -> bool {
        let name = name.to_lowercase();
        SECRET_NAMES.iter().any(|w| name.contains(w))
    }
    fn walk(v: &Value, secret: bool, out: &mut Vec<String>) {
        match v {
            Value::String(s) if secret => out.push(s.clone()),
            Value::Array(a) => a.iter().for_each(|v| walk(v, secret, out)),
            Value::Object(o) => o
                .iter()
                .for_each(|(k, v)| walk(v, secret || secret_name(k), out)),
            _ => {}
        }
    }
    let mut out = vec![];
    for file in files {
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        match serde_json::from_str::<Value>(&text) {
            Ok(v) => walk(&v, false, &mut out),
            Err(_) => out.extend(
                text.lines()
                    .filter_map(|l| l.trim().trim_start_matches("export ").split_once('='))
                    .filter(|(name, _)| secret_name(name))
                    .map(|(_, v)| v.trim().trim_matches(['"', '\'']).to_string()),
            ),
        }
    }
    out.retain(|s| s.chars().count() >= SECRET_MIN);
    // the longest first, so one secret holding another is replaced whole
    out.sort_by_key(|s| std::cmp::Reverse(s.len()));
    out.dedup();
    out
}

/// `text` with every occurrence of each of `secrets` replaced by `[redacted]`.
// simplified: exact occurrences only; a secret a transcript holds escaped (a JSON string of one
// with a quote or backslash) or split stays. Match escaped forms if a credential ever has them.
pub fn redact_text(text: &str, secrets: &[String]) -> String {
    let mut text = text.to_string();
    for s in secrets {
        if text.contains(s.as_str()) {
            text = text.replace(s.as_str(), "[redacted]");
        }
    }
    text
}

/// Reads `driver`'s credential secrets now (after the session, so a token it refreshed is
/// included) and redacts them in every text file under `dir`, at any depth; returns them, for
/// the run record. Every driver's kept transcripts and logs go through this.
pub fn redact_secrets(driver: &dyn Driver, dir: &Path) -> Result<Vec<String>> {
    fn walk(dir: &Path, secrets: &[String]) -> Result<()> {
        for e in std::fs::read_dir(dir)? {
            let e = e?;
            let path = e.path();
            // a link (Hermes's home links) is not followed: its target is not kept
            let kind = e.file_type()?;
            if kind.is_dir() {
                walk(&path, secrets)?;
            } else if kind.is_file()
                && let Ok(text) = std::fs::read_to_string(&path)
            {
                let redacted = redact_text(&text, secrets);
                if redacted != text {
                    std::fs::write(&path, redacted)?;
                }
            }
        }
        Ok(())
    }
    let secrets = credential_secrets(&driver.credential_files());
    if !secrets.is_empty() && dir.is_dir() {
        walk(dir, &secrets)?;
    }
    Ok(secrets)
}

/// Set by a SIGINT or SIGTERM to the runner; `run_session` clears it when it kills its command.
static SIGNALLED: AtomicBool = AtomicBool::new(false);
/// Set by a SIGINT or SIGTERM to the runner and never cleared: the session it interrupted is
/// recorded `failed` (`interrupted`) and no other starts.
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_interrupt(_: libc::c_int) {
    SIGNALLED.store(true, Ordering::SeqCst);
    INTERRUPTED.store(true, Ordering::SeqCst);
}

/// Catches the runner's next SIGINT and SIGTERM (see `SIGNALLED`, `INTERRUPTED`); a second one
/// gets the default action, so it ends the runner at once.
fn catch_interrupts() {
    // SAFETY: the handler only stores to atomics, which is async-signal-safe.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = on_interrupt as *const () as libc::sighandler_t;
        action.sa_flags = libc::SA_RESETHAND;
        for signal in [libc::SIGINT, libc::SIGTERM] {
            libc::sigaction(signal, &action, std::ptr::null_mut());
        }
    }
}

fn interrupted_error() -> anyhow::Error {
    CliError::new(
        "interrupted",
        "the eval was interrupted (SIGINT or SIGTERM); the session it stopped is recorded `failed` (`interrupted`) and the summary covers the sessions so far",
        "start the eval again",
    )
    .into()
}

/// Spawns `cmd` in its own process group and waits for it, killing the group after `limit`, or
/// at once on an interrupt (`SIGNALLED`, then `Failed`). Returns its status and seconds.
fn run_session(cmd: &mut Command, limit: Duration) -> Result<(Status, f64)> {
    std::os::unix::process::CommandExt::process_group(cmd, 0);
    let started = Instant::now();
    let mut child = cmd
        .spawn()
        .with_context(|| format!("starting {}", cmd.get_program().to_string_lossy()))?;
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break if s.success() {
                Status::Ok
            } else {
                Status::Failed
            };
        }
        if started.elapsed() >= limit {
            kill(&mut child);
            break Status::TimedOut;
        }
        if SIGNALLED.swap(false, Ordering::SeqCst) {
            kill(&mut child);
            break Status::Failed;
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    Ok((status, started.elapsed().as_secs_f64()))
}

/// Kills the session's whole process group, then the child itself; the runner then kills what
/// left the group (harnesses start shell commands, background subagents, and Hermes its gateway
/// daemon in their own groups).
fn kill(child: &mut Child) {
    let _ = Command::new("kill")
        .args(["-KILL", &format!("-{}", child.id())])
        .status();
    let _ = child.kill();
    let _ = child.wait();
}

/// A file's JSON lines; with `truncation_ok` an unparseable last line is dropped and counted.
fn records(path: &Path, truncation_ok: bool) -> Result<(Vec<Value>, usize)> {
    let text = std::fs::read_to_string(path).with_context(|| path.display().to_string())?;
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let mut out = vec![];
    for (n, l) in lines.iter().enumerate() {
        match serde_json::from_str(l) {
            Ok(v) => out.push(v),
            Err(_) if truncation_ok && n + 1 == lines.len() => return Ok((out, 1)),
            Err(e) => {
                return Err(e).with_context(|| format!("{}:{}", path.display(), n + 1));
            }
        }
    }
    Ok((out, 0))
}

/// A path a session opened, relative to the session's `cwd` (a `..` steps back up); `None` outside
/// it.
fn repo_relative(path: &str, cwd: &str) -> Option<String> {
    let p = Path::new(path);
    let rel = if p.is_absolute() {
        p.strip_prefix(cwd).ok()?
    } else {
        p
    };
    let mut parts = vec![];
    for c in rel.components() {
        match c {
            Component::Normal(s) => parts.push(s.to_string_lossy().into_owned()),
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop()?;
            }
            _ => return None,
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Prepares a harness command to run as a fresh top-level session: drops every variable a harness
/// sets for its own children (an enclosing Pi's `PI_*` hides the `subagent` tool from a nested Pi)
/// and `STEWARDS_DIRECT`, then sets `env`.
pub fn fresh_session_env(cmd: &mut Command, env: &[(&str, OsString)]) {
    let prefixes: Vec<&str> = HARNESSES
        .iter()
        .filter_map(|h| match h.env {
            EnvRule::Prefix(p) => Some(p),
            EnvRule::Unverified => None,
        })
        .collect();
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy();
        if name == "STEWARDS_DIRECT" || prefixes.iter().any(|p| name.starts_with(p)) {
            cmd.env_remove(&key);
        }
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
}

/// The runner's own `stewards next --no-brief` in a `full` run's directory after its session.
#[derive(Debug, Clone, Serialize)]
pub struct NextCheck {
    pub exit_code: Option<i32>,
    /// `None` when the output had no job list.
    pub jobs: Option<usize>,
}

/// Processes left running under a session's directory after it ended.
#[derive(Debug, Clone, Serialize)]
pub struct Leftovers {
    pub killed: usize,
    /// Still alive when the wait gave up; the eval stops after recording the run.
    pub survived: Vec<u32>,
}

/// One started session, as written to `runs/<label>.json`.
#[derive(Debug, Clone, Serialize)]
pub struct RunRecord {
    pub label: String,
    /// `None` for the adoption session.
    pub task: Option<u32>,
    pub arm: Arm,
    pub role: RunRole,
    /// 1-based run number within the task and arm.
    pub run: u32,
    pub model: Option<String>,
    pub thinking: String,
    /// True when the tree used a model other than `model` (flagged, not failed).
    pub model_mismatch: bool,
    /// True when a session of the tree ran at a thinking level other than `thinking` (flagged,
    /// not failed).
    pub thinking_mismatch: bool,
    pub status: Status,
    /// Why `status` is `failed` though the harness exited 0.
    pub failure: Option<String>,
    /// Under the results' `sessions/`.
    pub transcript: Option<PathBuf>,
    /// The fixture copy the session ran in (deleted after the run unless `--keep-work`).
    pub workdir: PathBuf,
    /// `None` where `/proc` is missing, so nothing could be found or killed.
    pub leftovers: Option<Leftovers>,
    /// `full` arms only.
    pub next_check: Option<NextCheck>,
    /// Whether the run's directory differs from its starting commit after the session, outside
    /// `.stewards/` and `docs/units/` (see `changed_paths`): the gate's first fact.
    pub changed: bool,
    /// `full` arms: each unit of each reviewer `submit`, in order (report-only).
    pub reviews: Vec<UnitReview>,
    pub result: Option<Run>,
    /// Why `result` is missing when the transcript could not be parsed.
    pub error: Option<String>,
}

/// One unit of one reviewer `submit` in a `full` run.
#[derive(Debug, Clone, Serialize)]
pub struct UnitReview {
    /// 1-based reviewer `submit` of the run.
    pub round: usize,
    pub unit: String,
    pub verdict: String,
    pub findings: usize,
    /// Bug gaps on the unit's card after the run that were not there before it.
    pub bugs: Vec<String>,
    /// Whether the submitting child ran the unit's test command (its test card's, else one of
    /// the repo's `commands.test`), as a substring of one of its shell commands.
    pub tests_ran: bool,
}

/// One eval invocation.
pub struct Plan {
    /// The harness table name of the driver the sessions run on.
    pub harness: &'static str,
    pub tasks: Vec<u32>,
    pub arms: Vec<Arm>,
    pub runs: u32,
    pub model: Option<String>,
    pub thinking: String,
    pub max_cost: Option<f64>,
    /// A previously adopted fixture to copy for the `full` arms instead of adopting again.
    pub setup_dir: Option<PathBuf>,
    /// The repo's `evals/` directory (suites, fixture, tasks, arms.sh).
    pub evals: PathBuf,
    /// What each arm copies and which tasks exist.
    pub suite: Suite,
    /// Where `runs/`, `sessions/`, and the summaries go.
    pub results: PathBuf,
    /// Scratch space for the per-run fixture copies, with a neutral name (`<tmp>/w-<random>`).
    pub work: PathBuf,
    /// Keep each run's fixture copy and session scratch instead of deleting it after the run.
    pub keep_work: bool,
    /// The stewards binary the `full` arms put on `PATH`.
    pub stewards: PathBuf,
    /// The `PATH` to start from.
    pub path: OsString,
    /// The harness setup the sessions ran with, in plain words, for the summary.
    pub harness_setup: Option<String>,
}

/// True when one more session, estimated at the mean of `costs` so far, would take the spend over
/// `max`. With no cost known yet there is no estimate, so the session runs.
pub fn would_exceed(max: f64, costs: &[f64]) -> bool {
    let spent: f64 = costs.iter().sum();
    !costs.is_empty() && spent + spent / costs.len() as f64 > max
}

/// What the `full` arm's setup did, for the summary.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SetupNotes {
    /// How the units were set, in plain words.
    pub units: Option<String>,
    /// Paths the adopted state added outside `.stewards/` and `docs/units/` (`AGENTS.md`, tests).
    pub added_paths: Vec<String>,
    /// Every gap on the adopted cards, as `unit: kind: title`: a task whose seeded bug or answer
    /// is listed started known.
    pub gaps: Vec<String>,
    /// The flows adoption recorded in `.stewards/repo.toml`, as `name: unit, unit`.
    pub flows: Vec<String>,
    /// A `repo` suite's one build, in seconds: setup, outside every ratio.
    pub build_s: Option<f64>,
    /// A `repo` suite's pinned commit as git resolved it in the clone.
    pub resolved_commit: Option<String>,
}

struct Runner<'a> {
    driver: &'a dyn Driver,
    plan: &'a Plan,
    records: Vec<RunRecord>,
    costs: Vec<f64>,
    /// Labels of sessions whose cost is unknown (no parsed transcript).
    unknown_cost: Vec<String>,
    stopped: bool,
}

impl Runner<'_> {
    /// Starts one session unless `--max-cost` says stop or the eval was interrupted, and records
    /// it. Returns whether it ran to its end, uninterrupted.
    fn session(&mut self, s: SessionSpec) -> Result<bool> {
        if INTERRUPTED.load(Ordering::SeqCst) {
            return Ok(false);
        }
        if let Some(m) = self.plan.max_cost
            && (!self.unknown_cost.is_empty() || would_exceed(m, &self.costs))
        {
            self.stopped = true;
            return Ok(false);
        }
        let n = self.records.len() + 1;
        let label = format!("{n:03}-{}", s.label);
        let scratch = self.plan.work.join("s").join(n.to_string());
        let sessions = scratch.join("t");
        let mut env = vec![
            ("PATH", s.path.clone()),
            ("CARGO_INSTALL_ROOT", scratch.join("c").into_os_string()),
        ];
        // the name alone would tell a bare or block-only session about stewards
        if s.arm.is_full() {
            env.push(("STEWARDS_HOME", scratch.join("h").into_os_string()));
        }
        let start = git(&s.dir, &["rev-parse", "HEAD"])?;
        let gaps_before = s.arm.is_full().then(|| card_gaps(&s.dir));
        let session = self.driver.start(
            &s.dir,
            &s.prompt,
            self.plan.model.as_deref(),
            &self.plan.thinking,
            &sessions,
            &env,
        )?;
        let interrupted = INTERRUPTED.load(Ordering::SeqCst);
        let root = s.dir.parent().unwrap_or(&s.dir).to_path_buf();
        let leftovers = kill_leftovers(&root)?;
        let changed = changed_paths(&s.dir, start.trim())?
            .iter()
            .any(|p| !p.starts_with(".stewards/") && !p.starts_with("docs/units/"));
        let next_check = if s.arm.is_full() {
            Some(next_check(&self.plan.stewards, &s.dir)?)
        } else {
            None
        };
        let secrets = redact_secrets(self.driver, &sessions)?;
        let kept = self.plan.results.join("sessions").join(&label);
        copy_dir(&sessions, &kept)?;
        if !self.plan.keep_work {
            std::fs::remove_dir_all(&scratch)?;
        }
        let transcript = session
            .transcript
            .as_ref()
            .map(|t| kept.join(t.strip_prefix(&sessions).unwrap_or(t)));
        let truncation_ok = session.status != Status::Ok;
        let (mut result, error) = match &transcript {
            None => (None, Some("no transcript".to_string())),
            Some(t) => match self.driver.parse(t, session.duration_s, truncation_ok) {
                Ok(run) => (Some(run), None),
                Err(e) => (None, Some(format!("{e:#}"))),
            },
        };
        let mut status = session.status;
        let mut failure = None;
        let mut reviews = vec![];
        if let Some(r) = &mut result {
            // a timed-out or interrupted run's counts are partial on every driver, so no median
            // mixes them in
            if session.status == Status::TimedOut || interrupted {
                (r.main_tokens, r.total_tokens) = (None, None);
            }
            r.files_opened = existing_files(&s.dir, &r.files_opened);
            r.files_opened_tree = existing_files(&s.dir, &r.files_opened_tree);
            if let Some(before) = &gaps_before {
                reviews = review_outcomes(&s.dir, before, &r.reviews);
            }
            if let Some(c) = &next_check {
                r.ritual_steps.insert("check_green", c.exit_code == Some(0));
            }
            let main_failed = r
                .stop_reason
                .as_deref()
                .filter(|s| FAILED_STOPS.contains(s));
            if status == Status::Ok && (main_failed.is_some() || !r.failed_children.is_empty()) {
                status = Status::Failed;
                let mut why: Vec<String> = main_failed
                    .map(|s| format!("main session stopped with `{s}`"))
                    .into_iter()
                    .collect();
                why.extend(r.failed_children.iter().map(|c| format!("child {c}")));
                failure = Some(why.join("; "));
            }
        }
        if interrupted {
            status = Status::Failed;
            failure = Some("interrupted".to_string());
        }
        match &result {
            None => self.unknown_cost.push(label.clone()),
            Some(r) => self.costs.extend(r.cost_usd),
        }
        let model = self
            .plan
            .model
            .clone()
            .or_else(|| result.as_ref().and_then(|r| r.model.clone()));
        let record = RunRecord {
            label: label.clone(),
            task: s.task,
            arm: s.arm,
            role: s.role,
            run: s.run,
            model_mismatch: result
                .as_ref()
                .is_some_and(|r| r.models.iter().any(|m| Some(m) != model.as_ref())),
            model,
            thinking_mismatch: result
                .as_ref()
                .is_some_and(|r| r.thinking_levels.iter().any(|t| *t != self.plan.thinking)),
            thinking: self.plan.thinking.clone(),
            status,
            failure,
            transcript,
            workdir: s.dir,
            leftovers,
            next_check,
            changed,
            reviews,
            result,
            error,
        };
        let runs = self.plan.results.join("runs");
        std::fs::create_dir_all(&runs)?;
        std::fs::write(
            runs.join(format!("{label}.json")),
            redact_text(&serde_json::to_string_pretty(&record)?, &secrets),
        )?;
        let survived = record
            .leftovers
            .as_ref()
            .map_or(vec![], |l| l.survived.clone());
        self.records.push(record);
        if !survived.is_empty() {
            bail!(
                "processes {survived:?} under {} survived SIGKILL after session {label}; the eval stopped so they cannot affect later runs",
                root.display()
            );
        }
        Ok(!interrupted)
    }
}

struct SessionSpec {
    label: String,
    task: Option<u32>,
    arm: Arm,
    role: RunRole,
    run: u32,
    /// `<work>/<n>/repo`; processes under `<work>/<n>` are killed after the session.
    dir: PathBuf,
    prompt: String,
    path: OsString,
}

/// Runs the plan sequentially: for a `repo` suite the clone and its one build, then the `full`
/// setup (when a `full` arm is planned), then per task, per run number, each arm in a fresh copy
/// at `<work>/<n>/repo`; a task with a `setup` task runs after it in the same copy. Every planned
/// task prompt is read first (see `task_prompt`). Writes `runs/*.json` as each session ends, then
/// `summary.json` and `summary.md`. A SIGINT or SIGTERM stops the session running (see
/// `run_session`; the driver and the sweep then stop what it left), records it `failed`
/// (`interrupted`), writes the summary, and returns an `interrupted` error.
pub fn execute(driver: &dyn Driver, plan: &Plan) -> Result<Summary> {
    catch_interrupts();
    let tasks_dir = plan.evals.join(&plan.suite.tasks_dir);
    for &t in &plan.tasks {
        task_prompt(&tasks_dir, t)?;
        if let Some(s) = plan.suite.task(t).and_then(|t| t.setup) {
            task_prompt(&tasks_dir, s)?;
        }
    }
    std::fs::create_dir_all(&plan.work)?;
    let path_off = path_without_stewards(&plan.path, &plan.work.join("p"))?;
    let bin = plan.work.join("b");
    std::fs::create_dir_all(&bin)?;
    let link = bin.join("stewards");
    if !link.exists() {
        symlink(&plan.stewards, &link)?;
    }
    let path_on =
        std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(&path_off)))?;
    let mut r = Runner {
        driver,
        plan,
        records: vec![],
        costs: vec![],
        unknown_cost: vec![],
        stopped: false,
    };
    let mut notes = SetupNotes::default();
    // a `repo` suite's clone, which every arm copies, and its build cache
    let (base, cache) = (plan.work.join("src"), plan.work.join("target"));
    let fixture = match &plan.suite.source {
        Source::Fixture {
            dir,
            units,
            unowned,
            tests,
            test_commands,
        } => Some((dir, units, unowned, tests, test_commands)),
        Source::Repo { commit, build } => {
            let commit = &pinned_commit(plan, commit)?;
            notes.build_s = Some(clone_and_build(plan, commit, build, &base, &cache)?);
            notes.resolved_commit = Some(git(&base, &["rev-parse", "HEAD"])?.trim().to_string());
            None
        }
    };
    let mut adopted = None;
    let fail = |r: &Runner, notes: SetupNotes, err: anyhow::Error| -> Result<Summary> {
        write_summary(
            plan,
            &grade::summarize(plan, &r.records, r.stopped, &r.unknown_cost, notes),
        )?;
        Err(err)
    };
    if let Some(arm) = plan.arms.iter().copied().find(|a| a.is_full()) {
        let dir = match (fixture, &plan.setup_dir) {
            (None, _) => {
                let dir = plan.work.join("0/repo");
                // the block and the context file this eval writes are unowned and newer than
                // every stamp
                let mut allowed = vec!["AGENTS.md".to_string()];
                allowed.extend(driver.context_file().map(|(name, _)| name.to_string()));
                full_repo_tree(&base, &dir, &allowed)?;
                notes.units = Some(format!(
                    "Units, cards, and docs are the repository's own at {}, with this binary's AGENTS.md block; no adoption session.",
                    plan.suite.commit().unwrap_or_default()
                ));
                dir
            }
            (Some(_), Some(d)) => {
                notes.units = Some(format!(
                    "Units are those of the --setup-dir copy at {}; what `stewards init` proposed on its own was not recorded in this eval.",
                    d.display()
                ));
                check_setup_dir(d)?;
                d.clone()
            }
            (Some((source, units, unowned, _, test_commands)), None) => {
                let dir = plan.work.join("0/repo");
                arms_sh(plan, "full", &dir, &path_on, source)?;
                add_context_file(driver, &dir)?;
                notes.units = Some(set_full_units(
                    &plan.evals,
                    &dir,
                    units,
                    unowned,
                    test_commands,
                )?);
                let mut allowed = unowned.clone();
                allowed.extend(driver.context_file().map(|(name, _)| name.to_string()));
                allow_unowned(&dir, &allowed)?;
                let ran = r.session(SessionSpec {
                    label: "adopt".into(),
                    task: None,
                    arm,
                    role: RunRole::Setup,
                    run: 1,
                    dir: dir.clone(),
                    prompt: ADOPT_PROMPT.into(),
                    path: path_on.clone(),
                })?;
                if !ran {
                    return fail(&r, notes, interrupted_error());
                }
                dir
            }
        };
        if let Some((_, _, unowned, tests, _)) = fixture {
            if plan.setup_dir.is_none() {
                git(&dir, &["add", "-A"])?;
            }
            match adoption_changes(&dir, tests) {
                Ok(added) => notes.added_paths = added,
                Err(e) => return fail(&r, notes, e),
            }
            let mut unowned = unowned.clone();
            unowned.extend(driver.context_file().map(|(name, _)| name.to_string()));
            if let Err(e) = check_full_units(&dir, &unowned) {
                return fail(&r, notes, e);
            }
        }
        if next_check(&plan.stewards, &dir)?.exit_code != Some(0) {
            let (what, fix) = match fixture {
                Some(_) => (
                    "adopted fixture",
                    format!(
                        "read the adoption transcript under {}, or pass `--setup-dir` an adopted copy where `stewards next` exits 0",
                        plan.results.join("sessions").display()
                    ),
                ),
                None => (
                    "`full` tree",
                    format!(
                        "pin `commit` in evals/suites/{}.toml to a commit where `stewards next` lists no jobs",
                        plan.suite.name
                    ),
                ),
            };
            let err = CliError::new(
                "eval_setup_incomplete",
                format!(
                    "`stewards next` is not clean in the {what} at {}; no `full` run started",
                    dir.display()
                ),
                fix,
            );
            return fail(&r, notes, err.into());
        }
        (notes.gaps, notes.flows) = adopted_knowledge(&dir)?;
        if fixture.is_some() && plan.setup_dir.is_none() {
            commit_adopted(&dir)?;
        }
        adopted = Some(dir);
    }
    let mut n = 0;
    'all: for &task in &plan.tasks {
        let setup = plan.suite.task(task).and_then(|t| t.setup);
        for run in 1..=plan.runs {
            for &arm in &plan.arms {
                n += 1;
                let label = format!("{}-t{task}-r{run}", arm.as_str());
                let root = plan.work.join(n.to_string());
                let dir = root.join("repo");
                let path = if arm.is_full() { &path_on } else { &path_off };
                match (&adopted, fixture) {
                    // the clean runs before the build cache is linked, so it never removes it
                    (Some(from), _) if arm.is_full() => copy_adopted(from, &dir, arm)?,
                    (_, Some((source, ..))) => arms_sh(plan, arm.as_str(), &dir, path, source)?,
                    (_, None) => repo_arm_tree(&base, &dir, arm)?,
                }
                add_context_file(driver, &dir)?;
                if fixture.is_none() {
                    link_build_cache(&cache, &dir)?;
                }
                let spec = |task: u32, role: RunRole, label: String| -> Result<SessionSpec> {
                    Ok(SessionSpec {
                        label,
                        task: Some(task),
                        arm,
                        role,
                        run,
                        dir: dir.clone(),
                        prompt: task_prompt(&plan.evals.join(&plan.suite.tasks_dir), task)?,
                        path: path.clone(),
                    })
                };
                let ran = match setup {
                    Some(s) => r.session(spec(s, RunRole::Setup, format!("{label}-setup"))?)?,
                    None => true,
                } && r.session(spec(task, RunRole::Task, label.clone())?)?;
                if !plan.keep_work {
                    std::fs::remove_dir_all(&root)?;
                }
                if !ran {
                    break 'all;
                }
            }
        }
    }
    if !plan.keep_work {
        std::fs::remove_dir_all(&plan.work)?;
    }
    let summary = grade::summarize(plan, &r.records, r.stopped, &r.unknown_cost, notes);
    write_summary(plan, &summary)?;
    if INTERRUPTED.load(Ordering::SeqCst) {
        return Err(interrupted_error());
    }
    Ok(summary)
}

fn write_summary(plan: &Plan, summary: &Summary) -> Result<()> {
    std::fs::create_dir_all(&plan.results)?;
    std::fs::write(
        plan.results.join("summary.json"),
        serde_json::to_string_pretty(summary)?,
    )?;
    std::fs::write(plan.results.join("summary.md"), grade::render(summary))?;
    Ok(())
}

/// The task file `<dir>/NN-*` verbatim; a `usage` error when it contains `think` in any case,
/// since Claude Code turns thinking on from that word in a user message and the arms of every
/// harness must run at one thinking setting.
pub fn task_prompt(dir: &Path, task: u32) -> Result<String> {
    let prefix = format!("{task:02}-");
    let mut names: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| dir.display().to_string())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with(&prefix))
        })
        .collect();
    names.sort();
    let file = names
        .first()
        .with_context(|| format!("no task {task} in {}", dir.display()))?;
    let text = std::fs::read_to_string(file).with_context(|| file.display().to_string())?;
    if text.to_lowercase().contains("think") {
        return Err(CliError::new(
            "usage",
            format!("task prompt {} contains `think`", file.display()),
            "reword it: Claude Code turns thinking on from that word in a user message, so its runs would not compare with another harness's",
        )
        .into());
    }
    Ok(text)
}

/// `out` when its command succeeded, else an error naming `what` with its stderr.
fn ok(out: Output, what: impl Display) -> Result<Output> {
    if !out.status.success() {
        bail!("{what}: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(out)
}

/// `evals/arms.sh <arm> <dir> <evals/source>`: the fixture copied, `git init`ed and committed,
/// plus the arm's AGENTS.md block or `stewards init`.
fn arms_sh(plan: &Plan, arm: &str, dir: &Path, path: &OsStr, source: &Path) -> Result<()> {
    let out = Command::new("bash")
        .arg(plan.evals.join("arms.sh"))
        .arg(arm)
        .arg(dir)
        .arg(plan.evals.join(source))
        .env("PATH", path)
        .output()
        .context("running evals/arms.sh")?;
    ok(out, format!("evals/arms.sh {arm} {}", dir.display()))?;
    Ok(())
}

/// The tracked files `stewards check` reports as unowned in `dir`.
fn unowned_files(dir: &Path) -> Result<Vec<String>> {
    let loaded = crate::model::load_repo(dir)?;
    let router = crate::route::Router::new(&loaded.units, &loaded.repo.docs_path)?;
    Ok(crate::git::ls_files(dir)?
        .into_iter()
        .filter(|p| router.owner(p).is_none() && !router.tool_owned(p))
        .collect())
}

/// Replaces the units `stewards init` proposed in `dir` with `evals/<units>`, gives each unit in
/// `tests` that test command on its test card, and returns the summary's plain-words note on what
/// init proposed on its own.
fn set_full_units(
    evals: &Path,
    dir: &Path,
    units: &Path,
    unowned: &[String],
    tests: &BTreeMap<String, String>,
) -> Result<String> {
    let proposed: Vec<String> = crate::model::load_repo(dir)?
        .units
        .into_iter()
        .map(|u| u.name)
        .collect();
    let proposed_unowned = unowned_files(dir)?.len();
    std::fs::copy(evals.join(units), dir.join(".stewards/units.toml"))
        .with_context(|| format!("copying evals/{}", units.display()))?;
    for (unit, cmd) in tests {
        let card = crate::model::TestCard {
            unit: unit.clone(),
            primary_paths: vec![],
            test_command: Some(cmd.clone()),
            scenarios: vec![],
        };
        let path = dir.join(format!(".stewards/units/{unit}.test.toml"));
        std::fs::write(&path, toml::to_string(&card)?)
            .with_context(|| path.display().to_string())?;
    }
    let units = units.display();
    Ok(format!(
        "Units were set by hand for this eval from evals/{units}, so these numbers do not measure `stewards init`'s unit proposal: on its own it proposed {} and left {proposed_unowned} tracked file(s) unowned. Unowned by design: {}.",
        proposed.join(", "),
        unowned.join(", ")
    ))
}

/// Adds `names` to `unowned_ok` in `dir`'s `.stewards/repo.toml`, keeping the rest of the file,
/// so `next` and `review` do not ask for them to be given a unit.
fn allow_unowned(dir: &Path, names: &[String]) -> Result<()> {
    let path = dir.join(".stewards/repo.toml");
    let text = std::fs::read_to_string(&path).with_context(|| path.display().to_string())?;
    let mut doc: toml_edit::DocumentMut =
        text.parse().with_context(|| path.display().to_string())?;
    let list = doc
        .entry("unowned_ok")
        .or_insert(toml_edit::value(toml_edit::Array::new()))
        .as_array_mut()
        .with_context(|| format!("{}: unowned_ok is not an array", path.display()))?;
    for name in names {
        if !list.iter().any(|v| v.as_str() == Some(name)) {
            list.push(name.as_str());
        }
    }
    std::fs::write(&path, doc.to_string()).with_context(|| path.display().to_string())?;
    Ok(())
}

fn setup_error(message: String, fix: &str) -> anyhow::Error {
    CliError::new("eval_setup_incomplete", message, fix).into()
}

/// An `eval_setup_incomplete` error when `dir` leaves a tracked file unowned beyond `unowned`.
fn check_full_units(dir: &Path, unowned: &[String]) -> Result<()> {
    let stray: Vec<String> = unowned_files(dir)?
        .into_iter()
        .filter(|p| !unowned.contains(p))
        .collect();
    if stray.is_empty() {
        return Ok(());
    }
    Err(setup_error(
        format!(
            "the adopted fixture at {} leaves tracked files unowned: {}",
            dir.display(),
            stray.join(", ")
        ),
        "give them a unit in the suite's `units` file",
    ))
}

/// `--setup-dir` must be a git repository with nothing uncommitted.
fn check_setup_dir(dir: &Path) -> Result<()> {
    let fix = "pass `--setup-dir` a committed adopted copy (a git repository with an empty `git status --porcelain`)";
    match git(dir, &["status", "--porcelain"]) {
        Ok(s) if s.trim().is_empty() => Ok(()),
        Ok(s) => Err(setup_error(
            format!(
                "--setup-dir {} has uncommitted changes: {}",
                dir.display(),
                s.trim()
            ),
            fix,
        )),
        Err(e) => Err(setup_error(
            format!(
                "--setup-dir {} is not a git repository: {e:#}",
                dir.display()
            ),
            fix,
        )),
    }
}

/// Checks the index of the adopted fixture against its `initial` (root) commit: only the
/// tool's own files (`ADOPTION_OWN`) may change, and new files may appear under the suite's
/// `tests` globs; anything else is `eval_setup_incomplete`. Returns the added paths outside
/// `.stewards/` and `docs/units/`.
fn adoption_changes(dir: &Path, test_globs: &[String]) -> Result<Vec<String>> {
    let initial = git(dir, &["rev-list", "--max-parents=0", "HEAD"])?;
    let diff = git(
        dir,
        &[
            "diff",
            "--cached",
            "--name-status",
            "--no-renames",
            "-z",
            initial.trim(),
        ],
    )?;
    let mut tests = globset::GlobSetBuilder::new();
    for t in test_globs {
        tests.add(
            globset::GlobBuilder::new(t)
                .literal_separator(true)
                .build()?,
        );
    }
    let tests = tests.build()?;
    let own = |p: &str| {
        ADOPTION_OWN
            .iter()
            .any(|o| p == *o || (o.ends_with('/') && p.starts_with(o)))
    };
    let (mut added, mut stray) = (vec![], vec![]);
    let mut fields = diff.split('\0').filter(|f| !f.is_empty());
    while let (Some(status), Some(path)) = (fields.next(), fields.next()) {
        let added_file = status == "A";
        if added_file && !path.starts_with(".stewards/") && !path.starts_with("docs/units/") {
            added.push(path.to_string());
        }
        if !own(path) && !(added_file && tests.is_match(path)) {
            stray.push(format!("{status} {path}"));
        }
    }
    if !stray.is_empty() {
        return Err(setup_error(
            format!(
                "adoption changed the fixture's code in {} (against its `initial` commit): {}",
                dir.display(),
                stray.join(", ")
            ),
            &format!(
                "read the adoption transcript; adoption may change only {} and add new files under {}",
                ADOPTION_OWN.join(", "),
                test_globs.join(", ")
            ),
        ));
    }
    Ok(added)
}

/// Every gap on the cards in `dir`, as `unit: kind: title`, and every flow, as `name: units`.
pub fn adopted_knowledge(dir: &Path) -> Result<(Vec<String>, Vec<String>)> {
    let loaded = crate::model::load_repo(dir)?;
    let gaps = loaded
        .cards
        .values()
        .flat_map(|c| {
            c.gaps.iter().map(|g| {
                let kind = format!("{:?}", g.kind).to_lowercase();
                format!("{}: {kind}: {}", c.unit, g.title)
            })
        })
        .collect();
    let flows = loaded
        .repo
        .flows
        .iter()
        .map(|f| format!("{}: {}", f.name, f.units.join(", ")))
        .collect();
    Ok((gaps, flows))
}

/// Every path in `dir` that differs from commit `start`: changed, added, or deleted against it
/// (committed or not), plus untracked files that are not ignored.
fn changed_paths(dir: &Path, start: &str) -> Result<Vec<String>> {
    let diff = git(dir, &["diff", "--name-only", "--no-renames", start])?;
    let untracked = git(dir, &["ls-files", "--others", "--exclude-standard"])?;
    Ok(diff
        .lines()
        .chain(untracked.lines())
        .map(String::from)
        .collect())
}

/// The gaps on the cards in `dir` (see `adopted_knowledge`); none when they cannot be loaded.
fn card_gaps(dir: &Path) -> Vec<String> {
    adopted_knowledge(dir).map(|(g, _)| g).unwrap_or_default()
}

/// Each unit of each reviewer `submit` of a `full` run, with the bug gaps its card gained since
/// `gaps_before` and whether the submitting child ran its test command, from the cards in `dir`.
fn review_outcomes(
    dir: &Path,
    gaps_before: &[String],
    reviews: &[ReviewSubmit],
) -> Vec<UnitReview> {
    let loaded = crate::model::load_repo(dir).ok();
    let gaps_after = card_gaps(dir);
    let mut out = vec![];
    for (n, review) in reviews.iter().enumerate() {
        for u in &review.units {
            let tests: Vec<&str> = loaded.as_ref().map_or(vec![], |l| {
                match l.tests.get(&u.unit).and_then(|t| t.test_command.as_deref()) {
                    Some(c) => vec![c],
                    None => l
                        .repo
                        .commands
                        .test
                        .iter()
                        .map(|c| match c {
                            CommandEntry::Plain(cmd) | CommandEntry::Timed { cmd, .. } => {
                                cmd.as_str()
                            }
                        })
                        .collect(),
                }
            });
            let prefix = format!("{}: bug: ", u.unit);
            out.push(UnitReview {
                round: n + 1,
                unit: u.unit.clone(),
                verdict: u.verdict.clone(),
                findings: u.findings,
                bugs: gaps_after
                    .iter()
                    .filter(|g| !gaps_before.contains(g))
                    .filter_map(|g| g.strip_prefix(&prefix).map(String::from))
                    .collect(),
                tests_ran: review
                    .commands
                    .iter()
                    .any(|c| tests.iter().any(|t| c.contains(t))),
            });
        }
    }
    out
}

/// `stewards next --no-brief` in `dir`: its exit code and job count; writes no briefs.
pub fn next_check(stewards: &Path, dir: &Path) -> Result<NextCheck> {
    let out = Command::new(stewards)
        .args(["next", "--no-brief"])
        .current_dir(dir)
        .env_remove("STEWARDS_DIRECT")
        .output()
        .with_context(|| format!("running {} next", stewards.display()))?;
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    Ok(NextCheck {
        exit_code: out.status.code(),
        jobs: report["jobs"].as_array().map(Vec::len),
    })
}

/// Kills every process whose working directory is under `dir`, or whose `HERMES_HOME` is a path
/// under it (Hermes's gateway daemon, which a session starts elsewhere; no other variable is read,
/// so a process that merely visited `dir`, with it in `OLDPWD`, is left alone), from Linux
/// `/proc/<pid>/cwd` and `/proc/<pid>/environ`, again until none is left or 10 s pass. `None`
/// where there is no `/proc`.
pub fn kill_leftovers(dir: &Path) -> Result<Option<Leftovers>> {
    use std::os::unix::ffi::OsStrExt;
    if !Path::new("/proc/self/cwd").exists() {
        return Ok(None);
    }
    let dir = dir.canonicalize()?;
    let in_env = |pid: u32| {
        std::fs::read(format!("/proc/{pid}/environ")).is_ok_and(|env| {
            env.split(|b| *b == 0).any(|kv| {
                kv.strip_prefix(b"HERMES_HOME=")
                    .is_some_and(|v| Path::new(OsStr::from_bytes(v)).starts_with(&dir))
            })
        })
    };
    let me = std::process::id();
    let mut killed = HashSet::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        // a zombie has no cwd link, so it is not counted as alive
        let pids: Vec<u32> = std::fs::read_dir("/proc")?
            .flatten()
            .filter_map(|e| e.file_name().to_str()?.parse().ok())
            .filter(|&pid| {
                pid != me
                    && std::fs::read_link(format!("/proc/{pid}/cwd"))
                        .is_ok_and(|cwd| cwd.starts_with(&dir) || in_env(pid))
            })
            .collect();
        if pids.is_empty() || Instant::now() >= deadline {
            return Ok(Some(Leftovers {
                killed: killed.len(),
                survived: pids,
            }));
        }
        let _ = Command::new("kill")
            .arg("-KILL")
            .args(pids.iter().map(u32::to_string))
            .status();
        killed.extend(pids);
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .args([
            "-c",
            "user.name=dev",
            "-c",
            "user.email=dev@example.invalid",
        ])
        .args(args)
        .current_dir(dir)
        .output()
        .context("running git")?;
    let out = ok(out, format!("git {args:?} in {}", dir.display()))?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Commits the staged adopted state, after removing every gitignored file (build output, briefs,
/// telemetry), so each `full` run starts from a clean tree as `bare` does. Stamps survive: they
/// name the tree, not a commit. The subject, like every arm commit's, names nothing a session
/// should not learn from `git log` (Claude Code shows the last five subjects unasked).
fn commit_adopted(dir: &Path) -> Result<()> {
    git(dir, &["clean", "-fdXq"])?;
    if !git(dir, &["status", "--porcelain"])?.trim().is_empty() {
        git(dir, &["commit", "-qm", "update"])?;
    }
    Ok(())
}

/// The developer line of the AGENTS.md block, and its `--no-developers` replacement.
const DEVELOPER_LINE: &str = "- Changing: make the edit yourself. When a maintainer split a plan into tasks, `stewards brief <unit> --role developer` prints one handoff per task.";
const NO_DEVELOPER_LINE: &str = "- Changing: make the edit yourself.";

/// The AGENTS.md text with the developer role switched off; an error when the block's developer
/// line is not found, so a changed block cannot silently leave developers on.
pub fn without_developers(agents_md: &str) -> Result<String> {
    if !agents_md.contains(DEVELOPER_LINE) {
        bail!("AGENTS.md block has no `{DEVELOPER_LINE}`");
    }
    Ok(agents_md.replace(DEVELOPER_LINE, NO_DEVELOPER_LINE))
}

/// Commits the driver's `context_file` into `dir` when `dir` has an `AGENTS.md` and not that
/// file, folded into the last commit with the reflog expired, so the arm's history keeps its shape.
fn add_context_file(driver: &dyn Driver, dir: &Path) -> Result<()> {
    let Some((name, text)) = driver.context_file() else {
        return Ok(());
    };
    if !dir.join("AGENTS.md").is_file() || dir.join(name).exists() {
        return Ok(());
    }
    std::fs::write(dir.join(name), text)?;
    git(dir, &["add", name])?;
    git(dir, &["commit", "-q", "--amend", "--no-edit"])?;
    git(dir, &["reflog", "expire", "--expire=now", "--all"])?;
    Ok(())
}

/// `cp -a from/. to/`.
fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    let out = Command::new("cp")
        .arg("-a")
        .arg(from.join("."))
        .arg(to)
        .output()?;
    ok(out, format!("cp -a {} {}", from.display(), to.display()))?;
    Ok(())
}

/// A `full` run's copy of the adopted state, with every gitignored file removed (`git clean -fdX`).
fn copy_adopted(from: &Path, to: &Path, arm: Arm) -> Result<()> {
    copy_dir(from, to)?;
    git(to, &["clean", "-fdXq"])?;
    if arm == Arm::FullNoDevelopers {
        let agents = to.join("AGENTS.md");
        let text = std::fs::read_to_string(&agents).context("AGENTS.md")?;
        std::fs::write(&agents, without_developers(&text)?)?;
        git(to, &["commit", "-qam", "update"])?;
    }
    Ok(())
}

/// A `repo` suite's tree: the repository holding `evals/` cloned to `base` at `commit` as
/// `main`, without its remote, then `build` run there once with the user's environment and its
/// `target/` moved to `cache`. Returns the build's seconds.
fn clone_and_build(
    plan: &Plan,
    commit: &str,
    build: &[String],
    base: &Path,
    cache: &Path,
) -> Result<f64> {
    let root = plan.evals.parent().context("evals/ has no parent")?;
    let path = |p: &Path| p.to_str().map(String::from).context("non-UTF-8 path");
    git(&plan.work, &["clone", "-q", &path(root)?, &path(base)?])?;
    git(base, &["checkout", "-q", "-B", "main", commit])?;
    // a session must not push to or fetch from the user's checkout
    git(base, &["remote", "remove", "origin"])?;
    let (program, args) = build
        .split_first()
        .context("the suite's `build` is empty")?;
    let started = Instant::now();
    let out = Command::new(program)
        .args(args)
        .current_dir(base)
        .output()
        .with_context(|| format!("running {program}"))?;
    ok(out, format!("{} in {}", build.join(" "), base.display()))?;
    let seconds = started.elapsed().as_secs_f64();
    std::fs::rename(base.join("target"), cache)
        .with_context(|| format!("`{}` left no target/", build.join(" ")))?;
    Ok(seconds)
}

/// The commit a `repo` suite's arms are made from, checked at the checkout holding `evals/`: for
/// `HEAD`, that checkout's commit, refused while it has uncommitted changes under `src/`,
/// `baml_src/` or `.stewards/` (the binary and the tree could then disagree); a literal commit
/// as given. Either way, a tree where an `absent` probe's text occurs (that task is already
/// done) is refused. All refusals are `eval_setup_incomplete`.
fn pinned_commit(plan: &Plan, commit: &str) -> Result<String> {
    let root = plan.evals.parent().context("evals/ has no parent")?;
    let commit = if commit == "HEAD" {
        let dirty = git(
            root,
            &[
                "status",
                "--porcelain",
                "--",
                "src",
                "baml_src",
                ".stewards",
            ],
        )?;
        if !dirty.trim().is_empty() {
            return Err(setup_error(
                format!(
                    "the checkout at {} has uncommitted changes under src/, baml_src/ or .stewards/, so the tree the sessions get may not be the source the binary was built from: {}",
                    root.display(),
                    dirty.trim().replace('\n', "; ")
                ),
                "commit or stash them, rebuild stewards, and run the eval again",
            ));
        }
        git(root, &["rev-parse", "--verify", "HEAD^{commit}"])?
            .trim()
            .to_string()
    } else {
        commit.to_string()
    };
    for a in &plan.suite.absent {
        // a file the commit lacks holds no text
        let text = git(root, &["show", &format!("{commit}:{}", a.file)]).unwrap_or_default();
        if text.contains(&a.text) {
            return Err(setup_error(
                format!(
                    "task {}'s change is already in {commit}: {} contains `{}`",
                    a.task, a.file, a.text
                ),
                "pin a commit before the change in the suite's `commit`, or update the task and its `absent` probe",
            ));
        }
    }
    Ok(commit)
}

/// The one commit message of every `repo` suite arm's history.
const ROOT_COMMIT: &str = "initial";

/// A `repo` suite's `full` tree: `base` with this binary's AGENTS.md block upserted (as
/// `stewards init` does) and `allowed` added to `unowned_ok`, committed as the one root commit of `main`, every other ref deleted and
/// the reflog expired. The clone's object store stays, unpruned: the units' stamp and role-mark
/// trees are unreachable objects there, and `next` and the review pack diff from them; a unit
/// whose tree is missing is `eval_setup_incomplete`.
fn full_repo_tree(base: &Path, dir: &Path, allowed: &[String]) -> Result<()> {
    copy_dir(base, dir)?;
    let repo = crate::workspace::Repo {
        name: String::new(),
        root: dir.to_path_buf(),
        loaded: None,
    };
    crate::guide::upsert_block(&repo)?;
    allow_unowned(dir, allowed)?;
    git(dir, &["checkout", "-q", "--orphan", "root"])?;
    git(dir, &["add", "-A"])?;
    git(dir, &["commit", "-qm", ROOT_COMMIT])?;
    for r in git(dir, &["for-each-ref", "--format=%(refname)"])?.lines() {
        if r != "refs/heads/root" {
            git(dir, &["update-ref", "-d", r])?;
        }
    }
    git(dir, &["branch", "-m", "main"])?;
    git(dir, &["reflog", "expire", "--expire=now", "--all"])?;
    let missing: Vec<String> = crate::model::load_repo(dir)?
        .cards
        .values()
        .filter(|c| {
            c.reviewed.as_ref().is_some_and(|r| {
                [Some(&r.tree), r.tester.as_ref(), r.documenter.as_ref()]
                    .into_iter()
                    .flatten()
                    .any(|t| !crate::git::has_object(dir, t))
            })
        })
        .map(|c| c.unit.clone())
        .collect();
    if !missing.is_empty() {
        return Err(setup_error(
            format!(
                "the `full` tree at {} lacks the stamped trees of: {}",
                dir.display(),
                missing.join(", ")
            ),
            "pin a commit whose stamped trees are in the checkout holding evals/ (`git cat-file -e <tree>`)",
        ));
    }
    Ok(())
}

/// The paths a `repo` suite's `bare` tree lacks.
const STEWARDS_PATHS: [&str; 3] = [".stewards", "docs/units", "AGENTS.md"];

/// A `repo` suite's `bare` or `block-only` run copy: `base` without `STEWARDS_PATHS` (and
/// `block-only` with an AGENTS.md holding only the block) in a new repository whose one root
/// commit holds it, so no history or object can bring the removed paths back.
fn repo_arm_tree(base: &Path, dir: &Path, arm: Arm) -> Result<()> {
    copy_dir(base, dir)?;
    for p in STEWARDS_PATHS.iter().chain([&".git"]).map(|p| dir.join(p)) {
        if p.is_dir() {
            std::fs::remove_dir_all(&p)?;
        } else if p.exists() {
            std::fs::remove_file(&p)?;
        }
    }
    if arm == Arm::BlockOnly {
        std::fs::write(
            dir.join("AGENTS.md"),
            format!("{}\n", crate::guide::block()),
        )?;
    }
    git(dir, &["init", "-q", "-b", "main"])?;
    git(dir, &["add", "-A"])?;
    git(dir, &["commit", "-qm", ROOT_COMMIT])?;
    // as in `full`, which must expire the clone's
    git(dir, &["reflog", "expire", "--expire=now", "--all"])?;
    Ok(())
}

/// `cp -a --reflink=auto cache dir/target`: the one build's output, copied (sharing blocks where
/// the filesystem can) so no run pays a cold build and no run's in-place writes reach the cache.
fn link_build_cache(cache: &Path, dir: &Path) -> Result<()> {
    let target = dir.join("target");
    let out = Command::new("cp")
        .args(["-a", "--reflink=auto"])
        .arg(cache)
        .arg(&target)
        .output()?;
    ok(
        out,
        format!("cp -a {} {}", cache.display(), target.display()),
    )?;
    Ok(())
}

/// `path` with `stewards` off it: each directory holding a `stewards` is replaced by a directory
/// under `mirrors` linking to everything else in it, so other tools there (cargo, protoc) stay.
pub fn path_without_stewards(path: &OsStr, mirrors: &Path) -> Result<OsString> {
    let mut dirs = vec![];
    for (n, dir) in std::env::split_paths(path).enumerate() {
        if !dir.join("stewards").exists() {
            dirs.push(dir);
            continue;
        }
        let mirror = mirrors.join(n.to_string());
        if !mirror.exists() {
            std::fs::create_dir_all(&mirror)?;
            for entry in std::fs::read_dir(&dir)? {
                let entry = entry?;
                if entry.file_name() != "stewards" {
                    symlink(entry.path(), mirror.join(entry.file_name()))?;
                }
            }
        }
        dirs.push(mirror);
    }
    Ok(std::env::join_paths(dirs)?)
}

/// A scratch directory under `tmp` whose name says nothing about the eval: `w-<random hex>`.
pub fn neutral_work_dir(tmp: &Path) -> PathBuf {
    let random = std::collections::hash_map::RandomState::new().hash_one(SystemTime::now());
    tmp.join(format!("w-{random:016x}"))
}

fn usage(message: String) -> anyhow::Error {
    CliError::new("usage", message, "run `stewards eval --help`").into()
}

/// The CLI flags of `stewards eval`; clap checks `--runs` >= 1 and the thinking level, `run` the
/// suite and its task numbers.
pub struct Flags<'a> {
    pub harness: Option<&'a str>,
    pub suite: &'a str,
    pub arm: Option<&'a str>,
    pub tasks: &'a [u32],
    pub runs: u32,
    pub max_cost: Option<f64>,
    pub no_developers: bool,
    pub model: Option<&'a str>,
    pub thinking: &'a str,
    pub setup_dir: Option<&'a Path>,
    pub keep_work: bool,
}

pub fn run(g: &Global, f: Flags) -> Result<ExitCode> {
    let h = harness::by_name(f.harness.unwrap_or("pi"))?;
    if !matches!(h.name, "pi" | "claude-code" | "hermes") {
        return not_implemented(&format!("eval --harness {}", h.name));
    }
    if h.name != "pi" && f.max_cost.is_some() {
        return Err(usage(format!(
            "--max-cost needs a cost figure, and {}'s is not used (its cost_usd is null)",
            h.name
        )));
    }
    if h.name == "hermes" && !f.model.is_some_and(|m| m.contains('/')) {
        return Err(usage(
            "--harness hermes needs --model <provider>/<model>, such as deepseek/deepseek-flash"
                .into(),
        ));
    }
    let full = if f.no_developers {
        Arm::FullNoDevelopers
    } else {
        Arm::Full
    };
    let arms = match f.arm {
        None => vec![Arm::Bare, Arm::BlockOnly, full],
        Some("bare") => vec![Arm::Bare],
        Some("block-only") => vec![Arm::BlockOnly],
        Some("full") => vec![full],
        Some(other) => {
            return Err(usage(format!(
                "unknown arm `{other}`; valid: bare, block-only, full"
            )));
        }
    };
    let (cwd, ws) = crate::workspace::here(g)?;
    let root = ws.current_repo(&cwd)?.root.clone();
    let evals = root.join("evals");
    if !evals.join("arms.sh").is_file() {
        return Err(CliError::new(
            "usage",
            format!("no `evals/arms.sh` in {}", root.display()),
            "run `stewards eval` inside the stewards repository",
        )
        .into());
    }
    let suite = Suite::load(&evals, f.suite)?;
    let mut tasks = if f.tasks.is_empty() {
        suite.numbers()
    } else {
        f.tasks.to_vec()
    };
    tasks.sort_unstable();
    tasks.dedup();
    suite.check_tasks(&tasks)?;
    if f.setup_dir.is_some() && suite.commit().is_some() {
        return Err(usage(format!(
            "suite `{}` is already adopted; --setup-dir is for a fixture suite",
            suite.name
        )));
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    let limit = Duration::from_secs(suite.session_minutes * 60);
    let work = neutral_work_dir(&std::env::temp_dir());
    let (driver, harness_setup, thinking): (Box<dyn Driver>, String, &str) = if h.name == "pi" {
        let mut pi = pi::Pi::find(&path, limit)?;
        let note = pi.isolate(&work.join("a"), f.thinking)?;
        (Box::new(pi), note, f.thinking)
    } else if h.name == "hermes" {
        let mut hermes = hermes::Hermes::find(&path, limit)?;
        let note = hermes.isolate(&work.join("a"))?;
        (Box::new(hermes), note, f.thinking)
    } else {
        let mut claude = claude::ClaudeCode::find(&path, limit)?;
        let note = claude.isolate(&work.join("a"))?;
        (Box::new(claude), note, "off")
    };
    let stamp = crate::telemetry::rfc3339(SystemTime::now()).replace(':', "-");
    let plan = Plan {
        harness: h.name,
        tasks,
        arms,
        runs: f.runs,
        model: f.model.map(str::to_string),
        thinking: thinking.to_string(),
        max_cost: f.max_cost,
        setup_dir: f.setup_dir.map(Path::to_path_buf),
        results: evals.join("results").join(&stamp),
        work,
        keep_work: f.keep_work,
        evals,
        suite,
        stewards: std::env::current_exe()?,
        path,
        harness_setup: Some(harness_setup),
    };
    let summary = execute(driver.as_ref(), &plan)?;
    emit(&summary, g.human);
    Ok(if summary.verdict == Verdict::Pass {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

// `stewards eval`: drives a harness over the `evals/` fixture in each arm, parses every transcript
// into a `Run`, and grades the medians.
use std::collections::{BTreeMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::fmt::Display;
use std::hash::BuildHasher;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Output};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::harness::{self, EnvRule, HARNESSES};
use crate::output::{CliError, Global, emit, not_implemented};

pub mod grade;
pub mod pi;

pub use grade::{Summary, TaskGrade, Verdict};

/// The task numbers under `evals/tasks/`.
pub const TASKS: [u32; 6] = [1, 2, 3, 4, 5, 6];

/// Each session's wall-clock limit; a session still running then is recorded as timed out.
pub const SESSION_LIMIT: Duration = Duration::from_secs(30 * 60);

/// The `full` arm's setup, read in one place. The hand-set units, under `evals/`: on the fixture
/// `stewards init` proposes only `cli`, its one manifest root.
pub const FULL_UNITS: &str = "full-units.toml";
/// The only tracked files `FULL_UNITS` may leave unowned.
pub const FULL_UNITS_UNOWNED: [&str; 2] = [".gitignore", "AGENTS.md"];
/// Where adoption may add new files (a tester may add tests); any other change to the fixture's
/// code stops the eval. Modifying or deleting a file here still stops it.
pub const FULL_UNITS_TESTS: [&str; 3] = ["api/test_*.py", "client/*.test.ts", "cli/tests/**"];
/// The tool's own files, which adoption may add, change, or delete.
const ADOPTION_OWN: [&str; 4] = [".stewards/", "docs/units/", "AGENTS.md", ".gitignore"];

/// The prompt of the `full` arm's adoption session, run once per eval before any `full` run.
pub const ADOPT_PROMPT: &str = "Adopt stewards in this repository: run `stewards init --draft` and hand every brief it writes to a subagent that can run shell, then follow the AGENTS.md block: run `stewards next` and dispatch every brief it writes until it lists no jobs.";

/// The final stop reasons of a response that did not end normally.
pub const FAILED_STOPS: [&str; 3] = ["error", "aborted", "length"];

/// What the summary says about the transcript-derived ritual steps.
pub const RITUAL_NOTE: &str = "check_green is the runner's own `stewards next --no-brief` after each full session (exit 0), and counts only for a session that changed tracked files (else n/a). On the read-only tasks 4 and 6 route, brief, submit, and check_green are n/a; each run record still shows every step it ran. The other five steps are best-effort from the transcripts: a `stewards` command is seen only as the program of a `;`, `&`, `|`, `(`, `)`, or newline separated segment (after `VAR=value` words), so `timeout`/`env`/`time` prefixes, `if stewards ...`, `{ ...; }`, `bash -c '...'`, backticks, and lines inside heredocs are misread or missed; `status` and `route` count when only a child ran them; `brief` is a subagent launch whose arguments name `.stewards/briefs/`, or any `submit`.";

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

/// `task` runs are graded; `setup` runs (the adoption session, task 5 before task 6) are not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunRole {
    Task,
    Setup,
}

/// Token counts. Every count is the sum of the harness's per-response `input`, `output`,
/// `cache_read`, and `cache_write` usage fields; `sum` adds all four, and that is the number the
/// token criteria compare, in every arm.
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

/// The spec's six ritual steps, in order.
pub const RITUAL_STEPS: [&str; 6] = ["status", "route", "brief", "next", "submit", "check_green"];

/// Each of `RITUAL_STEPS` and whether the run showed it.
pub type RitualSteps = BTreeMap<&'static str, bool>;

/// One session tree parsed from its transcript.
#[derive(Debug, Clone, Serialize)]
pub struct Run {
    /// The parent session only.
    pub main_tokens: Tokens,
    /// The parent plus every child session.
    pub total_tokens: Tokens,
    /// Cache refreshes over the tree, which do not enter model context: not in the token sums
    /// above; their cost is in `cost_usd`.
    pub cache_warming: Tokens,
    /// The largest `Tokens::context` of any one main-session response.
    pub peak_main_context: u64,
    /// Tool names called in the main session, in order.
    pub tool_calls: Vec<String>,
    /// Distinct repo-relative paths the main session opened (see `pi::parse` for what counts).
    pub files_opened: Vec<String>,
    /// The same over the whole session tree.
    pub files_opened_tree: Vec<String>,
    /// Shell commands the main session ran, in order.
    pub shell_commands: Vec<String>,
    /// From the transcripts; `check_green` is set by the runner (`false` until then).
    pub ritual_steps: RitualSteps,
    /// The harness's own cost figure summed over the tree; `None` when the transcript has none.
    pub cost_usd: Option<f64>,
    pub duration_s: f64,
    /// `provider/model` of the main session, as the transcript names it.
    pub model: Option<String>,
    /// Every `provider/model` used anywhere in the session tree, sorted.
    pub models: Vec<String>,
    /// Every thinking level any session of the tree was set to, sorted.
    pub thinking_levels: Vec<String>,
    /// The stop reason of the main session's last response.
    pub stop_reason: Option<String>,
    /// Child session files (relative to the tree) whose last response stopped with one of
    /// `FAILED_STOPS`, with that reason.
    pub failed_children: Vec<String>,
    /// Unparseable last lines tolerated in the files of a failed or timed-out session.
    pub truncated_lines: usize,
}

/// One thing a session did, in harness-neutral form, for `ritual_steps`.
pub enum Action {
    Shell {
        command: String,
    },
    /// A subagent launch; `text` is its arguments.
    Dispatch {
        text: String,
    },
}

/// The transcript-derived ritual steps: `status`, `route`, `next`, `submit` are shell commands
/// whose argv starts with `stewards <cmd>`; `brief` is a subagent launched with a
/// `.stewards/briefs/` path or a `stewards submit`. `check_green` is left `false` for the runner.
pub fn ritual_steps(actions: &[Action]) -> RitualSteps {
    let mut steps: RitualSteps = RITUAL_STEPS.iter().map(|&s| (s, false)).collect();
    for a in actions {
        match a {
            Action::Shell { command } => {
                for cmd in stewards_subcommands(command) {
                    if let Some(step) = ["status", "route", "next", "submit"]
                        .into_iter()
                        .find(|s| *s == cmd)
                    {
                        steps.insert(step, true);
                    }
                }
            }
            Action::Dispatch { text } if text.contains(".stewards/briefs/") => {
                steps.insert("brief", true);
            }
            _ => {}
        }
    }
    if steps["submit"] {
        steps.insert("brief", true);
    }
    steps
}

/// Each `;`, `&`, `|`, `(`, `)`, or newline separated segment of a command line as words, quotes
/// trimmed, leading `VAR=value` words skipped. A separator or space inside `'...'` or `"..."`, or
/// after a backslash outside single quotes, does not split.
fn segments(command: &str) -> Vec<Vec<&str>> {
    let (mut segments, mut words) = (vec![], vec![]);
    let (mut start, mut quote, mut escaped) = (None, None, false);
    for (i, c) in command.char_indices() {
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

/// The `stewards` subcommands a shell command line runs: each segment (see `segments`) whose
/// program is `stewards`, skipping its global flags.
pub fn stewards_subcommands(command: &str) -> Vec<&str> {
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
                    _ => return Some(w),
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
/// after it that is not a flag, an option value, a number, a glob, a word with `$` or starting
/// with `!`, or a redirection (a lone `>`/`<` operator also drops the word after it). Option
/// values: `-n`/`-c` of `head`/`tail`, `-e`/`-f` of `sed`, `GREP_VALUE_FLAGS` of `grep`/`rg`.
/// `sed -n` drops its script and `grep`/`rg` their pattern (the first such word) unless `-e`/`-f`
/// gave it. Directories given to `grep`/`rg` count.
/// simplified: a path relative to an earlier `cd`, a line of a heredoc, or a `for` list is
/// misread; upgrade if `files_opened` disagrees with the transcripts.
pub fn shell_reads(command: &str) -> Vec<&str> {
    let mut out = vec![];
    for words in segments(command) {
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
                || w.contains(['<', '>', '*', '?', '$'])
                || w.chars().all(|c| c.is_ascii_digit()))
            {
                operands.push(w);
            }
        }
        out.extend(
            operands
                .into_iter()
                .skip(usize::from(drop_first && !pattern_given)),
        );
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
    /// Whether the session left a tracked file (or the commit) different from how it started;
    /// `check_green` counts only then.
    pub changed_tracked_files: bool,
    pub result: Option<Run>,
    /// Why `result` is missing when the transcript could not be parsed.
    pub error: Option<String>,
}

/// One eval invocation.
pub struct Plan {
    pub tasks: Vec<u32>,
    pub arms: Vec<Arm>,
    pub runs: u32,
    pub model: Option<String>,
    pub thinking: String,
    pub max_cost: Option<f64>,
    /// A previously adopted fixture to copy for the `full` arms instead of adopting again.
    pub setup_dir: Option<PathBuf>,
    /// The repo's `evals/` directory (fixture, tasks, arms.sh).
    pub evals: PathBuf,
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
    /// Starts one session unless `--max-cost` says stop, and records it. Returns whether it ran.
    fn session(&mut self, s: SessionSpec) -> Result<bool> {
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
        let before = tracked_state(&s.dir);
        let session = self.driver.start(
            &s.dir,
            &s.prompt,
            self.plan.model.as_deref(),
            &self.plan.thinking,
            &sessions,
            &env,
        )?;
        let root = s.dir.parent().unwrap_or(&s.dir).to_path_buf();
        let leftovers = kill_leftovers(&root)?;
        let changed_tracked_files = tracked_state(&s.dir) != before;
        let next_check = if s.arm.is_full() {
            Some(next_check(&self.plan.stewards, &s.dir)?)
        } else {
            None
        };
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
        if let Some(r) = &mut result {
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
            changed_tracked_files,
            result,
            error,
        };
        let runs = self.plan.results.join("runs");
        std::fs::create_dir_all(&runs)?;
        std::fs::write(
            runs.join(format!("{label}.json")),
            serde_json::to_string_pretty(&record)?,
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
        Ok(true)
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

/// Runs the plan sequentially: the adoption setup (when a `full` arm is planned), then per task,
/// per run number, each arm in a fresh fixture copy at `<work>/<n>/repo`; task 6 runs after task 5
/// in the same copy. Writes `runs/*.json` as each session ends, then `summary.json` and `summary.md`.
pub fn execute(driver: &dyn Driver, plan: &Plan) -> Result<Summary> {
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
    let mut adopted = None;
    if let Some(arm) = plan.arms.iter().copied().find(|a| a.is_full()) {
        let dir = match &plan.setup_dir {
            Some(d) => {
                notes.units = Some(format!(
                    "Units are those of the --setup-dir copy at {}; what `stewards init` proposed on its own was not recorded in this eval.",
                    d.display()
                ));
                check_setup_dir(d)?;
                d.clone()
            }
            None => {
                let dir = plan.work.join("0/repo");
                arms_sh(plan, "full", &dir, &path_on)?;
                notes.units = Some(set_full_units(&plan.evals, &dir)?);
                r.session(SessionSpec {
                    label: "adopt".into(),
                    task: None,
                    arm,
                    role: RunRole::Setup,
                    run: 1,
                    dir: dir.clone(),
                    prompt: ADOPT_PROMPT.into(),
                    path: path_on.clone(),
                })?;
                dir
            }
        };
        let fail = |r: &Runner, notes: SetupNotes, err: anyhow::Error| -> Result<Summary> {
            write_summary(
                plan,
                &grade::summarize(plan, &r.records, r.stopped, &r.unknown_cost, notes),
            )?;
            Err(err)
        };
        if plan.setup_dir.is_none() {
            git(&dir, &["add", "-A"])?;
        }
        match adoption_changes(&dir) {
            Ok(added) => notes.added_paths = added,
            Err(e) => return fail(&r, notes, e),
        }
        if let Err(e) = check_full_units(&dir) {
            return fail(&r, notes, e);
        }
        if next_check(&plan.stewards, &dir)?.exit_code != Some(0) {
            let err = CliError::new(
                "eval_setup_incomplete",
                format!(
                    "`stewards next` is not clean in the adopted fixture at {}; no `full` run started",
                    dir.display()
                ),
                format!(
                    "read the adoption transcript under {}, or pass `--setup-dir` an adopted copy where `stewards next` exits 0",
                    plan.results.join("sessions").display()
                ),
            );
            return fail(&r, notes, err.into());
        }
        (notes.gaps, notes.flows) = adopted_knowledge(&dir)?;
        if plan.setup_dir.is_none() {
            commit_adopted(&dir)?;
        }
        adopted = Some(dir);
    }
    let mut n = 0;
    'all: for &task in &plan.tasks {
        for run in 1..=plan.runs {
            for &arm in &plan.arms {
                n += 1;
                let label = format!("{}-t{task}-r{run}", arm.as_str());
                let root = plan.work.join(n.to_string());
                let dir = root.join("repo");
                let path = if arm.is_full() { &path_on } else { &path_off };
                match &adopted {
                    Some(from) if arm.is_full() => copy_adopted(from, &dir, arm)?,
                    _ => arms_sh(plan, arm.as_str(), &dir, path)?,
                }
                let spec = |task: u32, role: RunRole, label: String| -> Result<SessionSpec> {
                    Ok(SessionSpec {
                        label,
                        task: Some(task),
                        arm,
                        role,
                        run,
                        dir: dir.clone(),
                        prompt: task_prompt(&plan.evals, task)?,
                        path: path.clone(),
                    })
                };
                let ran = (task != 6
                    || r.session(spec(5, RunRole::Setup, format!("{label}-setup"))?)?)
                    && r.session(spec(task, RunRole::Task, label.clone())?)?;
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

/// The task file verbatim.
pub fn task_prompt(evals: &Path, task: u32) -> Result<String> {
    let dir = evals.join("tasks");
    let prefix = format!("{task:02}-");
    let mut names: Vec<PathBuf> = std::fs::read_dir(&dir)
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
    std::fs::read_to_string(file).with_context(|| file.display().to_string())
}

/// `out` when its command succeeded, else an error naming `what` with its stderr.
fn ok(out: Output, what: impl Display) -> Result<Output> {
    if !out.status.success() {
        bail!("{what}: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(out)
}

/// `evals/arms.sh <arm> <dir>`: the fixture copied, `git init`ed and committed, plus the arm's
/// AGENTS.md block or `stewards init`.
fn arms_sh(plan: &Plan, arm: &str, dir: &Path, path: &OsStr) -> Result<()> {
    let out = Command::new("bash")
        .arg(plan.evals.join("arms.sh"))
        .arg(arm)
        .arg(dir)
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

/// Replaces the units `stewards init` proposed in `dir` with `evals/FULL_UNITS`, and returns the
/// summary's plain-words note on what init proposed on its own.
fn set_full_units(evals: &Path, dir: &Path) -> Result<String> {
    let proposed: Vec<String> = crate::model::load_repo(dir)?
        .units
        .into_iter()
        .map(|u| u.name)
        .collect();
    let proposed_unowned = unowned_files(dir)?.len();
    std::fs::copy(evals.join(FULL_UNITS), dir.join(".stewards/units.toml"))
        .with_context(|| format!("copying evals/{FULL_UNITS}"))?;
    Ok(format!(
        "Units were set by hand for this eval from evals/{FULL_UNITS}, so these numbers do not measure `stewards init`'s unit proposal: on its own it proposed {} and left {proposed_unowned} tracked file(s) unowned. Unowned by design: {}.",
        proposed.join(", "),
        FULL_UNITS_UNOWNED.join(", ")
    ))
}

fn setup_error(message: String, fix: &str) -> anyhow::Error {
    CliError::new("eval_setup_incomplete", message, fix).into()
}

/// An `eval_setup_incomplete` error when `dir` leaves a tracked file unowned beyond `FULL_UNITS_UNOWNED`.
fn check_full_units(dir: &Path) -> Result<()> {
    let stray: Vec<String> = unowned_files(dir)?
        .into_iter()
        .filter(|p| !FULL_UNITS_UNOWNED.contains(&p.as_str()))
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
        &format!("give them a unit in evals/{FULL_UNITS}"),
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

/// Checks the index of the adopted fixture against its `fixture: initial` (root) commit: only the
/// tool's own files (`ADOPTION_OWN`) may change, and new files may appear under
/// `FULL_UNITS_TESTS`; anything else is `eval_setup_incomplete`. Returns the added paths outside
/// `.stewards/` and `docs/units/`.
fn adoption_changes(dir: &Path) -> Result<Vec<String>> {
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
    for t in FULL_UNITS_TESTS {
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
                "adoption changed the fixture's code in {} (against `fixture: initial`): {}",
                dir.display(),
                stray.join(", ")
            ),
            &format!(
                "read the adoption transcript; adoption may change only {} and add new files under {}",
                ADOPTION_OWN.join(", "),
                FULL_UNITS_TESTS.join(", ")
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

/// `HEAD` and the diff of the tracked files against it, or `None` when git cannot say.
fn tracked_state(dir: &Path) -> Option<(String, String)> {
    Some((
        git(dir, &["rev-parse", "HEAD"]).ok()?,
        git(dir, &["diff", "HEAD", "--binary"]).ok()?,
    ))
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

/// Kills every process whose working directory is under `dir` (from Linux `/proc/<pid>/cwd`),
/// again until none is left or 10 s pass. `None` where there is no `/proc`.
pub fn kill_leftovers(dir: &Path) -> Result<Option<Leftovers>> {
    if !Path::new("/proc/self/cwd").exists() {
        return Ok(None);
    }
    let dir = dir.canonicalize()?;
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
                        .is_ok_and(|cwd| cwd.starts_with(&dir))
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
            "user.name=fixture",
            "-c",
            "user.email=fixture@example.invalid",
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
/// name the tree, not a commit.
fn commit_adopted(dir: &Path) -> Result<()> {
    git(dir, &["clean", "-fdXq"])?;
    if !git(dir, &["status", "--porcelain"])?.trim().is_empty() {
        git(dir, &["commit", "-qm", "stewards: adopted"])?;
    }
    Ok(())
}

/// The developer lines of the AGENTS.md block, and their `--no-developers` replacements.
const DEVELOPER_LINE: &str = "- Changing: run `stewards brief <unit> --role developer` and dispatch every brief it writes, together when there are several. A small edit you may make yourself.";
const NO_DEVELOPER_LINE: &str = "- Changing: make the edits yourself.";
const DEVELOPER_FIX: &str = "run `stewards brief <unit> --role developer` again to fix them.";
const NO_DEVELOPER_FIX: &str = "fix them yourself.";

/// The AGENTS.md text with the developer role switched off; an error when the block's developer
/// lines are not found, so a changed block cannot silently leave developers on.
pub fn without_developers(agents_md: &str) -> Result<String> {
    for needle in [DEVELOPER_LINE, DEVELOPER_FIX] {
        if !agents_md.contains(needle) {
            bail!("AGENTS.md block has no `{needle}`");
        }
    }
    Ok(agents_md
        .replace(DEVELOPER_LINE, NO_DEVELOPER_LINE)
        .replace(DEVELOPER_FIX, NO_DEVELOPER_FIX))
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
        git(to, &["commit", "-qam", "eval: no developers"])?;
    }
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

/// The CLI flags of `stewards eval`; clap checks the task range, `--runs` >= 1, and the
/// thinking level.
pub struct Flags<'a> {
    pub harness: Option<&'a str>,
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
    if h.name != "pi" {
        return not_implemented(&format!("eval --harness {}", h.name));
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
    let mut tasks = if f.tasks.is_empty() {
        TASKS.to_vec()
    } else {
        f.tasks.to_vec()
    };
    tasks.sort_unstable();
    tasks.dedup();
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
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut pi = pi::Pi::find(&path, SESSION_LIMIT)?;
    let work = neutral_work_dir(&std::env::temp_dir());
    let harness_setup = pi.isolate(&work.join("a"), f.thinking)?;
    let stamp = crate::telemetry::rfc3339(SystemTime::now()).replace(':', "-");
    let plan = Plan {
        tasks,
        arms,
        runs: f.runs,
        model: f.model.map(str::to_string),
        thinking: f.thinking.to_string(),
        max_cost: f.max_cost,
        setup_dir: f.setup_dir.map(Path::to_path_buf),
        results: evals.join("results").join(&stamp),
        work,
        keep_work: f.keep_work,
        evals,
        stewards: std::env::current_exe()?,
        path,
        harness_setup: Some(harness_setup),
    };
    let summary = execute(&pi, &plan)?;
    emit(&summary, g.human);
    Ok(if summary.verdict == Verdict::Pass {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

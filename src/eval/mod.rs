// `stewards eval`: drives a harness over the `evals/` fixture in each arm, parses every transcript
// into a `Run`, and grades the medians.
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{Duration, SystemTime};

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

/// The hand-set units of the `full` arm, under `evals/`: on the fixture `stewards init` proposes
/// only `cli`, its one manifest root.
pub const FULL_UNITS: &str = "full-units.toml";
/// The only tracked files `FULL_UNITS` may leave unowned.
pub const FULL_UNITS_UNOWNED: [&str; 2] = [".gitignore", "AGENTS.md"];

/// The prompt of the `full` arm's adoption session, run once per eval before any `full` run.
pub const ADOPT_PROMPT: &str = "Adopt stewards in this repository: run `stewards init --draft` and hand every brief it writes to a subagent that can run shell, then follow the AGENTS.md block: run `stewards next` and dispatch every brief it writes until it lists no jobs.";

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

    pub fn add(&mut self, other: Tokens) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
    }
}

/// The ritual steps a run showed, over the main session and every child.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct RitualSteps {
    pub status: bool,
    pub route: bool,
    pub brief: bool,
    pub next: bool,
    pub submit: bool,
    pub check_green: bool,
}

impl RitualSteps {
    pub const NAMES: [&str; 6] = ["status", "route", "brief", "next", "submit", "check_green"];

    pub fn get(&self, name: &str) -> bool {
        match name {
            "status" => self.status,
            "route" => self.route,
            "brief" => self.brief,
            "next" => self.next,
            "submit" => self.submit,
            _ => self.check_green,
        }
    }
}

/// One session tree parsed from its transcript.
#[derive(Debug, Clone, Serialize)]
pub struct Run {
    /// The parent session only.
    pub main_tokens: Tokens,
    /// The parent plus every child session.
    pub total_tokens: Tokens,
    /// Tool names called in the main session, in order.
    pub tool_calls: Vec<String>,
    /// Distinct repo-relative paths the main session read with its read tool.
    pub files_opened: Vec<String>,
    /// Shell commands the main session ran, in order.
    pub shell_commands: Vec<String>,
    pub ritual_steps: RitualSteps,
    /// The harness's own cost figure summed over the tree; `None` when the transcript has none.
    pub cost_usd: Option<f64>,
    pub duration_s: f64,
    /// `provider/model` as the transcript names it.
    pub model: Option<String>,
}

/// One thing a session did, in harness-neutral form, for `ritual_steps`. `at` is an RFC 3339 UTC
/// timestamp in one fixed format, so timestamps compare as text.
pub enum Action {
    Shell {
        command: String,
        ok: bool,
        at: String,
    },
    /// A subagent launch; `text` is its arguments.
    Dispatch { text: String },
    /// A file write or edit through a tool.
    Edit { at: String },
}

/// The spec's six ritual steps: `status`, `route`, `next`, `submit` are shell commands whose argv
/// starts with `stewards <cmd>`; `brief` is a subagent launched with a `.stewards/briefs/` path or a
/// `stewards submit`; `check_green` is a `stewards next` that exited 0 no earlier than the last edit.
/// simplified: a piped `stewards next | head` reports the pipe's exit; edits made through shell
/// commands are not seen. Upgrade if the graders disagree with the transcripts.
pub fn ritual_steps(actions: &[Action]) -> RitualSteps {
    let mut steps = RitualSteps::default();
    let last_edit = actions
        .iter()
        .filter_map(|a| match a {
            Action::Edit { at } => Some(at.as_str()),
            _ => None,
        })
        .max();
    for a in actions {
        match a {
            Action::Shell { command, ok, at } => {
                for cmd in stewards_subcommands(command) {
                    match cmd {
                        "status" => steps.status = true,
                        "route" => steps.route = true,
                        "submit" => steps.submit = true,
                        "next" => {
                            steps.next = true;
                            if *ok && last_edit.is_none_or(|e| at.as_str() >= e) {
                                steps.check_green = true;
                            }
                        }
                        _ => {}
                    }
                }
            }
            Action::Dispatch { text } if text.contains(".stewards/briefs/") => steps.brief = true,
            _ => {}
        }
    }
    steps.brief |= steps.submit;
    steps
}

/// The `stewards` subcommands a shell command line runs: each `;`, `&`, `|`, `(`, `)`, or newline
/// separated segment whose program (after `VAR=value` words) is `stewards`, skipping its global flags.
pub fn stewards_subcommands(command: &str) -> Vec<&str> {
    command
        .split(['\n', ';', '&', '|', '(', ')'])
        .filter_map(|segment| {
            let mut words = segment
                .split_whitespace()
                .map(|w| w.trim_matches(['"', '\'']))
                .skip_while(|w| w.contains('='));
            let program = words.next()?;
            if Path::new(program).file_name() != Some(OsStr::new("stewards")) {
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

/// What a driver's session left behind.
pub struct Session {
    pub status: Status,
    /// The main session's transcript; `None` when the harness wrote none.
    pub transcript: Option<PathBuf>,
    pub duration_s: f64,
}

/// One harness. Graders see only the `Run` that `parse` returns.
pub trait Driver {
    /// Runs one session on `prompt` in `dir` (on `model`, else the harness default) with `PATH` set
    /// to `path`, keeps its transcript under `sessions`, and waits for it to end or time out.
    fn start(
        &self,
        dir: &Path,
        prompt: &str,
        model: Option<&str>,
        sessions: &Path,
        path: &OsStr,
    ) -> Result<Session>;

    fn parse(&self, transcript: &Path, duration_s: f64) -> Result<Run>;
}

/// Prepares a harness command to run as a fresh top-level session: drops every variable a harness
/// sets for its own children (an enclosing Pi's `PI_*` hides the `subagent` tool from a nested Pi)
/// and `STEWARDS_DIRECT`, and sets `PATH`.
pub fn fresh_session_env(cmd: &mut Command, path: &OsStr) {
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
    cmd.env("PATH", path);
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
    pub status: Status,
    pub transcript: Option<PathBuf>,
    pub workdir: PathBuf,
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
    pub max_cost: Option<f64>,
    /// A previously adopted fixture to copy for the `full` arms instead of adopting again.
    pub setup_dir: Option<PathBuf>,
    /// The repo's `evals/` directory (fixture, tasks, arms.sh).
    pub evals: PathBuf,
    /// Where `runs/`, `sessions/`, and the summaries go.
    pub results: PathBuf,
    /// Scratch space for the per-run fixture copies.
    pub work: PathBuf,
    /// The stewards binary the `full` arms put on `PATH`.
    pub stewards: PathBuf,
    /// The `PATH` to start from.
    pub path: OsString,
}

/// True when one more session, estimated at the mean of `costs` so far, would take the spend over
/// `max`. With no cost known yet there is no estimate, so the session runs.
pub fn would_exceed(max: f64, costs: &[f64]) -> bool {
    let spent: f64 = costs.iter().sum();
    !costs.is_empty() && spent + spent / costs.len() as f64 > max
}

struct Runner<'a> {
    driver: &'a dyn Driver,
    plan: &'a Plan,
    records: Vec<RunRecord>,
    costs: Vec<f64>,
    stopped: bool,
}

impl Runner<'_> {
    /// Starts one session unless `--max-cost` says stop, and records it. Returns whether it ran.
    fn session(&mut self, s: SessionSpec) -> Result<bool> {
        if self.stopped
            || (!self.records.is_empty()
                && self
                    .plan
                    .max_cost
                    .is_some_and(|m| would_exceed(m, &self.costs)))
        {
            self.stopped = true;
            return Ok(false);
        }
        let label = format!("{:03}-{}", self.records.len() + 1, s.label);
        let sessions = self.plan.results.join("sessions").join(&label);
        let session = self.driver.start(
            &s.dir,
            &s.prompt,
            self.plan.model.as_deref(),
            &sessions,
            &s.path,
        )?;
        let (result, error) = match &session.transcript {
            None => (None, Some("no transcript".to_string())),
            Some(t) => match self.driver.parse(t, session.duration_s) {
                Ok(run) => (Some(run), None),
                Err(e) => (None, Some(format!("{e:#}"))),
            },
        };
        if let Some(c) = result.as_ref().and_then(|r| r.cost_usd) {
            self.costs.push(c);
        }
        let record = RunRecord {
            label: label.clone(),
            task: s.task,
            arm: s.arm,
            role: s.role,
            run: s.run,
            model: self
                .plan
                .model
                .clone()
                .or_else(|| result.as_ref().and_then(|r| r.model.clone())),
            status: session.status,
            transcript: session.transcript,
            workdir: s.dir,
            result,
            error,
        };
        let runs = self.plan.results.join("runs");
        std::fs::create_dir_all(&runs)?;
        std::fs::write(
            runs.join(format!("{label}.json")),
            serde_json::to_string_pretty(&record)?,
        )?;
        self.records.push(record);
        Ok(true)
    }
}

struct SessionSpec {
    label: String,
    task: Option<u32>,
    arm: Arm,
    role: RunRole,
    run: u32,
    dir: PathBuf,
    prompt: String,
    path: OsString,
}

/// Runs the plan sequentially: the adoption setup (when a `full` arm is planned), then per task,
/// per run number, each arm in a fresh fixture copy; task 6 runs after task 5 in the same copy.
/// Writes `runs/*.json` as each session ends, then `summary.json` and `summary.md`.
pub fn execute(driver: &dyn Driver, plan: &Plan) -> Result<Summary> {
    std::fs::create_dir_all(&plan.work)?;
    let path_off = path_without_stewards(&plan.path, &plan.work.join("path"))?;
    let bin = plan.work.join("bin");
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
        stopped: false,
    };
    let full_arm = plan.arms.iter().copied().find(|a| a.is_full());
    let mut adopted = None;
    let mut units_note = None;
    if let Some(arm) = full_arm {
        let dir = match &plan.setup_dir {
            Some(d) => {
                units_note = Some(format!(
                    "Units are those of the --setup-dir copy at {}; what `stewards init` proposed on its own was not recorded in this eval.",
                    d.display()
                ));
                d.clone()
            }
            None => {
                let dir = plan.work.join("adopted");
                arms_sh(plan, "full", &dir, &path_on)?;
                units_note = Some(set_full_units(&plan.evals, &dir)?);
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
        check_full_units(&dir)?;
        if !next_is_clean(&plan.stewards, &dir)? {
            let summary = grade::summarize(plan, &r.records, r.stopped, units_note);
            write_summary(plan, &summary)?;
            return Err(CliError::new(
                "eval_setup_incomplete",
                format!(
                    "`stewards next` is not clean in the adopted fixture at {}; no `full` run started",
                    dir.display()
                ),
                format!(
                    "read the adoption transcript under {}, or pass `--setup-dir` an adopted copy where `stewards next` exits 0",
                    plan.results.join("sessions").display()
                ),
            )
            .into());
        }
        if plan.setup_dir.is_none() {
            commit_adopted(&dir)?;
        }
        adopted = Some(dir);
    }
    'all: for &task in &plan.tasks {
        for run in 1..=plan.runs {
            for &arm in &plan.arms {
                let label = format!("{}-t{task}-r{run}", arm.as_str());
                let dir = plan.work.join(&label);
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
                if task == 6 && !r.session(spec(5, RunRole::Setup, format!("{label}-setup"))?)? {
                    break 'all;
                }
                if !r.session(spec(task, RunRole::Task, label.clone())?)? {
                    break 'all;
                }
            }
        }
    }
    let summary = grade::summarize(plan, &r.records, r.stopped, units_note);
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
    if !out.status.success() {
        bail!(
            "evals/arms.sh {arm} {}: {}",
            dir.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// The tracked files `stewards check` reports as unowned in `dir`.
pub fn unowned_files(dir: &Path) -> Result<Vec<String>> {
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
    check_full_units(dir)?;
    Ok(format!(
        "Units were set by hand for this eval from evals/{FULL_UNITS}, so these numbers do not measure `stewards init`'s unit proposal: on its own it proposed {} and left {proposed_unowned} tracked file(s) unowned. Unowned by design: {}.",
        proposed.join(", "),
        FULL_UNITS_UNOWNED.join(", ")
    ))
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
    Err(CliError::new(
        "eval_setup_incomplete",
        format!(
            "the adopted fixture at {} leaves tracked files unowned: {}",
            dir.display(),
            stray.join(", ")
        ),
        format!("give them a unit in evals/{FULL_UNITS}"),
    )
    .into())
}

/// Whether `stewards next` lists no jobs and no check failures in `dir` (exit 0); writes no briefs.
pub fn next_is_clean(stewards: &Path, dir: &Path) -> Result<bool> {
    let out = Command::new(stewards)
        .args(["next", "--no-brief"])
        .current_dir(dir)
        .env_remove("STEWARDS_DIRECT")
        .output()
        .with_context(|| format!("running {} next", stewards.display()))?;
    Ok(out.status.success())
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
    if !out.status.success() {
        bail!(
            "git {args:?} in {}: {}",
            dir.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Commits the adopted state, after dropping the build output `evals/arms.sh` also drops, so each
/// `full` run starts from a clean tree as `bare` does. Stamps survive: they name the tree, not a commit.
fn commit_adopted(dir: &Path) -> Result<()> {
    for build in ["cli/target", "api/__pycache__"] {
        match std::fs::remove_dir_all(dir.join(build)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
    }
    match std::fs::remove_file(dir.join("api/items.json")) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
        _ => {}
    }
    git(dir, &["add", "-A"])?;
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

fn copy_adopted(from: &Path, to: &Path, arm: Arm) -> Result<()> {
    std::fs::create_dir_all(to)?;
    let out = Command::new("cp")
        .arg("-a")
        .arg(from.join("."))
        .arg(to)
        .output()?;
    if !out.status.success() {
        bail!(
            "cp -a {} {}: {}",
            from.display(),
            to.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    if arm == Arm::FullNoDevelopers {
        let agents = to.join("AGENTS.md");
        let text = std::fs::read_to_string(&agents).context("AGENTS.md")?;
        std::fs::write(&agents, without_developers(&text)?)?;
        git(to, &["commit", "-qam", "eval: no developers"])?;
    }
    Ok(())
}

#[cfg(unix)]
fn symlink(from: &Path, to: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(from, to)
}

#[cfg(not(unix))]
fn symlink(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::copy(from, to).map(|_| ())
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
                    symlink(&entry.path(), &mirror.join(entry.file_name()))?;
                }
            }
        }
        dirs.push(mirror);
    }
    Ok(std::env::join_paths(dirs)?)
}

fn usage(message: String) -> anyhow::Error {
    CliError::new("usage", message, "run `stewards eval --help`").into()
}

/// The CLI flags of `stewards eval`.
pub struct Flags<'a> {
    pub harness: Option<&'a str>,
    pub arm: Option<&'a str>,
    pub tasks: &'a [u32],
    pub runs: u32,
    pub max_cost: Option<f64>,
    pub no_developers: bool,
    pub model: Option<&'a str>,
    pub setup_dir: Option<&'a Path>,
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
    if let Some(t) = f.tasks.iter().find(|t| !TASKS.contains(t)) {
        return Err(usage(format!("no task {t}; tasks are 1 to 6")));
    }
    if f.runs == 0 {
        return Err(usage("`--runs` must be at least 1".into()));
    }
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
    let pi = pi::Pi::find(&path, SESSION_LIMIT)?;
    let stamp = crate::telemetry::rfc3339(SystemTime::now()).replace(':', "-");
    let plan = Plan {
        tasks,
        arms,
        runs: f.runs,
        model: f.model.map(str::to_string),
        max_cost: f.max_cost,
        setup_dir: f.setup_dir.map(Path::to_path_buf),
        results: evals.join("results").join(&stamp),
        work: std::env::temp_dir().join(format!("stewards-eval-{stamp}")),
        evals,
        stewards: std::env::current_exe()?,
        path,
    };
    let summary = execute(&pi, &plan)?;
    emit(&summary, g.human);
    Ok(if summary.verdict == Verdict::Pass {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

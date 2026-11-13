// Launching a brief: the harness's own headless session reads it and runs `submit`. The outcome
// comes from the repository (the brief retired and the result file an accepted submit writes),
// never from what the session said.
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::{Value, json};

use crate::brief;
use crate::check::one_line;
use crate::eval::{Tokens, fresh_session_env, pi};

use crate::harness::{self, Harness, Usage};
use crate::model::Role;
use crate::output::{CliError, Global, Human, emit};
use crate::telemetry::{self, LaunchRecord, ReviewedUnit};
use crate::user::{self, Config};
use crate::workspace::{Repo, safe_path};

/// Set to `1` in every launched session: `review` and `--run` do nothing inside one.
pub const CHILD_ENV: &str = "STEWARDS_CHILD";
pub const DEFAULT_TIMEOUT_S: u64 = 1800;
/// The most of a session's captured output its log keeps: the last bytes.
const LOG_CAP: u64 = 256 * 1024;
/// How much of a session's last message an outcome shows, as finding text is capped.
const TAIL_SHOWN: usize = 300;
/// The largest result file read.
const RESULT_BYTES: u64 = 256 * 1024;

/// The manual path, for a job that cannot be launched.
pub const MANUAL_PATH: &str = "run `stewards next` (or the same command without `--run`) and give each brief it writes to a fresh subagent that can run shell, with exactly: Read <absolute brief path> and follow it.";

/// The one prompt a launched session gets.
pub fn prompt(brief: &Path) -> String {
    format!("Read {} and follow it.", brief.display())
}

pub fn inside_child() -> bool {
    std::env::var_os(CHILD_ENV).is_some_and(|v| !v.is_empty())
}

#[derive(Serialize)]
struct Skipped {
    skipped: String,
}

impl Human for Skipped {
    fn render(&self) -> String {
        self.skipped.clone()
    }
}

/// Inside a launched session, prints why `what` does nothing and returns the exit code (0);
/// `None` elsewhere.
pub fn guard(g: &Global, what: &str) -> Option<ExitCode> {
    if !inside_child() {
        return None;
    }
    emit(
        &Skipped {
            skipped: format!(
                "`stewards {what}` does nothing here: this is a session stewards launched ({CHILD_ENV} is set), whose job is its brief; the session that launched it runs the rest"
            ),
        },
        g.human,
    );
    Some(ExitCode::SUCCESS)
}

/// The flags `review` and every `--run` take.
pub struct Options<'a> {
    pub harness: Option<&'a str>,
    pub model: Option<&'a str>,
    pub timeout_s: u64,
}

fn no_launcher(message: String) -> anyhow::Error {
    CliError::new("no_launcher", message, MANUAL_PATH).into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// A review whose every unit was approved.
    Approved,
    /// A review with findings.
    Changes,
    /// Any other accepted submit.
    Done,
    /// The session ended without an accepted submit.
    NotSubmitted,
    /// The session could not be started (its launch line has an unknown part, or the harness
    /// would not start), or it exited non-zero without an accepted submit.
    Failed,
    TimedOut,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Approved => "approved",
            Status::Changes => "changes",
            Status::Done => "done",
            Status::NotSubmitted => "not_submitted",
            Status::Failed => "failed",
            Status::TimedOut => "timed_out",
        }
    }

    /// The job ended in an accepted submit.
    pub fn submitted(self) -> bool {
        matches!(self, Status::Approved | Status::Changes | Status::Done)
    }
}

/// One launched job.
#[derive(Debug, Serialize)]
pub struct Outcome {
    pub role: Role,
    pub unit: Option<String>,
    /// The units a reviewer job covers.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub units: Vec<String>,
    pub brief_path: PathBuf,
    pub status: Status,
    /// What the job's `submit` printed; `None` without an accepted submit.
    pub result: Option<Value>,
    /// Without an accepted submit: the end of the session's last message, on one line, or why
    /// it could not be started.
    pub tail: Option<String>,
    /// A session that could not be started: the manual path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<&'static str>,
    /// The session's captured output (its last `LOG_CAP` bytes).
    pub log: PathBuf,
    pub seconds: f64,
    /// What the harness reported; `None` when it reported nothing.
    pub usage: Option<Tokens>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

impl Human for Outcome {
    fn render(&self) -> String {
        let who = match (&self.unit, self.units.is_empty()) {
            (Some(u), _) => format!("{u} {}", self.role.as_str()),
            (None, false) => format!("{} ({})", self.role.as_str(), self.units.join(", ")),
            (None, true) => self.role.as_str().to_string(),
        };
        let mut out = vec![format!(
            "- `{}`: {} ({:.0} s)",
            one_line(&who),
            self.status.as_str(),
            self.seconds
        )];
        let r = self.result.as_ref();
        let text = |k: &str| r.and_then(|r| r[k].as_str()).map(one_line);
        out.extend(text("summary").map(|s| format!("  {s}")));
        out.extend(text("explanation").map(|s| format!("  {s}")));
        for f in r
            .and_then(|r| r["findings"].as_array())
            .into_iter()
            .flatten()
        {
            out.push(one_line(&format!(
                "  - {} {}:{}: cut {}; replace with {}",
                f["unit"].as_str().unwrap_or_default(),
                f["file"].as_str().unwrap_or_default(),
                f["line"],
                f["cut"].as_str().unwrap_or_default(),
                f["replace_with"].as_str().unwrap_or_default()
            )));
        }
        out.extend(text("blocked_on").map(|s| format!("  blocked on: {s}")));
        out.extend(text("warning").map(|s| format!("  warning: {s}")));
        out.extend(self.tail.as_ref().map(|t| format!("  last message: {t}")));
        out.extend(self.fix.map(|f| format!("  fix: {f}")));
        out.push(format!("  log: {}", self.log.display()));
        out.join("\n")
    }
}

/// A resolved harness, the model pins, and the per-job wall-clock limit.
pub struct Launcher {
    pub harness: &'static Harness,
    /// The stewards command launching, for telemetry.
    command: &'static str,
    model: Option<String>,
    config: Config,
    timeout: Duration,
}

/// How a session ended.
enum Ended {
    NotStarted(String),
    Exited(bool),
    TimedOut,
}

impl Launcher {
    /// The harness: `--harness`, else the one detected from env, else `harness` in the user
    /// config; `no_launcher` when none is named.
    pub fn new(o: &Options, command: &'static str) -> Result<Launcher> {
        let config = user::load_config()?;
        let chosen = match o.harness {
            Some(name) => Some(harness::by_name(name)?),
            None => match harness::detect() {
                Some(h) => Some(h),
                None => config
                    .harness
                    .as_deref()
                    .map(harness::by_name)
                    .transpose()?,
            },
        };
        let Some(h) = chosen else {
            return Err(no_launcher(
                "no harness given (`--harness`), detected from the environment, or set as `harness` in the user config".into(),
            ));
        };
        Ok(Launcher {
            harness: h,
            command,
            model: o.model.map(String::from),
            config,
            timeout: Duration::from_secs(o.timeout_s),
        })
    }

    /// Starts the harness's headless session on `brief` (absolute, in `repo`) in the repo root,
    /// waits for it (killing its process group after `timeout`), and reads the outcome from the
    /// repository. Appends one telemetry record.
    pub fn run_job(&self, repo: &Repo, brief: &Path) -> Result<Outcome> {
        let rel = brief
            .strip_prefix(&repo.root)
            .ok()
            .and_then(Path::to_str)
            .with_context(|| format!("{} is not in {}", brief.display(), repo.root.display()))?
            .to_string();
        let header = brief::read_header(brief)?;
        let result_rel = brief::result_rel(&rel);
        let log_rel = format!("{}.log", rel.strip_suffix(".md").context("a brief is .md")?);
        repo.remove(&result_rel)?;
        repo.remove(&log_rel)?;
        let log_path = safe_path(&repo.root, &log_rel, true)?;
        // create_new never follows a link; 0600, since the output may quote the repository
        let log = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&log_path)
            .with_context(|| format!("create {}", log_path.display()))?;
        let model = self.model.clone().or_else(|| {
            self.config
                .model(self.harness.name, header.role.as_str())
                .map(String::from)
        });
        let launch = &self.harness.launch;
        let mut env = vec![(CHILD_ENV, OsString::from("1"))];
        for k in launch.keep_env {
            env.extend(std::env::var_os(k).map(|v| (*k, v)));
        }
        let started = Instant::now();
        let ended = match launch.argv(&repo.root, model.as_deref(), &prompt(brief)) {
            Err(part) => Ended::NotStarted(format!(
                "the `{}` launch line has no known `{part}`, so stewards cannot start it",
                self.harness.name
            )),
            Ok((program, args, set)) => {
                env.extend(set.iter().map(|(k, v)| (*k, OsString::from(v))));
                let mut cmd = Command::new(program);
                cmd.args(args)
                    .current_dir(&repo.root)
                    .stdin(Stdio::null())
                    .stdout(log.try_clone()?)
                    .stderr(log);
                fresh_session_env(&mut cmd, &env);
                cmd.process_group(0);
                match cmd.spawn() {
                    Err(e) => Ended::NotStarted(format!("starting `{program}`: {e}")),
                    Ok(mut child) => loop {
                        if let Some(s) = child.try_wait()? {
                            // what the session left running in its group goes with it
                            pi::kill(&mut child);
                            break Ended::Exited(s.success());
                        }
                        if started.elapsed() >= self.timeout {
                            pi::kill(&mut child);
                            break Ended::TimedOut;
                        }
                        std::thread::sleep(Duration::from_millis(200));
                    },
                }
            }
        };
        let seconds = started.elapsed().as_secs_f64();
        let (usage, last) = read_output(&log_path, launch.usage.as_ref());
        keep_tail(repo, &log_rel, &log_path)?;

        let result = if brief.exists() {
            None
        } else {
            read_result(&repo.root, &result_rel)
        };
        repo.remove(&result_rel)?;
        let status = match (&result, &ended) {
            (Some(r), _) => status_of(r),
            (None, Ended::TimedOut) => Status::TimedOut,
            (None, Ended::NotStarted(_) | Ended::Exited(false)) => Status::Failed,
            (None, Ended::Exited(true)) => Status::NotSubmitted,
        };
        let fix = matches!((&result, &ended), (None, Ended::NotStarted(_))).then_some(MANUAL_PATH);
        let tail = match (&result, ended) {
            (Some(_), _) => None,
            (None, Ended::NotStarted(why)) => Some(why),
            (None, _) => last.map(|t| {
                let t = one_line(&t);
                let skip = t.chars().count().saturating_sub(TAIL_SHOWN);
                t.chars().skip(skip).collect()
            }),
        };
        let reviewed = result.as_ref().map_or(vec![], reviewed_units);
        let record = LaunchRecord {
            event: telemetry::LAUNCH_EVENT,
            command: self.command,
            role: header.role.as_str(),
            unit: header.unit.clone(),
            units: header.units.clone().unwrap_or_default(),
            status: status.as_str(),
            seconds,
            usage,
            reviewed,
            harness: self.harness.name,
            timestamp: telemetry::rfc3339(SystemTime::now()),
        };
        let warning = telemetry::append_launch(repo, &record)
            .err()
            .map(|e| format!("telemetry not written: {e:#}"));
        Ok(Outcome {
            role: header.role,
            unit: header.unit,
            units: header.units.unwrap_or_default(),
            brief_path: brief.to_path_buf(),
            status,
            result,
            tail,
            fix,
            log: log_path,
            seconds,
            usage,
            warning,
        })
    }
}

/// The usage the session reported where `usage` says, and its last message (else its last
/// non-empty output line).
fn read_output(path: &Path, usage: Option<&Usage>) -> (Option<Tokens>, Option<String>) {
    let Ok(file) = File::open(path) else {
        return (None, None);
    };
    let (mut total, mut text, mut last_line): (Option<Tokens>, Option<String>, Option<String>) =
        (None, None, None);
    for line in BufReader::new(file).split(b'\n').map_while(Result::ok) {
        let shown = String::from_utf8_lossy(&line);
        if !shown.trim().is_empty() {
            last_line = Some(shown.trim().to_string());
        }
        let Some(u) = usage else { continue };
        let Ok(v) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        if !u
            .when
            .iter()
            .all(|(at, want)| v.pointer(at) == Some(&json!(want)))
        {
            continue;
        }
        let counts = &v.pointer(u.at).cloned().unwrap_or_default();
        let n = |k: &str| {
            let at = (!k.is_empty()).then(|| counts.pointer(k)).flatten();
            at.and_then(Value::as_u64).unwrap_or(0)
        };
        let [input, output, cache_read, cache_write] = u.keys.map(n);
        let line_usage = Tokens {
            input,
            output,
            cache_read,
            cache_write,
        };
        match total.as_mut().filter(|_| u.sum) {
            Some(t) => t.add(line_usage),
            None => total = Some(line_usage),
        }
        text = match u.text.and_then(|t| v.pointer(t)) {
            Some(Value::String(s)) => Some(s.clone()),
            Some(Value::Array(blocks)) => Some(
                blocks
                    .iter()
                    .filter_map(|b| b["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            _ => text,
        };
    }
    (total, text.filter(|t| !t.trim().is_empty()).or(last_line))
}

/// Cuts the log at `rel` to its last `LOG_CAP` bytes, rewritten through `Repo::write`.
fn keep_tail(repo: &Repo, rel: &str, path: &Path) -> Result<()> {
    let mut file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let len = file.metadata()?.len();
    if len <= LOG_CAP {
        return Ok(());
    }
    file.seek(SeekFrom::Start(len - LOG_CAP))?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail)?;
    repo.write(rel, &tail)
}

/// The result file at `rel` as JSON; `None` when it is absent, too large, or not JSON.
fn read_result(root: &Path, rel: &str) -> Option<Value> {
    let path = safe_path(root, rel, false).ok()?;
    let meta = std::fs::symlink_metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > RESULT_BYTES {
        return None;
    }
    serde_json::from_slice(&std::fs::read(&path).ok()?).ok()
}

/// `approved` or `changes` for a review's result (one with `units`), else `done`.
fn status_of(result: &Value) -> Status {
    match result["units"].as_array().filter(|u| !u.is_empty()) {
        None => Status::Done,
        Some(units) if units.iter().any(|u| u["verdict"] == "changes") => Status::Changes,
        Some(_) => Status::Approved,
    }
}

/// Each unit of a review's result, with its verdict, finding count, and reported check.
fn reviewed_units(result: &Value) -> Vec<ReviewedUnit> {
    let findings = result["findings"].as_array().map_or(&[][..], Vec::as_slice);
    result["units"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|u| {
            let unit = u["unit"].as_str()?;
            Some(ReviewedUnit {
                unit: unit.to_string(),
                verdict: u["verdict"].as_str()?.to_string(),
                findings: findings.iter().filter(|f| f["unit"] == unit).count(),
                check_reported: u["check_reported"].as_bool(),
            })
        })
        .collect()
}

/// What a `--run` printed.
#[derive(Serialize)]
pub struct Launched {
    pub harness: &'static str,
    pub jobs: Vec<Outcome>,
}

impl Human for Launched {
    fn render(&self) -> String {
        let mut out = vec![format!("## Launched ({})\n", self.harness)];
        out.extend(self.jobs.iter().map(Outcome::render));
        out.join("\n")
    }
}

/// 0 when every job ended in an accepted submit, else 1.
pub fn exit_code(jobs: &[Outcome]) -> ExitCode {
    if jobs.iter().all(|j| j.status.submitted()) {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

/// Runs `briefs` of `repo` one after another and prints what each ended in.
pub fn run_briefs(g: &Global, l: &Launcher, repo: &Repo, briefs: &[PathBuf]) -> Result<ExitCode> {
    let jobs = briefs
        .iter()
        .map(|b| l.run_job(repo, b))
        .collect::<Result<Vec<_>>>()?;
    let code = exit_code(&jobs);
    emit(
        &Launched {
            harness: l.harness.name,
            jobs,
        },
        g.human,
    );
    Ok(code)
}

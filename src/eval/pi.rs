// The Pi driver: `pi --mode json` with pi-subagents, and its session transcripts.
use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs::File;
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde_json::Value;

use super::{Action, Driver, Run, Session, Status, Tokens, fresh_session_env, ritual_steps};
use crate::output::CliError;

pub struct Pi {
    pub bin: PathBuf,
    /// A session still running after this is killed and recorded as timed out.
    pub limit: Duration,
}

impl Pi {
    /// The `pi` on `path`, or a `harness_missing` error.
    pub fn find(path: &OsStr, limit: Duration) -> Result<Pi> {
        std::env::split_paths(path)
            .map(|d| d.join("pi"))
            .find(|p| p.is_file())
            .map(|bin| Pi { bin, limit })
            .ok_or_else(|| {
                CliError::new(
                    "harness_missing",
                    "`pi` is not on PATH",
                    "install Pi (`npm install -g @earendil-works/pi-coding-agent`) and pi-subagents (`pi install npm:pi-subagents`)",
                )
                .into()
            })
    }
}

impl Driver for Pi {
    /// `pi --mode json --session-dir <sessions> --exclude-tools subagents_enable [--model m] -- <prompt>`
    /// in `dir`: excluding the loader keeps `subagent` active from the first request. Pi stays up
    /// until its background subagents finish. stdout and stderr go to `stdout.log` and `stderr.log`.
    fn start(
        &self,
        dir: &Path,
        prompt: &str,
        model: Option<&str>,
        sessions: &Path,
        path: &OsStr,
    ) -> Result<Session> {
        std::fs::create_dir_all(sessions)?;
        let mut cmd = Command::new(&self.bin);
        cmd.current_dir(dir)
            .args(["--mode", "json", "--session-dir"])
            .arg(sessions)
            .args(["--exclude-tools", "subagents_enable"]);
        if let Some(m) = model {
            cmd.args(["--model", m]);
        }
        cmd.arg("--")
            .arg(prompt)
            .stdin(Stdio::null())
            .stdout(File::create(sessions.join("stdout.log"))?)
            .stderr(File::create(sessions.join("stderr.log"))?);
        fresh_session_env(&mut cmd, path);
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        let started = Instant::now();
        let mut child = cmd
            .spawn()
            .with_context(|| format!("starting {}", self.bin.display()))?;
        let status = loop {
            if let Some(s) = child.try_wait()? {
                break if s.success() {
                    Status::Ok
                } else {
                    Status::Failed
                };
            }
            if started.elapsed() >= self.limit {
                kill(&mut child);
                break Status::TimedOut;
            }
            std::thread::sleep(Duration::from_millis(200));
        };
        Ok(Session {
            status,
            transcript: main_transcript(sessions)?,
            duration_s: started.elapsed().as_secs_f64(),
        })
    }

    fn parse(&self, transcript: &Path, duration_s: f64) -> Result<Run> {
        parse(transcript, duration_s)
    }
}

/// Kills the session's whole process group, then the child itself.
fn kill(child: &mut Child) {
    #[cfg(unix)]
    let _ = Command::new("kill")
        .args(["-KILL", &format!("-{}", child.id())])
        .status();
    let _ = child.kill();
    let _ = child.wait();
}

/// The parent session file: the `.jsonl` directly in the session directory.
fn main_transcript(sessions: &Path) -> Result<Option<PathBuf>> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(sessions)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension() == Some(OsStr::new("jsonl")))
        .collect();
    files.sort();
    Ok(files.into_iter().next())
}

fn records(path: &Path) -> Result<Vec<Value>> {
    let text = std::fs::read_to_string(path).with_context(|| path.display().to_string())?;
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .enumerate()
        .map(|(n, l)| {
            serde_json::from_str(l).with_context(|| format!("{}:{}", path.display(), n + 1))
        })
        .collect()
}

/// Every child session file: `*.jsonl` anywhere under `<transcript without .jsonl>/`, sorted, so
/// a session file always comes before the directory of its own children and forks.
fn child_transcripts(transcript: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension() == Some(OsStr::new("jsonl")) {
                out.push(p);
            }
        }
    }
    let mut out = vec![];
    walk(&transcript.with_extension(""), &mut out);
    out.sort();
    out
}

/// The usage object a record carries: an assistant message's `message.usage`, or the top-level
/// `usage` of `usage`, `compaction`, and `branch_summary` records.
fn usage(r: &Value) -> Option<&Value> {
    let u = if r["type"] == "message" {
        if r["message"]["role"] != "assistant" {
            return None;
        }
        &r["message"]["usage"]
    } else {
        &r["usage"]
    };
    u.is_object().then_some(u)
}

fn tokens(u: &Value) -> Tokens {
    let n = |k: &str| u[k].as_u64().unwrap_or(0);
    Tokens {
        input: n("input"),
        output: n("output"),
        cache_read: n("cacheRead"),
        cache_write: n("cacheWrite"),
    }
}

/// A path the read tool opened, relative to the session's `cwd`; `None` outside it.
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
            _ => return None,
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// The actions of one session's records, in order: tool calls from assistant messages, matched to
/// their tool results by id for shell exits.
fn actions(records: &[&Value]) -> Vec<Action> {
    let mut calls = std::collections::HashMap::new();
    let mut out = vec![];
    for r in records {
        let m = &r["message"];
        let at = r["timestamp"].as_str().unwrap_or_default().to_string();
        if m["role"] == "assistant" {
            for b in m["content"].as_array().into_iter().flatten() {
                if b["type"] != "toolCall" {
                    continue;
                }
                match b["name"].as_str() {
                    Some("subagent") => out.push(Action::Dispatch {
                        text: b["arguments"].to_string(),
                    }),
                    Some("edit" | "write") => out.push(Action::Edit { at: at.clone() }),
                    _ => {}
                }
                calls.insert(b["id"].as_str().unwrap_or_default().to_string(), b);
            }
        } else if m["role"] == "toolResult"
            && let Some(call) = calls.get(m["toolCallId"].as_str().unwrap_or_default())
            && call["name"] == "bash"
        {
            out.push(Action::Shell {
                command: call["arguments"]["command"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                ok: m["isError"] != true,
                at,
            });
        }
    }
    out
}

/// Parses a Pi session tree. Main tokens sum the parent session file; total tokens add every
/// child session file under `<parent without .jsonl>/` (subagent runs, nested children, forks).
/// The token fields summed are `input`, `output`, `cacheRead`, and `cacheWrite` of every assistant
/// message's usage and of every `usage`, `compaction`, and `branch_summary` record (Pi counts those
/// toward session totals; `totalTokens` is not read, `reasoning` is inside `output`). A record is
/// counted once across the tree by its `(id, timestamp)`: a fork starts as a copy of its parent's
/// records. `cost_usd` sums `usage.cost.total`, Pi's own figure from its price table.
pub fn parse(transcript: &Path, duration_s: f64) -> Result<Run> {
    let main = records(transcript)?;
    let cwd = main
        .first()
        .and_then(|r| r["cwd"].as_str())
        .unwrap_or_default()
        .to_string();
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut first_time = |r: &Value| {
        seen.insert((
            r["id"].as_str().unwrap_or_default().to_string(),
            r["timestamp"].as_str().unwrap_or_default().to_string(),
        ))
    };
    let main: Vec<&Value> = main.iter().filter(|r| first_time(r)).collect();
    let children: Vec<Vec<Value>> = child_transcripts(transcript)
        .iter()
        .map(|p| records(p))
        .collect::<Result<_>>()?;
    let children: Vec<Vec<&Value>> = children
        .iter()
        .map(|c| c.iter().filter(|r| first_time(r)).collect())
        .collect();

    let mut main_tokens = Tokens::default();
    let mut cost: Option<f64> = None;
    let mut add_usage = |recs: &[&Value], into: &mut Tokens| {
        for u in recs.iter().filter_map(|r| usage(r)) {
            into.add(tokens(u));
            if let Some(c) = u["cost"]["total"].as_f64() {
                cost = Some(cost.unwrap_or(0.0) + c);
            }
        }
    };
    add_usage(&main, &mut main_tokens);
    let mut total_tokens = main_tokens;
    for c in &children {
        add_usage(c, &mut total_tokens);
    }

    let mut tool_calls = vec![];
    let mut files_opened: Vec<String> = vec![];
    let mut shell_commands = vec![];
    for b in main
        .iter()
        .filter(|r| r["message"]["role"] == "assistant")
        .flat_map(|r| r["message"]["content"].as_array().into_iter().flatten())
        .filter(|b| b["type"] == "toolCall")
    {
        let name = b["name"].as_str().unwrap_or_default();
        tool_calls.push(name.to_string());
        match name {
            "bash" => shell_commands.push(
                b["arguments"]["command"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
            ),
            "read" => {
                if let Some(p) = b["arguments"]["path"]
                    .as_str()
                    .and_then(|p| repo_relative(p, &cwd))
                    && !files_opened.contains(&p)
                {
                    files_opened.push(p);
                }
            }
            _ => {}
        }
    }
    let mut all = actions(&main);
    for c in &children {
        all.extend(actions(c));
    }
    let model = main.iter().find(|r| r["type"] == "model_change").map(|r| {
        format!(
            "{}/{}",
            r["provider"].as_str().unwrap_or_default(),
            r["modelId"].as_str().unwrap_or_default()
        )
    });
    Ok(Run {
        main_tokens,
        total_tokens,
        tool_calls,
        files_opened,
        shell_commands,
        ritual_steps: ritual_steps(&all),
        cost_usd: cost,
        duration_s,
        model,
    })
}

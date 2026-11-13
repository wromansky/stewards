// The Hermes Agent driver: `hermes chat --oneshot --format stream-json`, whose agent runs in a
// gateway daemon, and the sessions `hermes sessions export` writes from its home.
use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::os::unix::fs::{MetadataExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use super::{
    Action, Driver, ReviewSubmit, Run, Session, Tokens, fresh_session_env, is_reviewer_submit,
    reads_git_history, records, repo_relative, reviewed_units, ritual_steps, run_session,
    shell_reads, stewards_commands,
};
use crate::output::CliError;

/// The eval's own config for every session: children run shell commands unprompted, as `--yolo`
/// lets the parent.
const CONFIG: &str = "delegation:\n  subagent_auto_approve: true\n";

pub struct Hermes {
    pub bin: PathBuf,
    /// A session still running after this is killed and recorded as timed out.
    pub limit: Duration,
    /// The eval's template home, from `isolate`: `config.yaml`, `.env` linked to the user's, and
    /// the `installs/` dependency environment Hermes built there. Each session's home links them.
    pub template: Option<PathBuf>,
    /// `fingerprint` of the template's `installs/` when `isolate` built it.
    pub installs: Vec<String>,
}

impl Hermes {
    /// The `hermes` on `path`, or a `harness_missing` error.
    pub fn find(path: &OsStr, limit: Duration) -> Result<Hermes> {
        std::env::split_paths(path)
            .map(|d| d.join("hermes"))
            .find(|p| p.is_file())
            .map(|bin| Hermes {
                bin,
                limit,
                template: None,
                installs: vec![],
            })
            .ok_or_else(|| {
                CliError::new(
                    "harness_missing",
                    "`hermes` is not on PATH",
                    "install Hermes Agent (https://hermes-agent.nousresearch.com/docs) and set a provider key in ~/.hermes/.env",
                )
                .into()
            })
    }

    /// Builds the eval's template home at `dir` from the user's (`HERMES_HOME`, else `~/.hermes`)
    /// and returns what the sessions run with in plain words. See `template`.
    pub fn isolate(&mut self, dir: &Path) -> Result<String> {
        let user = match std::env::var_os("HERMES_HOME") {
            Some(d) => PathBuf::from(d),
            None => {
                PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?).join(".hermes")
            }
        };
        let (note, installs) = template(&self.bin, &user, dir)?;
        self.template = Some(dir.to_path_buf());
        self.installs = installs;
        Ok(note)
    }
}

/// Writes a template home at `dir` holding only what a session needs from the user's home `user`,
/// and never writes there: `config.yaml` (`CONFIG`), `.env` linked to the user's (the provider
/// keys), and `installs/`, the dependency environment Hermes builds in every new home, built by
/// `hermes sessions list` (no model call). Returns that in plain words and the `fingerprint` of
/// `installs/`.
pub fn template(bin: &Path, user: &Path, dir: &Path) -> Result<(String, Vec<String>)> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("config.yaml"), CONFIG)?;
    let env_file = user.join(".env");
    let linked = if env_file.exists() {
        symlink(&env_file, dir.join(".env"))?;
        format!(".env linked to {}", env_file.display())
    } else {
        "no .env (the user's home has none)".to_string()
    };
    let mut cmd = Command::new(bin);
    cmd.args(["sessions", "list"]).stdin(Stdio::null());
    fresh_session_env(&mut cmd, &[]);
    cmd.env_remove("AI_AGENT").env("HERMES_HOME", dir);
    let out = cmd.output().context("running hermes sessions list")?;
    if !out.status.success() {
        bail!(
            "hermes sessions list in {}: {}",
            dir.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let installs = fingerprint(&dir.join("installs"))?;
    Ok((
        format!(
            "Hermes template home {}: config.yaml `delegation.subagent_auto_approve: true` (children run shell commands unprompted, as `--yolo` lets the parent); {linked}; installs/ (the Python dependency environment Hermes builds per home) built once. Each session gets a fresh home `<run>/h` (HERMES_HOME) holding that config, the linked .env, and installs/ linked; a session starts only while installs/ is unchanged since it was built. No user config, memory, SOUL.md, skills, plugins, or prior sessions; a fresh home holds Hermes's own default SOUL.md, bundled skills, and bundled plugins. The project AGENTS.md loads in its system prompt (no `--ignore-rules`). The Hermes agent runs in a gateway daemon that `hermes chat` starts for that home, whose working directory is Hermes's source tree, not the task directory; its shell and file tools act in the task directory (`--in`). The driver kills the daemon's process group after every session, and the runner's sweep kills anything else whose environment names the run directory. Thinking: `--thinking` passes as `--reasoning` (off as none) and children inherit it; each session's effective setting is recorded from its transcript, on or off. cost_usd: null, since Hermes prices DeepSeek at its off-peak rates in every hour (peak hours cost twice that); its figure is kept as harness_reported_cost_usd.",
            dir.display()
        ),
        installs,
    ))
}

/// Every entry under `dir` (recursively, links not followed) as `path size mtime mode`, sorted;
/// the `.leases` directories and `*.lock` files Hermes updates on every use are left out. Empty
/// when `dir` does not exist.
pub fn fingerprint(dir: &Path) -> Result<Vec<String>> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) -> Result<()> {
        for e in std::fs::read_dir(dir)? {
            let e = e?;
            let name = e.file_name();
            let name = name.to_string_lossy();
            if name == ".leases" || name.ends_with(".lock") {
                continue;
            }
            let p = e.path();
            let m = std::fs::symlink_metadata(&p)?;
            out.push(format!(
                "{} {} {}.{} {:o}",
                p.strip_prefix(root).unwrap_or(&p).display(),
                m.len(),
                m.mtime(),
                m.mtime_nsec(),
                m.mode()
            ));
            if m.is_dir() {
                walk(&p, root, out)?;
            }
        }
        Ok(())
    }
    let mut out = vec![];
    if dir.is_dir() {
        walk(dir, dir, &mut out)?;
    }
    out.sort();
    Ok(out)
}

impl Driver for Hermes {
    /// `hermes chat --query=<prompt> --oneshot --yolo --in <dir> --format stream-json --reasoning
    /// <thinking, off as none> [--provider p -m m]` (`model` is `provider/model`, split at its
    /// first `/`) in `dir`, with `HERMES_HOME` the session's own home `<dir's parent>/h` (see
    /// `isolate`) and `AI_AGENT` removed (Hermes keeps an inherited value). The agent runs in the
    /// gateway daemon that command starts; it is killed by its process group (from
    /// `<home>/gateway.pid`) when the command ends or times out. Then `hermes sessions export
    /// --format jsonl` writes the home's sessions to `transcript.jsonl` and the home's
    /// `logs/agent.log` is copied beside it; stdout (the stream) and stderr go to `stdout.log` and
    /// `stderr.log`.
    fn start(
        &self,
        dir: &Path,
        prompt: &str,
        model: Option<&str>,
        thinking: &str,
        sessions: &Path,
        env: &[(&str, OsString)],
    ) -> Result<Session> {
        if let Some(t) = &self.template
            && fingerprint(&t.join("installs"))? != self.installs
        {
            bail!(
                "{} changed after the eval built it (a session wrote into Hermes's dependency environment); the eval stopped so later sessions do not run on it",
                t.join("installs").display()
            );
        }
        std::fs::create_dir_all(sessions)?;
        let dir = dir.canonicalize()?;
        // a task's setup session ran in the same directory: each session starts from a new home
        let home = dir.parent().unwrap_or(&dir).join("h");
        if home.exists() {
            std::fs::remove_dir_all(&home)?;
        }
        std::fs::create_dir_all(&home)?;
        if let Some(t) = &self.template {
            std::fs::copy(t.join("config.yaml"), home.join("config.yaml"))?;
            for name in [".env", "installs"] {
                match std::fs::read_link(t.join(name)) {
                    Ok(target) => symlink(target, home.join(name))?,
                    Err(_) if t.join(name).exists() => symlink(t.join(name), home.join(name))?,
                    Err(_) => {}
                }
            }
        }
        let mut cmd = Command::new(&self.bin);
        cmd.current_dir(&dir)
            .arg("chat")
            .arg(format!("--query={prompt}"))
            .args(["--oneshot", "--yolo", "--in"])
            .arg(&dir)
            .args(["--format", "stream-json", "--reasoning"])
            .arg(if thinking == "off" { "none" } else { thinking });
        if let Some((provider, m)) = model.and_then(|m| m.split_once('/')) {
            cmd.args(["--provider", provider, "-m", m]);
        }
        cmd.stdin(Stdio::null())
            .stdout(File::create(sessions.join("stdout.log"))?)
            .stderr(File::create(sessions.join("stderr.log"))?);
        session_env(&mut cmd, env, &home);
        let (status, duration_s) = run_session(&mut cmd, self.limit)?;
        stop_daemon(&home);
        let transcript = sessions.join("transcript.jsonl");
        let mut export = Command::new(&self.bin);
        export
            .args(["sessions", "export", "--format", "jsonl", "--yes"])
            .arg(&transcript)
            .stdin(Stdio::null())
            .stdout(File::create(sessions.join("export.log"))?)
            .stderr(Stdio::null());
        session_env(&mut export, env, &home);
        run_session(&mut export, Duration::from_secs(300))?;
        let log = home.join("logs/agent.log");
        if log.is_file() {
            std::fs::copy(&log, sessions.join("agent.log"))?;
        }
        Ok(Session {
            status,
            transcript: transcript.is_file().then_some(transcript),
            duration_s,
        })
    }

    fn parse(&self, transcript: &Path, duration_s: f64, truncation_ok: bool) -> Result<Run> {
        parse(transcript, duration_s, truncation_ok)
    }
}

/// `fresh_session_env` with `env`, then `AI_AGENT` removed (Hermes only sets it when unset, and an
/// enclosing agent's value would stay) and `HERMES_HOME` set to `home`.
fn session_env(cmd: &mut Command, env: &[(&str, OsString)], home: &Path) {
    fresh_session_env(cmd, env);
    cmd.env_remove("AI_AGENT").env("HERMES_HOME", home);
}

/// SIGKILLs the process group of the gateway daemon named by `<home>/gateway.pid` (it starts
/// its own session, so the command's group kill misses it); Hermes's process supervisor then
/// reaps the groups the daemon registered.
fn stop_daemon(home: &Path) {
    let pid = std::fs::read_to_string(home.join("gateway.pid"))
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v["pid"].as_u64());
    if let Some(pid) = pid.filter(|&p| p > 1) {
        let _ = Command::new("kill")
            .args(["-KILL", &format!("-{pid}"), &pid.to_string()])
            .stderr(Stdio::null())
            .status();
    }
}

/// One exported session's counts.
fn tokens(s: &Value) -> Tokens {
    let n = |k: &str| s[k].as_u64().unwrap_or(0);
    Tokens {
        input: n("input_tokens"),
        output: n("output_tokens"),
        cache_read: n("cache_read_tokens"),
        cache_write: n("cache_write_tokens"),
    }
}

fn messages(s: &Value) -> impl DoubleEndedIterator<Item = &Value> {
    s["messages"].as_array().into_iter().flatten()
}

/// One tool call: its id, tool name, and arguments (a JSON string in the export, parsed).
struct Call<'a> {
    id: &'a Value,
    name: &'a str,
    args: Value,
}

/// The tool calls of one session, in order.
fn calls(s: &Value) -> Vec<Call<'_>> {
    messages(s)
        .filter(|m| m["role"] == "assistant")
        .flat_map(|m| m["tool_calls"].as_array().into_iter().flatten())
        .map(|c| {
            let f = &c["function"];
            Call {
                id: &c["id"],
                name: f["name"].as_str().unwrap_or_default(),
                args: match &f["arguments"] {
                    Value::String(a) => serde_json::from_str(a).unwrap_or_default(),
                    a => a.clone(),
                },
            }
        })
        .collect()
}

/// The `terminal` commands of one session, in order.
fn commands(s: &Value) -> Vec<String> {
    calls(s)
        .into_iter()
        .filter(|c| c.name == "terminal")
        .map(|c| c.args["command"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// The tool message answering `call`, and its content parsed as JSON.
fn result_of<'a>(s: &'a Value, call: &Call) -> Option<(&'a Value, Value)> {
    let m = messages(s).find(|m| m["role"] == "tool" && m["tool_call_id"] == *call.id)?;
    Some((m, serde_json::from_str(m["content"].as_str()?).ok()?))
}

/// What each child of a `delegate_task` call is given: its `goal` (its first message), with its
/// `context` (added to its system prompt) after a blank line when there is one. Each entry of
/// `tasks`, or the legacy top-level `goal`/`context`.
fn tasks(args: &Value) -> Vec<String> {
    let task = |t: &Value| {
        let goal = t["goal"].as_str()?;
        Some(
            match t["context"].as_str().filter(|c| !c.trim().is_empty()) {
                Some(c) => format!("{goal}\n\n{c}"),
                None => goal.to_string(),
            },
        )
    };
    let mut out: Vec<String> = args["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(task)
        .collect();
    out.extend(task(args));
    out
}

/// The ritual actions of one session (a child's when `child`): `terminal` commands and
/// `delegate_task` launches (an `action` other than `spawn` launches nothing).
fn actions(s: &Value, child: bool) -> Vec<Action> {
    calls(s)
        .into_iter()
        .filter_map(|c| match c.name {
            "terminal" => Some(Action::Shell {
                command: c.args["command"].as_str().unwrap_or_default().to_string(),
                child,
            }),
            "delegate_task" if c.args["action"].as_str().is_none_or(|a| a == "spawn") => {
                Some(Action::Dispatch {
                    tasks: tasks(&c.args),
                })
            }
            _ => None,
        })
        .collect()
}

/// Each reviewer `submit` in one session whose `terminal` output `reviewed_units` reads, with the
/// time of its result.
fn review_submits(s: &Value) -> Vec<(f64, ReviewSubmit)> {
    let commands = commands(s);
    calls(s)
        .iter()
        .filter(|c| c.name == "terminal")
        .filter(|c| {
            stewards_commands(c.args["command"].as_str().unwrap_or_default())
                .iter()
                .any(|cmd| is_reviewer_submit(cmd))
        })
        .filter_map(|c| {
            let (m, content) = result_of(s, c)?;
            let units = reviewed_units(content["output"].as_str()?)?;
            Some((
                m["timestamp"].as_f64().unwrap_or_default(),
                ReviewSubmit {
                    units,
                    commands: commands.clone(),
                },
            ))
        })
        .collect()
}

/// The distinct paths one session opened (see `Run::files_opened`), relative to `root` (the main
/// session's `cwd`), in order: each `read_file` `path`, each content `search_files` `path` (a
/// directory is dropped later by `existing_files`; a `target: files` listing opens nothing), and
/// each path `shell_reads` finds in a `terminal` command. A relative path is taken from the
/// session's `cwd`. `execute_code` scripts are not read.
/// simplified: a `cd` in one `terminal` call is not followed into the next; upgrade if
/// `files_opened` disagrees with the transcripts.
fn files_opened(s: &Value, root: &str) -> Vec<String> {
    let cwd = Path::new(s["cwd"].as_str().unwrap_or(root));
    let mut out: Vec<String> = vec![];
    for c in calls(s) {
        let paths: Vec<String> = match c.name {
            "read_file" => c.args["path"]
                .as_str()
                .map(String::from)
                .into_iter()
                .collect(),
            "search_files" if c.args["target"] != "files" => c.args["path"]
                .as_str()
                .map(String::from)
                .into_iter()
                .collect(),
            "terminal" => shell_reads(c.args["command"].as_str().unwrap_or_default()),
            _ => vec![],
        };
        for p in paths {
            let Some(p) = repo_relative(&cwd.join(p).to_string_lossy(), root) else {
                continue;
            };
            if !out.contains(&p) {
                out.push(p);
            }
        }
    }
    out
}

/// A session's effective thinking: `on` when it counted reasoning tokens or any response holds
/// non-blank `reasoning` or `reasoning_content` (Hermes stores a lone space with reasoning off),
/// else `off`; `None` with no response.
fn thinking(s: &Value) -> Option<&'static str> {
    let mut responses = messages(s).filter(|m| m["role"] == "assistant").peekable();
    responses.peek()?;
    let thought = s["reasoning_tokens"].as_u64().unwrap_or(0) > 0
        || responses.any(|m| {
            ["reasoning", "reasoning_content"]
                .iter()
                .any(|k| m[k].as_str().is_some_and(|t| !t.trim().is_empty()))
        });
    Some(if thought { "on" } else { "off" })
}

/// `provider/model` of one session, as the export names them.
fn model(s: &Value) -> Option<String> {
    let m = s["model"].as_str().filter(|m| !m.is_empty())?;
    Some(
        match s["billing_provider"].as_str().filter(|p| !p.is_empty()) {
            Some(p) => format!("{p}/{m}"),
            None => m.to_string(),
        },
    )
}

/// The children of one session's `delegate_task` calls that Hermes reported as not completed.
fn failed_children(s: &Value) -> Vec<String> {
    let mut out = vec![];
    for c in calls(s).iter().filter(|c| c.name == "delegate_task") {
        let Some((_, r)) = result_of(s, c) else {
            continue;
        };
        for e in r["results"].as_array().into_iter().flatten() {
            let status = e["status"].as_str().unwrap_or("unknown");
            if status != "completed" {
                out.push(format!(
                    "task {} of delegate_task {} ended `{status}`",
                    e["task_index"],
                    c.id.as_str().unwrap_or_default()
                ));
            }
        }
    }
    out
}

/// The largest prompt (`in=`, cache included) of any API call `agent.log` beside the transcript
/// logs for session `id`; `None` without such a line.
fn peak_context(transcript: &Path, id: &str) -> Option<u64> {
    let log = std::fs::read_to_string(transcript.with_file_name("agent.log")).ok()?;
    let tag = format!("[{id}]");
    log.lines()
        .filter(|l| l.contains(&tag) && l.contains(" API call #"))
        .filter_map(|l| {
            l.split_whitespace()
                .find_map(|w| w.strip_prefix("in="))?
                .parse()
                .ok()
        })
        .max()
}

/// The stream's final `result` line in `stdout.log` beside the transcript, if it got that far.
fn stream_result(transcript: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(transcript.with_file_name("stdout.log")).ok()?;
    text.lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .rfind(|v| v["type"] == "result")
}

/// Parses a `hermes sessions export --format jsonl` file: one line per session of the home,
/// exactly one with no `parent_session_id` (the main session), every other a child at any depth
/// (`source: subagent`). Each session's counts are its `input_tokens` (uncached), `output_tokens`
/// (reasoning inside), `cache_read_tokens`, and `cache_write_tokens`, which Hermes keeps per
/// session: main tokens are the main session's, total tokens add every child's. Hermes persists
/// no per-response usage, so `peak_main_context` is the largest prompt of the main session's API
/// calls in the `agent.log` copied beside the transcript (`None` without one). `cost_usd` is
/// `None` and the sessions' summed `estimated_cost_usd` is `harness_reported_cost_usd` (see
/// `Hermes::isolate`). `models` are each session's `billing_provider/model`; `thinking_levels`
/// each session's effective setting (see `thinking`), a session with no response in
/// `sessions_without_thinking`. The main stop reason is `error` when the stream's `result` line
/// has a nonzero `exit_code` or an `error`, else its last response's `finish_reason`; a child
/// failed when the `delegate_task` call that ran it reports a status other than `completed`.
/// `reviews` are the children's reviewer `submit` outputs, ordered by their results' times.
pub fn parse(transcript: &Path, duration_s: f64, truncation_ok: bool) -> Result<Run> {
    let (all, truncated_lines) = records(transcript, truncation_ok)?;
    let tops = all
        .iter()
        .filter(|s| s["parent_session_id"].is_null())
        .count();
    if tops != 1 {
        bail!(
            "{tops} top-level sessions in {}, expected one",
            transcript.display()
        );
    }
    let mut sessions: Vec<&Value> = all.iter().collect();
    sessions.sort_by_key(|s| !s["parent_session_id"].is_null());
    let main = sessions[0];
    let root = main["cwd"].as_str().unwrap_or_default();

    let mut total = Tokens::default();
    let mut models = BTreeSet::new();
    let mut thinking_levels = BTreeSet::new();
    let mut sessions_without_thinking = vec![];
    let mut all_actions = vec![];
    let mut tree_files: Vec<String> = vec![];
    let mut reviews = vec![];
    let mut git_history_commands = vec![];
    let mut failed = vec![];
    let mut cost: Option<f64> = None;
    for (i, s) in sessions.iter().enumerate() {
        total.add(tokens(s));
        models.extend(model(s));
        match thinking(s) {
            Some(t) => {
                thinking_levels.insert(t.to_string());
            }
            None => sessions_without_thinking.push(if i == 0 {
                "main".to_string()
            } else {
                s["id"].as_str().unwrap_or_default().to_string()
            }),
        }
        all_actions.extend(actions(s, i > 0));
        if i > 0 {
            reviews.extend(review_submits(s));
        }
        git_history_commands.extend(commands(s).into_iter().filter(|c| reads_git_history(c)));
        for f in files_opened(s, root) {
            if !tree_files.contains(&f) {
                tree_files.push(f);
            }
        }
        failed.extend(failed_children(s));
        if let Some(c) = s["estimated_cost_usd"].as_f64() {
            cost = Some(cost.unwrap_or(0.0) + c);
        }
    }
    let result = stream_result(transcript);
    let stop_reason = if result
        .as_ref()
        .is_some_and(|r| r["exit_code"].as_i64().is_some_and(|c| c != 0) || !r["error"].is_null())
    {
        Some("error".to_string())
    } else {
        messages(main)
            .rfind(|m| m["role"] == "assistant")
            .and_then(|m| m["finish_reason"].as_str())
            .map(String::from)
    };
    reviews.sort_by(|a, b| a.0.total_cmp(&b.0));
    Ok(Run {
        main_tokens: Some(tokens(main)),
        total_tokens: Some(total),
        cache_warming: Tokens::default(),
        peak_main_context: peak_context(transcript, main["id"].as_str().unwrap_or_default()),
        tool_calls: calls(main).iter().map(|c| c.name.to_string()).collect(),
        files_opened: files_opened(main, root),
        files_opened_tree: tree_files,
        shell_commands: commands(main),
        ritual_steps: ritual_steps(&all_actions),
        cost_usd: None,
        harness_reported_cost_usd: cost,
        duration_s,
        model: model(main),
        models: models.into_iter().collect(),
        thinking_levels: thinking_levels.into_iter().collect(),
        sessions_without_thinking,
        subagent_sessions: sessions.len() - 1,
        reviews: reviews.into_iter().map(|(_, r)| r).collect(),
        git_history_commands,
        stop_reason,
        failed_children: failed,
        truncated_lines,
    })
}

// The Pi driver: `pi --mode json` with pi-subagents, and its session transcripts.
use std::collections::{BTreeSet, HashSet};
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use super::{
    Action, Driver, FAILED_STOPS, ReviewSubmit, Run, Session, Tokens, fresh_session_env,
    is_reviewer_submit, reads_git_history, records, repo_relative, reviewed_units, ritual_steps,
    run_session, shell_reads, stewards_commands,
};
use crate::output::CliError;

pub struct Pi {
    pub bin: PathBuf,
    /// A session still running after this is killed and recorded as timed out.
    pub limit: Duration,
    /// The eval's own Pi agent directory (`PI_CODING_AGENT_DIR`), from `isolate`.
    pub agent_dir: Option<PathBuf>,
}

impl Pi {
    /// The `pi` on `path`, or a `harness_missing` error.
    pub fn find(path: &OsStr, limit: Duration) -> Result<Pi> {
        std::env::split_paths(path)
            .map(|d| d.join("pi"))
            .find(|p| p.is_file())
            .map(|bin| Pi {
                bin,
                limit,
                agent_dir: None,
            })
            .ok_or_else(|| missing("`pi` is not on PATH".into()))
    }

    /// Builds the eval's Pi agent directory at `dir` from the user's (`PI_CODING_AGENT_DIR`, else
    /// `~/.pi/agent`), so sessions load none of the user's personal setup, and returns what it
    /// holds in plain words. See `agent_dir`.
    pub fn isolate(&mut self, dir: &Path, thinking: &str) -> Result<String> {
        let home = PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?);
        let user = std::env::var_os("PI_CODING_AGENT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".pi/agent"));
        let note = agent_dir(&user, &home, dir, thinking)?;
        self.agent_dir = Some(dir.to_path_buf());
        Ok(note)
    }
}

fn missing(message: String) -> anyhow::Error {
    CliError::new(
        "harness_missing",
        message,
        "install Pi (`npm install -g @earendil-works/pi-coding-agent`) and pi-subagents (`pi install npm:pi-subagents`)",
    )
    .into()
}

/// Writes a Pi agent directory at `dir` holding only what a session needs from the user's `user`
/// directory, and never writes there: `settings.json` listing only the `npm:pi-subagents` package
/// (with the user's `defaultProvider`/`defaultModel`, if any, and the skills Pi always scans in
/// `<home>/.agents/skills` excluded, and every child at `thinking`: pi-subagents'
/// `subagents.defaultThinking` and `maxThinking` set to it and an `agentOverrides.<name>.thinking`
/// for each bundled agent, since `defaultThinking` does not override an agent's own frontmatter and
/// `maxThinking` fails a launch above it instead of lowering it); `auth.json` linked, so an OAuth refresh is shared with the
/// user's own Pi instead of rotating their token away (Pi locks the linked file); `models-store.json`
/// copied; `npm/` linked (the installed package store). No `AGENTS.md`, `extensions/`, hooks,
/// agents, or other packages. Returns that in plain words.
pub fn agent_dir(user: &Path, home: &Path, dir: &Path, thinking: &str) -> Result<String> {
    let package = user.join("npm/node_modules/pi-subagents");
    if !package.is_dir() {
        return Err(missing(format!(
            "pi-subagents is not installed in {}",
            user.display()
        )));
    }
    std::fs::create_dir_all(dir)?;
    let theirs: Value = std::fs::read_to_string(user.join("settings.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let mut overrides = serde_json::Map::new();
    for e in std::fs::read_dir(package.join("agents"))
        .into_iter()
        .flatten()
        .flatten()
    {
        let p = e.path();
        if p.extension() == Some(OsStr::new("md"))
            && let Some(name) = p.file_stem().and_then(OsStr::to_str)
        {
            overrides.insert(name.to_string(), json!({ "thinking": thinking }));
        }
    }
    let mut settings = json!({
        "packages": ["npm:pi-subagents"],
        "skills": [format!("!{}/.agents/skills/**", home.display())],
        "subagents": {
            "defaultThinking": thinking,
            "maxThinking": thinking,
            "agentOverrides": overrides,
        },
    });
    for key in ["defaultProvider", "defaultModel"] {
        if let Some(v) = theirs.get(key) {
            settings[key] = v.clone();
        }
    }
    std::fs::write(
        dir.join("settings.json"),
        serde_json::to_string_pretty(&settings)?,
    )?;
    let mut held = vec![format!("settings.json {settings}")];
    for (name, link) in [
        ("auth.json", true),
        ("models-store.json", false),
        ("npm", true),
    ] {
        let from = user.join(name);
        if !from.exists() {
            continue;
        }
        if link {
            std::os::unix::fs::symlink(&from, dir.join(name))?;
            held.push(format!("{name} linked to {}", from.display()));
        } else {
            std::fs::copy(&from, dir.join(name))?;
            held.push(format!("{name} copied"));
        }
    }
    Ok(format!(
        "Pi agent dir {} (PI_CODING_AGENT_DIR): {}. Active: the pi-subagents extension and its skills, and Pi's built-in extensions; no user AGENTS.md, extensions, hooks, agents, other packages, or ~/.agents skills.",
        dir.display(),
        held.join("; ")
    ))
}

impl Driver for Pi {
    /// `pi --mode json --session-dir <sessions> --exclude-tools subagents_enable --thinking <level>
    /// [--model m] -- <prompt>` in `dir`, with `PI_CODING_AGENT_DIR` set to the eval's agent
    /// directory: excluding the loader keeps `subagent` active from the first request. Pi stays up
    /// until its background subagents finish. stdout and stderr go to `stdout.log` and `stderr.log`.
    fn start(
        &self,
        dir: &Path,
        prompt: &str,
        model: Option<&str>,
        thinking: &str,
        sessions: &Path,
        env: &[(&str, OsString)],
    ) -> Result<Session> {
        std::fs::create_dir_all(sessions)?;
        let mut cmd = Command::new(&self.bin);
        cmd.current_dir(dir)
            .args(["--mode", "json", "--session-dir"])
            .arg(sessions)
            .args([
                "--exclude-tools",
                "subagents_enable",
                "--thinking",
                thinking,
            ]);
        if let Some(m) = model {
            cmd.args(["--model", m]);
        }
        cmd.arg("--")
            .arg(prompt)
            .stdin(Stdio::null())
            .stdout(File::create(sessions.join("stdout.log"))?)
            .stderr(File::create(sessions.join("stderr.log"))?);
        fresh_session_env(&mut cmd, env);
        if let Some(a) = &self.agent_dir {
            cmd.env("PI_CODING_AGENT_DIR", a);
        }
        let (status, duration_s) = run_session(&mut cmd, self.limit)?;
        Ok(Session {
            status,
            transcript: top_level(sessions)?.into_iter().next(),
            duration_s,
        })
    }

    fn parse(&self, transcript: &Path, duration_s: f64, truncation_ok: bool) -> Result<Run> {
        parse(transcript, duration_s, truncation_ok)
    }
}

/// The `.jsonl` files directly in a session directory, sorted.
fn top_level(sessions: &Path) -> Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(sessions)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension() == Some(OsStr::new("jsonl")))
        .collect();
    files.sort();
    Ok(files)
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

/// The usage a record carries, and whether it is context usage: an assistant message's
/// `message.usage` and the top-level `usage` of `compaction` and `branch_summary` records are;
/// the top-level `usage` of a `usage` record (a cache refresh) is not.
fn usage(r: &Value) -> Option<(&Value, bool)> {
    let (u, context) = match r["type"].as_str()? {
        "message" if r["message"]["role"] == "assistant" => (&r["message"]["usage"], true),
        "compaction" | "branch_summary" => (&r["usage"], true),
        "usage" => (&r["usage"], false),
        _ => return None,
    };
    u.is_object().then_some((u, context))
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

/// The tool calls of one session's records, in order.
fn tool_calls<'a>(records: &[&'a Value]) -> impl Iterator<Item = &'a Value> {
    records
        .iter()
        .filter(|r| r["message"]["role"] == "assistant")
        .flat_map(|r| r["message"]["content"].as_array().into_iter().flatten())
        .filter(|b| b["type"] == "toolCall")
}

/// The shell commands of one session's records, in order.
fn bash_commands<'a>(records: &[&'a Value]) -> impl Iterator<Item = (&'a Value, &'a str)> {
    tool_calls(records)
        .filter(|b| b["name"] == "bash")
        .map(|b| (b, b["arguments"]["command"].as_str().unwrap_or_default()))
}

/// The ritual actions of one session (a child's when `child`): shell commands and subagent
/// launches.
fn actions(records: &[&Value], child: bool) -> Vec<Action> {
    tool_calls(records)
        .filter_map(|b| match b["name"].as_str() {
            Some("subagent") => Some(Action::Dispatch {
                tasks: tasks(&b["arguments"]),
            }),
            Some("bash") => Some(Action::Shell {
                command: b["arguments"]["command"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                child,
            }),
            _ => None,
        })
        .collect()
}

/// Every string under a `task` key in `v`, at any depth: what each child of a pi-subagents
/// `subagent` call (single, `tasks`, or `chain`) is given as its task.
fn tasks(v: &Value) -> Vec<String> {
    match v {
        Value::Array(a) => a.iter().flat_map(tasks).collect(),
        Value::Object(o) => o
            .iter()
            .flat_map(|(k, v)| match (k.as_str(), v) {
                ("task", Value::String(s)) => vec![s.clone()],
                _ => tasks(v),
            })
            .collect(),
        _ => vec![],
    }
}

/// Each reviewer `submit` in one session whose output `reviewed_units` reads, with the
/// timestamp of its result.
fn review_submits(records: &[&Value]) -> Vec<(String, ReviewSubmit)> {
    let commands: Vec<String> = bash_commands(records).map(|(_, c)| c.to_string()).collect();
    let mut out = vec![];
    for (call, command) in bash_commands(records) {
        if !stewards_commands(command)
            .iter()
            .any(|c| is_reviewer_submit(c))
        {
            continue;
        }
        let Some(result) = records.iter().find(|r| {
            r["message"]["role"] == "toolResult" && r["message"]["toolCallId"] == call["id"]
        }) else {
            continue;
        };
        let text: String = result["message"]["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|b| b["text"].as_str())
            .collect();
        if let Some(units) = reviewed_units(&text) {
            let at = result["timestamp"].as_str().unwrap_or_default().to_string();
            out.push((
                at,
                ReviewSubmit {
                    units,
                    commands: commands.clone(),
                },
            ));
        }
    }
    out
}

/// The distinct repo-relative paths one session opened (see `Run::files_opened`), in order: each
/// `read` tool path and each path `shell_reads` finds in a `bash` command.
fn files_opened(records: &[&Value]) -> Vec<String> {
    let cwd = records
        .iter()
        .find_map(|r| r["cwd"].as_str())
        .unwrap_or_default();
    let mut out: Vec<String> = vec![];
    for b in tool_calls(records) {
        let paths = match b["name"].as_str() {
            Some("read") => b["arguments"]["path"]
                .as_str()
                .map(String::from)
                .into_iter()
                .collect(),
            Some("bash") => shell_reads(b["arguments"]["command"].as_str().unwrap_or_default()),
            _ => vec![],
        };
        for p in paths.iter().filter_map(|p| repo_relative(p, cwd)) {
            if !out.contains(&p) {
                out.push(p);
            }
        }
    }
    out
}

/// The stop reason of a session's last assistant message.
fn last_stop(records: &[&Value]) -> Option<String> {
    records
        .iter()
        .rev()
        .find(|r| r["message"]["role"] == "assistant")
        .and_then(|r| r["message"]["stopReason"].as_str())
        .map(str::to_string)
}

/// Parses a Pi session tree. The session directory must hold exactly one top-level `.jsonl`.
/// Main tokens sum the parent session file; total tokens add every child session file under
/// `<parent without .jsonl>/` (subagent runs, nested children, forks). The token fields summed
/// are `input`, `output`, `cacheRead`, and `cacheWrite` of every assistant message's usage and of
/// every `compaction` and `branch_summary` record (`totalTokens` is not read, `reasoning` is
/// inside `output`); `usage` records (cache refreshes, which do not enter context) go to
/// `cache_warming` instead. A record is counted once across the tree by its `(id, timestamp)`: a
/// fork starts as a copy of its parent's records. `cost_usd` sums `usage.cost.total` of all of
/// them, Pi's own figure from its price table. `files_opened` counts, per session, the paths of
/// the `read` tool plus the shell reads `shell_reads` recognises, each relative to that session's
/// own `cwd`. Models come from `model_change` records and assistant messages over the tree,
/// thinking levels from `thinking_level_change` records and each assistant message's
/// `thinkingLevel` over the tree; a session with neither is listed in
/// `sessions_without_thinking`. `reviews` are the child sessions' reviewer `submit` outputs (a
/// main session's own is not a review),
/// ordered by their results' timestamps.
pub fn parse(transcript: &Path, duration_s: f64, truncation_ok: bool) -> Result<Run> {
    let dir = transcript.parent().unwrap_or(Path::new("."));
    let tops = top_level(dir)?;
    if tops.len() > 1 {
        bail!(
            "{} top-level session files in {}, expected one",
            tops.len(),
            dir.display()
        );
    }
    let (main, mut truncated_lines) = records(transcript, truncation_ok)?;
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut first_time = |r: &Value| {
        seen.insert((
            r["id"].as_str().unwrap_or_default().to_string(),
            r["timestamp"].as_str().unwrap_or_default().to_string(),
        ))
    };
    let main: Vec<&Value> = main.iter().filter(|r| first_time(r)).collect();
    let tree = transcript.with_extension("");
    let mut children: Vec<(String, Vec<Value>)> = vec![];
    for p in child_transcripts(transcript) {
        let (recs, t) = records(&p, truncation_ok)?;
        truncated_lines += t;
        let name = p.strip_prefix(&tree).unwrap_or(&p).display().to_string();
        children.push((name, recs));
    }
    let children: Vec<(&str, Vec<&Value>)> = children
        .iter()
        .map(|(n, c)| (n.as_str(), c.iter().filter(|r| first_time(r)).collect()))
        .collect();

    let mut main_tokens = Tokens::default();
    let mut total_tokens = Tokens::default();
    let mut cache_warming = Tokens::default();
    let mut peak_main_context = 0;
    let mut cost: Option<f64> = None;
    let sessions = std::iter::once(("", &main)).chain(children.iter().map(|(n, c)| (*n, c)));
    for (i, (_, recs)) in sessions.clone().enumerate() {
        for (u, context) in recs.iter().filter_map(|r| usage(r)) {
            let t = tokens(u);
            if !context {
                cache_warming.add(t);
            } else {
                total_tokens.add(t);
                if i == 0 {
                    main_tokens.add(t);
                    peak_main_context = peak_main_context.max(t.context());
                }
            }
            if let Some(c) = u["cost"]["total"].as_f64() {
                cost = Some(cost.unwrap_or(0.0) + c);
            }
        }
    }

    let mut tool_names = vec![];
    let mut shell_commands = vec![];
    for b in tool_calls(&main) {
        let name = b["name"].as_str().unwrap_or_default();
        tool_names.push(name.to_string());
        if name == "bash" {
            shell_commands.push(
                b["arguments"]["command"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
            );
        }
    }
    let mut all = vec![];
    let mut tree_files: Vec<String> = vec![];
    let mut models = BTreeSet::new();
    let mut thinking_levels = BTreeSet::new();
    let mut sessions_without_thinking = vec![];
    let mut reviews = vec![];
    let mut git_history_commands = vec![];
    let mut failed_children = vec![];
    for (i, (name, recs)) in sessions.enumerate() {
        all.extend(actions(recs, i > 0));
        if i > 0 {
            reviews.extend(review_submits(recs));
        }
        git_history_commands.extend(
            bash_commands(recs)
                .map(|(_, c)| c)
                .filter(|c| reads_git_history(c))
                .map(String::from),
        );
        for f in files_opened(recs) {
            if !tree_files.contains(&f) {
                tree_files.push(f);
            }
        }
        let mut levels = 0;
        for r in recs.iter() {
            let m = &r["message"];
            let level = match r["type"].as_str() {
                Some("thinking_level_change") => r["thinkingLevel"].as_str(),
                _ if m["role"] == "assistant" => m["thinkingLevel"].as_str(),
                _ => None,
            };
            if let Some(l) = level {
                levels += 1;
                thinking_levels.insert(l.to_string());
            }
            if r["type"] == "model_change" {
                models.insert(format!(
                    "{}/{}",
                    r["provider"].as_str().unwrap_or_default(),
                    r["modelId"].as_str().unwrap_or_default()
                ));
            } else if m["role"] == "assistant"
                && let (Some(p), Some(id)) = (m["provider"].as_str(), m["model"].as_str())
            {
                models.insert(format!("{p}/{id}"));
            }
        }
        if levels == 0 {
            sessions_without_thinking.push(if i == 0 { "main" } else { name }.to_string());
        }
        if i > 0
            && let Some(s) = last_stop(recs).filter(|s| FAILED_STOPS.contains(&s.as_str()))
        {
            failed_children.push(format!("{name} stopped with `{s}`"));
        }
    }
    let model = main.iter().find(|r| r["type"] == "model_change").map(|r| {
        format!(
            "{}/{}",
            r["provider"].as_str().unwrap_or_default(),
            r["modelId"].as_str().unwrap_or_default()
        )
    });
    Ok(Run {
        main_tokens: Some(main_tokens),
        total_tokens: Some(total_tokens),
        cache_warming,
        peak_main_context: Some(peak_main_context),
        tool_calls: tool_names,
        files_opened: files_opened(&main),
        files_opened_tree: tree_files,
        shell_commands,
        ritual_steps: ritual_steps(&all),
        cost_usd: cost,
        harness_reported_cost_usd: None,
        duration_s,
        model,
        models: models.into_iter().collect(),
        thinking_levels: thinking_levels.into_iter().collect(),
        sessions_without_thinking,
        subagent_sessions: children.len(),
        uncounted_sessions: vec![],
        reviews: {
            reviews.sort_by(|a, b| a.0.cmp(&b.0));
            reviews.into_iter().map(|(_, r)| r).collect()
        },
        git_history_commands,
        stop_reason: last_stop(&main),
        failed_children,
        truncated_lines,
    })
}

// The Claude Code driver: `claude -p --output-format stream-json` and its persisted session file.
use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::{Value, json};

use super::{
    Action, Driver, FAILED_STOPS, ReviewSubmit, Run, Session, Tokens, fresh_session_env,
    is_reviewer_submit, reads_git_history, records, repo_relative, reviewed_units, ritual_steps,
    run_session, shell_reads, stewards_commands,
};
use crate::output::CliError;

pub struct ClaudeCode {
    pub bin: PathBuf,
    /// A session still running after this is killed and recorded as timed out.
    pub limit: Duration,
    /// The eval's own Claude Code config directory (`CLAUDE_CONFIG_DIR`), from `isolate`.
    pub config_dir: Option<PathBuf>,
    /// The user's own `model` setting: the children's model when no `--model` is given.
    pub default_model: Option<String>,
}

impl ClaudeCode {
    /// The `claude` on `path`, or a `harness_missing` error.
    pub fn find(path: &OsStr, limit: Duration) -> Result<ClaudeCode> {
        std::env::split_paths(path)
            .map(|d| d.join("claude"))
            .find(|p| p.is_file())
            .map(|bin| ClaudeCode {
                bin,
                limit,
                config_dir: None,
                default_model: None,
            })
            .ok_or_else(|| {
                CliError::new(
                    "harness_missing",
                    "`claude` is not on PATH",
                    "install Claude Code (`npm install -g @anthropic-ai/claude-code`) and log in with `claude`",
                )
                .into()
            })
    }

    /// Builds the eval's config directory at `dir` from the user's (`CLAUDE_CONFIG_DIR`, else
    /// `~/.claude`), so sessions load none of the user's personal setup, and returns what it holds
    /// in plain words. See `config_dir`.
    pub fn isolate(&mut self, dir: &Path) -> Result<String> {
        let user = match std::env::var_os("CLAUDE_CONFIG_DIR") {
            Some(d) => PathBuf::from(d),
            None => {
                PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?).join(".claude")
            }
        };
        let (note, model) = config_dir(&user, dir)?;
        self.config_dir = Some(dir.to_path_buf());
        self.default_model = model;
        Ok(note)
    }
}

/// Writes a Claude Code config directory at `dir` holding only what a session needs from the
/// user's `user` directory, and never writes there: `settings.json` with only the user's `model`
/// (if any), and `.credentials.json` linked, so an OAuth refresh is shared with the user's own
/// Claude Code. No `CLAUDE.md`, `agents/`, `commands/`, hooks, plugins, or `.claude.json` (where
/// user MCP servers live). Returns that in plain words and the user's model.
pub fn config_dir(user: &Path, dir: &Path) -> Result<(String, Option<String>)> {
    std::fs::create_dir_all(dir)?;
    let theirs: Value = std::fs::read_to_string(user.join("settings.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let model = theirs["model"].as_str().map(String::from);
    let settings = match &model {
        Some(m) => json!({ "model": m }),
        None => json!({}),
    };
    std::fs::write(dir.join("settings.json"), settings.to_string())?;
    let mut held = vec![format!("settings.json {settings}")];
    let credentials = user.join(".credentials.json");
    if credentials.exists() {
        std::os::unix::fs::symlink(&credentials, dir.join(".credentials.json"))?;
        held.push(format!(
            ".credentials.json linked to {}",
            credentials.display()
        ));
    }
    Ok((
        format!(
            "Claude Code config dir {} (CLAUDE_CONFIG_DIR): {}; children on the session's model (CLAUDE_CODE_SUBAGENT_MODEL); no user CLAUDE.md, agents, commands, hooks, plugins, or MCP servers. Claude Code 1.0.128 does not read AGENTS.md: every arm copy with an AGENTS.md also gets a committed one-line CLAUDE.md, `@AGENTS.md`, which imports it. Thinking: off, since this version has no thinking level, only a budget from MAX_THINKING_TOKENS (removed from the session's environment) or from words such as `think` in a user message (no task prompt may contain it); each session's effective setting is recorded from its thinking blocks. cost_usd: null, since its total_cost_usd prices model ids it does not know at Sonnet rates (checked on claude-haiku-5-5); that figure is kept as harness_reported_cost_usd.",
            dir.display(),
            held.join("; ")
        ),
        model,
    ))
}

impl Driver for ClaudeCode {
    /// `claude -p --output-format stream-json --verbose --dangerously-skip-permissions [--model m]
    /// -- <prompt>` in `dir`, with `CLAUDE_CONFIG_DIR` set to the eval's config directory and
    /// `CLAUDE_CODE_SUBAGENT_MODEL` to the session's model (else `general-purpose` children run on
    /// the harness's built-in default). There is no thinking level, so `thinking` is unused, and
    /// `MAX_THINKING_TOKENS`, the one variable that sets a thinking budget, is removed. stdout
    /// (the stream) and stderr go to `stdout.log` and `stderr.log`; the transcript is the session
    /// file Claude Code persisted, copied in as `<session id>.jsonl`.
    fn start(
        &self,
        dir: &Path,
        prompt: &str,
        model: Option<&str>,
        _thinking: &str,
        sessions: &Path,
        env: &[(&str, OsString)],
    ) -> Result<Session> {
        std::fs::create_dir_all(sessions)?;
        let mut cmd = Command::new(&self.bin);
        cmd.current_dir(dir).args([
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
            "--dangerously-skip-permissions",
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
        // what an enclosing Claude Code sets for its shell; the `CLAUDE_CODE` prefix misses it
        cmd.env_remove("CLAUDECODE");
        cmd.env_remove("MAX_THINKING_TOKENS");
        if let Some(c) = &self.config_dir {
            cmd.env("CLAUDE_CONFIG_DIR", c);
        }
        if let Some(m) = model.or(self.default_model.as_deref()) {
            cmd.env("CLAUDE_CODE_SUBAGENT_MODEL", m);
        }
        let (status, duration_s) = run_session(&mut cmd, self.limit)?;
        Ok(Session {
            status,
            transcript: self.keep_transcript(sessions)?,
            duration_s,
        })
    }

    fn parse(&self, transcript: &Path, duration_s: f64, truncation_ok: bool) -> Result<Run> {
        parse(transcript, duration_s, truncation_ok)
    }

    fn context_file(&self) -> Option<(&'static str, &'static str)> {
        Some(("CLAUDE.md", "@AGENTS.md\n"))
    }
}

impl ClaudeCode {
    /// Copies the session file named by the stream's `system` line, from
    /// `<config dir>/projects/<cwd slug>/<session id>.jsonl`, to `sessions/`.
    fn keep_transcript(&self, sessions: &Path) -> Result<Option<PathBuf>> {
        let stream = std::fs::read_to_string(sessions.join("stdout.log")).unwrap_or_default();
        let id = stream
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .find(|v| v["type"] == "system")
            .and_then(|v| v["session_id"].as_str().map(String::from));
        let (Some(config), Some(id)) = (&self.config_dir, id) else {
            return Ok(None);
        };
        let name = format!("{id}.jsonl");
        let found = std::fs::read_dir(config.join("projects"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path().join(&name))
            .find(|p| p.is_file());
        let Some(found) = found else {
            return Ok(None);
        };
        let kept = sessions.join(name);
        std::fs::copy(&found, &kept)?;
        Ok(Some(kept))
    }
}

/// Each message's last record, in order: a response is persisted once per content block with the
/// same message id and its usage so far. Its input and cache counts are final from the first
/// record; its output count may stay short of the final one, which only the stream's `result`
/// line holds.
fn responses<'a>(recs: &[&'a Value]) -> Vec<&'a Value> {
    let mut out: Vec<&Value> = vec![];
    for r in recs.iter().filter(|r| r["type"] == "assistant") {
        let id = &r["message"]["id"];
        match out
            .iter_mut()
            .find(|o| id.is_string() && o["message"]["id"] == *id)
        {
            Some(o) => *o = r,
            None => out.push(r),
        }
    }
    out
}

/// A usage object's four counts under the names given (`input`, `output`, `cache read`,
/// `cache write`).
fn tokens(u: &Value, [input, output, cache_read, cache_write]: [&str; 4]) -> Tokens {
    let n = |k: &str| u[k].as_u64().unwrap_or(0);
    Tokens {
        input: n(input),
        output: n(output),
        cache_read: n(cache_read),
        cache_write: n(cache_write),
    }
}

/// The names of a response's (and the `result` line's `usage`) counts.
const USAGE: [&str; 4] = [
    "input_tokens",
    "output_tokens",
    "cache_read_input_tokens",
    "cache_creation_input_tokens",
];

/// The names of each model's counts in the `result` line's `modelUsage`.
const MODEL_USAGE: [&str; 4] = [
    "inputTokens",
    "outputTokens",
    "cacheReadInputTokens",
    "cacheCreationInputTokens",
];

/// The tool calls of one session's records, in order.
fn tool_uses<'a>(recs: &[&'a Value]) -> impl Iterator<Item = &'a Value> {
    recs.iter()
        .filter(|r| r["type"] == "assistant")
        .flat_map(|r| r["message"]["content"].as_array().into_iter().flatten())
        .filter(|b| b["type"] == "tool_use")
}

/// The `Bash` calls of one session's records with their commands, in order.
fn bash_commands<'a>(recs: &[&'a Value]) -> impl Iterator<Item = (&'a Value, &'a str)> {
    tool_uses(recs)
        .filter(|b| b["name"] == "Bash")
        .map(|b| (b, b["input"]["command"].as_str().unwrap_or_default()))
}

/// The ritual actions of one session (a child's when `child`): `Bash` commands and `Task`
/// launches, whose `prompt` is the child's task.
fn actions(recs: &[&Value], child: bool) -> Vec<Action> {
    tool_uses(recs)
        .filter_map(|b| {
            let input = &b["input"];
            match b["name"].as_str() {
                Some("Task") => Some(Action::Dispatch {
                    tasks: input["prompt"]
                        .as_str()
                        .map(String::from)
                        .into_iter()
                        .collect(),
                }),
                Some("Bash") => Some(Action::Shell {
                    command: input["command"].as_str().unwrap_or_default().to_string(),
                    child,
                }),
                _ => None,
            }
        })
        .collect()
}

/// The record holding a tool call's result, and its `tool_result` block.
fn tool_result<'a>(recs: &[&'a Value], call: &Value) -> Option<(&'a Value, &'a Value)> {
    recs.iter().find_map(|r| {
        let block = r["message"]["content"]
            .as_array()?
            .iter()
            .find(|b| b["type"] == "tool_result" && b["tool_use_id"] == call["id"])?;
        Some((*r, block))
    })
}

/// Each reviewer `submit` in one session whose output `reviewed_units` reads, with the
/// timestamp of its result.
fn review_submits(recs: &[&Value]) -> Vec<(String, ReviewSubmit)> {
    let commands: Vec<String> = bash_commands(recs).map(|(_, c)| c.to_string()).collect();
    let mut out = vec![];
    for (call, command) in bash_commands(recs) {
        if !stewards_commands(command)
            .iter()
            .any(|c| is_reviewer_submit(c))
        {
            continue;
        }
        let Some((r, block)) = tool_result(recs, call) else {
            continue;
        };
        let text = match &block["content"] {
            Value::String(s) => s.clone(),
            c => c
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|b| b["text"].as_str())
                .collect(),
        };
        if let Some(units) = reviewed_units(&text) {
            let at = r["timestamp"].as_str().unwrap_or_default().to_string();
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

/// The distinct paths one session opened, relative to `root` (the main session's starting
/// directory), in order: each `Read` tool `file_path`, each `Grep` tool `path` (a directory is
/// dropped later by `existing_files`), each file a `Glob` call matched as its persisted result
/// lists them (`toolUseResult.filenames`; a call without that result counts nothing), and each
/// path `shell_reads` finds in a `Bash` command. A relative path is resolved against the `cwd`
/// of the record that made the call, since a `cd` in one `Bash` call carries over to the next.
fn files_opened(recs: &[&Value], root: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for r in recs.iter().filter(|r| r["type"] == "assistant") {
        let cwd = Path::new(r["cwd"].as_str().unwrap_or(root));
        for b in r["message"]["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|b| b["type"] == "tool_use")
        {
            let input = &b["input"];
            let paths: Vec<String> = match b["name"].as_str() {
                Some("Read") => input["file_path"]
                    .as_str()
                    .map(String::from)
                    .into_iter()
                    .collect(),
                Some("Grep") => input["path"]
                    .as_str()
                    .map(String::from)
                    .into_iter()
                    .collect(),
                Some("Glob") => tool_result(recs, b)
                    .map(|(res, _)| &res["toolUseResult"]["filenames"])
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|f| f.as_str().map(String::from))
                    .collect(),
                Some("Bash") => shell_reads(input["command"].as_str().unwrap_or_default()),
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
    }
    out
}

/// A session's effective thinking, from its responses' content blocks: `on` when one holds a
/// non-empty `thinking` or a `redacted_thinking` block, else `off` (Haiku 5.5 emits an empty
/// `thinking` block with no budget); `None` with no response but API failures (`<synthetic>`).
fn thinking(recs: &[&Value]) -> Option<&'static str> {
    let responses: Vec<&Value> = recs
        .iter()
        .copied()
        .filter(|r| r["type"] == "assistant" && r["message"]["model"] != "<synthetic>")
        .collect();
    if responses.is_empty() {
        return None;
    }
    let thought = responses
        .iter()
        .flat_map(|r| r["message"]["content"].as_array().into_iter().flatten())
        .any(|b| {
            b["type"] == "redacted_thinking"
                || (b["type"] == "thinking"
                    && b["thinking"].as_str().is_some_and(|t| !t.is_empty()))
        });
    Some(if thought { "on" } else { "off" })
}

/// How a session's last response ended: `error` for an API failure (a record Claude Code marks
/// `isApiErrorMessage`), else its `stop_reason` (null in every record 1.0.128 persists).
fn last_stop(recs: &[&Value]) -> Option<String> {
    let r = recs.iter().rev().find(|r| r["type"] == "assistant")?;
    if r["isApiErrorMessage"] == true {
        return Some("error".into());
    }
    r["message"]["stop_reason"].as_str().map(String::from)
}

/// The stream's final `result` line in `stdout.log` beside the transcript, if it got that far.
fn stream_result(transcript: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(transcript.with_file_name("stdout.log")).ok()?;
    text.lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .rfind(|v| v["type"] == "result")
}

/// Parses a Claude Code session file. Records with `isSidechain: true` are the children's; each
/// child is the chain of records from a sidechain record with no sidechain parent, linked by
/// `parentUuid`. Main tokens are the stream's `result.usage` (the parent's alone) and total
/// tokens the sum of its `modelUsage` (every request of the process, children included); both
/// are `None` without a `result` line (a session that did not finish), since the records' output
/// counts fall short (see `responses`). `peak_main_context` comes from the main records, whose
/// input and cache counts are final. `cost_usd` is `None`: the stream's `total_cost_usd`
/// prices an unknown model id at another model's rates, so it is only
/// `harness_reported_cost_usd`. `models` holds every model of the records and every key of
/// `modelUsage`, which also names side requests (titles, summaries) outside the parent and its
/// children. `thinking_levels` are the sessions' effective settings (see `thinking`), and a
/// session without one is in `sessions_without_thinking`. The main stop reason is `error` when
/// the stream's `result` line says `is_error`, else `last_stop`. Children are named by their first record's
/// `uuid`. `reviews` are the children's reviewer `submit` outputs, ordered by their results'
/// timestamps.
pub fn parse(transcript: &Path, duration_s: f64, truncation_ok: bool) -> Result<Run> {
    let (all, truncated_lines) = records(transcript, truncation_ok)?;
    let recs: Vec<&Value> = all
        .iter()
        .filter(|r| r["type"] == "user" || r["type"] == "assistant")
        .collect();
    // each sidechain record's session, by the uuid of the chain's first record
    let mut roots: Vec<(&str, &str)> = vec![];
    let mut sessions: Vec<(String, Vec<&Value>)> = vec![("main".into(), vec![])];
    for r in &recs {
        if r["isSidechain"] != true {
            sessions[0].1.push(r);
            continue;
        }
        let id = r["uuid"].as_str().unwrap_or_default();
        let parent = r["parentUuid"].as_str();
        let root = match roots.iter().find(|(u, _)| Some(*u) == parent) {
            Some((_, root)) => *root,
            None => {
                sessions.push((id.to_string(), vec![]));
                id
            }
        };
        roots.push((id, root));
        if let Some(s) = sessions.iter_mut().find(|(n, _)| n == root) {
            s.1.push(r);
        }
    }

    let mut peak_main_context = 0;
    let mut models = BTreeSet::new();
    let mut all_actions = vec![];
    let mut tree_files: Vec<String> = vec![];
    let mut reviews = vec![];
    let mut git_history_commands = vec![];
    let mut failed_children = vec![];
    let mut thinking_levels = BTreeSet::new();
    let mut sessions_without_thinking = vec![];
    let root = recs
        .iter()
        .find_map(|r| r["cwd"].as_str())
        .unwrap_or_default();
    for (i, (name, recs)) in sessions.iter().enumerate() {
        match thinking(recs) {
            Some(t) => {
                thinking_levels.insert(t.to_string());
            }
            None => sessions_without_thinking.push(name.clone()),
        }
        for r in responses(recs) {
            if i == 0 {
                let t = tokens(&r["message"]["usage"], USAGE);
                peak_main_context = peak_main_context.max(t.context());
            }
            match r["message"]["model"].as_str() {
                Some(m) if m != "<synthetic>" => {
                    models.insert(m.to_string());
                }
                _ => {}
            }
        }
        all_actions.extend(actions(recs, i > 0));
        if i > 0 {
            reviews.extend(review_submits(recs));
            if let Some(s) = last_stop(recs).filter(|s| FAILED_STOPS.contains(&s.as_str())) {
                failed_children.push(format!("{name} stopped with `{s}`"));
            }
        }
        git_history_commands.extend(
            bash_commands(recs)
                .map(|(_, c)| c)
                .filter(|c| reads_git_history(c))
                .map(String::from),
        );
        for f in files_opened(recs, root) {
            if !tree_files.contains(&f) {
                tree_files.push(f);
            }
        }
    }
    let main = &sessions[0].1;
    let result = stream_result(transcript);
    let stop_reason = if result.as_ref().is_some_and(|r| r["is_error"] == true) {
        Some("error".to_string())
    } else {
        last_stop(main)
    };
    let main_tokens = result.as_ref().map(|r| tokens(&r["usage"], USAGE));
    let total_tokens = result.as_ref().map(|r| {
        let mut t = Tokens::default();
        for (model, u) in r["modelUsage"].as_object().into_iter().flatten() {
            models.insert(model.clone());
            t.add(tokens(u, MODEL_USAGE));
        }
        t
    });
    reviews.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(Run {
        main_tokens,
        total_tokens,
        cache_warming: Tokens::default(),
        peak_main_context,
        tool_calls: tool_uses(main)
            .map(|b| b["name"].as_str().unwrap_or_default().to_string())
            .collect(),
        files_opened: files_opened(main, root),
        files_opened_tree: tree_files,
        shell_commands: bash_commands(main).map(|(_, c)| c.to_string()).collect(),
        ritual_steps: ritual_steps(&all_actions),
        cost_usd: None,
        harness_reported_cost_usd: result.and_then(|r| r["total_cost_usd"].as_f64()),
        duration_s,
        model: responses(main)
            .iter()
            .find_map(|r| {
                r["message"]["model"]
                    .as_str()
                    .filter(|m| *m != "<synthetic>")
            })
            .map(String::from),
        models: models.into_iter().collect(),
        thinking_levels: thinking_levels.into_iter().collect(),
        sessions_without_thinking,
        subagent_sessions: sessions.len() - 1,
        reviews: reviews.into_iter().map(|(_, r)| r).collect(),
        git_history_commands,
        stop_reason,
        failed_children,
        truncated_lines,
    })
}

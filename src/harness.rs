// The per-harness facts table (DESIGN.md Facts, "Harness table"). Rules never name a harness; this does.
use std::ffi::OsString;
use std::path::Path;

use crate::output::CliError;

#[derive(Debug, PartialEq, Eq)]
pub enum Kind {
    /// The harness has a role-file format: `render` writes one file per role.
    Rendered,
    /// No role-file format: the brief also carries the role text.
    Briefed,
}

#[derive(Debug)]
pub enum EnvRule {
    /// Detected when any env var name starts with this prefix.
    Prefix(&'static str),
    /// Marked UNVERIFIED in Facts: never detected from env.
    Unverified,
}

#[derive(Debug)]
pub struct Harness {
    pub name: &'static str,
    pub kind: Kind,
    /// Role-file directory under the home dir (`~/...`); `None` when none is verified.
    pub user_dir: Option<&'static str>,
    /// Role-file directory in the repo.
    pub repo_dir: Option<&'static str>,
    /// Role-file extension; `None` for briefed harnesses.
    pub file_format: Option<&'static str>,
    pub spawn_tool: &'static str,
    pub model_pin: bool,
    pub env: EnvRule,
    pub delegation: &'static str,
    pub verified: &'static str,
    /// How `stewards review` and every `--run` start this harness's own headless session.
    pub launch: Launch,
}

/// One part of a launch line: arguments, environment, nothing needed, or not known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    Args(&'static [&'static str]),
    /// Variables set in the session's environment (a harness with no flag for it).
    Env(&'static [(&'static str, &'static str)]),
    /// The harness needs nothing for this (it has no permission prompts, say).
    NotNeeded,
    /// No source gives it: a line with a required part unknown is not started.
    Unknown,
}

/// A headless launch line, data only: one code path starts every harness from it. The session
/// always starts in the repo root (its process cwd).
#[derive(Debug)]
pub struct Launch {
    /// `None`: unknown.
    pub program: Option<&'static str>,
    /// The argument template. Whole arguments `{unprompted}`, `{no_instructions}`, and `{model}`
    /// expand to those parts (`{model}` to nothing when no model is chosen), `{dir}` is the repo
    /// root, and `{prompt}` the fixed sentence; without `{prompt}` the prompt is unknown.
    pub argv: &'static [&'static str],
    /// What lets the session run shell and write files with no permission prompt.
    pub unprompted: Setting,
    /// What stops project instruction files (AGENTS.md, CLAUDE.md, ...) loading.
    pub no_instructions: Setting,
    /// How a model is chosen, `{id}` its id; required only when one is.
    pub model: Setting,
    /// Variables kept from the caller though their harness prefix is stripped: configuration,
    /// not a tie to the outer session.
    pub keep_env: &'static [&'static str],
    /// Where the session reports usage on stdout; `None`: unknown or not reported.
    pub usage: Option<Usage>,
    /// `verified <date>` when run on the maintainer's machine, else `UNVERIFIED (<source>)`.
    pub verified: &'static str,
}

/// Where usage is in a session's stdout, one JSON object per line: each line whose `when`
/// pointers hold the given strings carries counts at `at` + each of `keys` (input, output, cache
/// read, cache write; `""` or an absent key counts 0) and the message text at `text` (a string,
/// or an array of blocks with `text`). `sum`: every such line adds; else the last is the total.
#[derive(Debug)]
pub struct Usage {
    pub when: &'static [(&'static str, &'static str)],
    pub at: &'static str,
    pub keys: [&'static str; 4],
    pub text: Option<&'static str>,
    pub sum: bool,
}

/// What a launch line gives for one session: the program, its arguments, and the variables to
/// set.
pub type Started = (
    &'static str,
    Vec<OsString>,
    Vec<(&'static str, &'static str)>,
);

impl Launch {
    /// The session for `prompt` in `dir` (with `model` when one is chosen), or the name of the
    /// first required part no source gives. Placeholders are whole arguments, so nothing in the
    /// prompt or the path is ever read as one.
    pub fn argv(
        &self,
        dir: &Path,
        model: Option<&str>,
        prompt: &str,
    ) -> Result<Started, &'static str> {
        let program = self.program.ok_or("program")?;
        if !self.argv.contains(&"{prompt}") {
            return Err("prompt");
        }
        let mut args = vec![];
        let mut env = vec![];
        for &a in self.argv {
            let (name, setting) = match a {
                "{unprompted}" => ("unprompted", self.unprompted),
                "{no_instructions}" => ("no_instructions", self.no_instructions),
                "{model}" if model.is_none() => continue,
                "{model}" => ("model", self.model),
                "{dir}" => {
                    args.push(dir.as_os_str().to_owned());
                    continue;
                }
                "{prompt}" => {
                    args.push(OsString::from(prompt));
                    continue;
                }
                _ => {
                    args.push(OsString::from(a));
                    continue;
                }
            };
            match setting {
                Setting::Args(list) => args.extend(
                    list.iter()
                        .map(|x| OsString::from(x.replace("{id}", model.unwrap_or_default()))),
                ),
                Setting::Env(vars) => env.extend_from_slice(vars),
                Setting::NotNeeded => {}
                Setting::Unknown => return Err(name),
            }
        }
        Ok((program, args, env))
    }
}

/// A launch line no source gives.
const UNKNOWN_LAUNCH: Launch = Launch {
    program: None,
    argv: &[],
    unprompted: Setting::Unknown,
    no_instructions: Setting::Unknown,
    model: Setting::Unknown,
    keep_env: &[],
    usage: None,
    verified: "UNVERIFIED (no source)",
};

/// What every launched session may do, the same for every harness, for `guide`.
pub const LAUNCH_NOTE: &str = "`stewards review` and every `--run` start this harness's own headless session on each brief, with permission prompts off and no project instruction files loaded: the session can run any shell command and write any file as you, in the repo's directory, the same reach a role subagent has.";

const LAUNCH_VERIFIED: &str = "verified 2026-11-13";
/// The sourced lines' research date.
macro_rules! docs {
    ($url:literal) => {
        concat!("UNVERIFIED (docs 2026-11-13, ", $url, ")")
    };
}

/// Used when no harness is detected.
pub const GENERIC_DELEGATION: &str = "Start a fresh-context subagent that can run shell, give it the absolute brief path and nothing else, and wait for its reply: what `submit` printed.";

const VERIFIED: &str = "2026-10-21";

pub static HARNESSES: [Harness; 10] = [
    Harness {
        name: "claude-code",
        kind: Kind::Rendered,
        user_dir: Some("~/.claude/agents"),
        repo_dir: Some(".claude/agents"),
        file_format: Some("md"),
        spawn_tool: "Agent tool",
        model_pin: true,
        env: EnvRule::Prefix("CLAUDE_CODE"),
        delegation: "Run `stewards render --harness claude-code` once (add `--repo-level` for a team). Then start the role's agent with the Agent tool, give it the absolute brief path and nothing else, and wait for its reply: what `submit` printed.",
        verified: VERIFIED,
        launch: Launch {
            program: Some("claude"),
            argv: &[
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "{unprompted}",
                "{no_instructions}",
                "{model}",
                "--",
                "{prompt}",
            ],
            unprompted: Setting::Args(&["--dangerously-skip-permissions"]),
            no_instructions: Setting::Args(&["--setting-sources", "user"]),
            model: Setting::Args(&["--model", "{id}"]),
            keep_env: &[],
            usage: Some(Usage {
                when: &[("/type", "result")],
                at: "/usage",
                keys: [
                    "/input_tokens",
                    "/output_tokens",
                    "/cache_read_input_tokens",
                    "/cache_creation_input_tokens",
                ],
                text: Some("/result"),
                sum: false,
            }),
            verified: LAUNCH_VERIFIED,
        },
    },
    Harness {
        name: "cursor",
        kind: Kind::Rendered,
        user_dir: None,
        repo_dir: Some(".cursor/agents"),
        file_format: Some("md"),
        spawn_tool: "Task tool",
        model_pin: true,
        env: EnvRule::Unverified,
        delegation: "Run `stewards render --harness cursor --repo-level` once. Then start the role's agent with the Task tool, give it the absolute brief path and nothing else, and wait for its reply: what `submit` printed.",
        verified: VERIFIED,
        launch: Launch {
            program: Some("agent"),
            argv: &[
                "-p",
                "--workspace",
                "{dir}",
                "{unprompted}",
                "{no_instructions}",
                "{model}",
                "{prompt}",
            ],
            unprompted: Setting::Args(&["--force", "--trust"]),
            model: Setting::Args(&["--model", "{id}"]),
            verified: docs!("https://cursor.com/docs/cli/reference/parameters"),
            ..UNKNOWN_LAUNCH
        },
    },
    Harness {
        name: "codex",
        kind: Kind::Rendered,
        user_dir: Some("~/.codex/agents"),
        repo_dir: Some(".codex/agents"),
        file_format: Some("toml"),
        spawn_tool: "spawn tool",
        model_pin: true,
        env: EnvRule::Prefix("CODEX_"),
        delegation: "Run `stewards render --harness codex` once (add `--repo-level` for a team). Then start the role's agent with the spawn tool, give it the absolute brief path and nothing else, and wait for its reply: what `submit` printed.",
        verified: VERIFIED,
        launch: Launch {
            program: Some("codex"),
            argv: &[
                "{no_instructions}",
                "exec",
                "{unprompted}",
                "-C",
                "{dir}",
                "{model}",
                "--json",
                "{prompt}",
            ],
            unprompted: Setting::Args(&["--dangerously-bypass-approvals-and-sandbox"]),
            // a config override: no project instruction bytes are read
            no_instructions: Setting::Args(&["-c", "project_doc_max_bytes=0"]),
            model: Setting::Args(&["-m", "{id}"]),
            keep_env: &[],
            usage: Some(Usage {
                when: &[("/type", "turn.completed")],
                at: "/usage",
                keys: [
                    "/input_tokens",
                    "/output_tokens",
                    "/cached_input_tokens",
                    "/cache_write_input_tokens",
                ],
                text: None,
                sum: true,
            }),
            verified: docs!("https://github.com/openai/codex"),
        },
    },
    Harness {
        name: "copilot",
        kind: Kind::Rendered,
        user_dir: None,
        repo_dir: Some(".github/agents"),
        file_format: Some("agent.md"),
        spawn_tool: "runSubagent",
        model_pin: true,
        env: EnvRule::Unverified,
        delegation: "Run `stewards render --harness copilot --repo-level` once. Then start the role's agent with `runSubagent`, give it the absolute brief path and nothing else, and wait for its reply: what `submit` printed.",
        verified: VERIFIED,
        launch: Launch {
            program: Some("copilot"),
            argv: &[
                "-C",
                "{dir}",
                "{unprompted}",
                "{no_instructions}",
                "{model}",
                "-p",
                "{prompt}",
            ],
            unprompted: Setting::Args(&["--allow-all", "--no-ask-user"]),
            no_instructions: Setting::Args(&["--no-custom-instructions"]),
            model: Setting::Args(&["--model={id}"]),
            keep_env: &[],
            usage: None,
            verified: docs!(
                "https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-command-reference"
            ),
        },
    },
    Harness {
        name: "gemini",
        kind: Kind::Rendered,
        user_dir: Some("~/.gemini/agents"),
        repo_dir: Some(".gemini/agents"),
        file_format: Some("md"),
        spawn_tool: "@name",
        model_pin: true,
        env: EnvRule::Unverified,
        delegation: "Run `stewards render --harness gemini` once (add `--repo-level` for a team). Then call the role's agent with `@name`, give it the absolute brief path and nothing else, and wait for its reply: what `submit` printed.",
        verified: VERIFIED,
        launch: Launch {
            program: Some("gemini"),
            argv: &[
                "{unprompted}",
                "{no_instructions}",
                "{model}",
                "--output-format",
                "stream-json",
                "-p",
                "{prompt}",
            ],
            unprompted: Setting::Args(&["--approval-mode=yolo"]),
            model: Setting::Args(&["-m", "{id}"]),
            usage: Some(Usage {
                when: &[("/type", "result")],
                at: "/stats",
                keys: ["/input_tokens", "/output_tokens", "/cached", ""],
                text: None,
                sum: false,
            }),
            verified: docs!("https://github.com/google-gemini/gemini-cli"),
            ..UNKNOWN_LAUNCH
        },
    },
    Harness {
        name: "opencode",
        kind: Kind::Rendered,
        user_dir: Some("~/.config/opencode/agents"),
        repo_dir: Some(".opencode/agents"),
        file_format: Some("md"),
        spawn_tool: "Task tool",
        model_pin: true,
        env: EnvRule::Unverified,
        delegation: "Run `stewards render --harness opencode` once (add `--repo-level` for a team). Then start the role's agent with the Task tool, give it the absolute brief path and nothing else, and wait for its reply: what `submit` printed.",
        verified: VERIFIED,
        launch: Launch {
            program: Some("opencode"),
            argv: &[
                "run",
                "{unprompted}",
                "{no_instructions}",
                "{model}",
                "--format",
                "json",
                "{prompt}",
            ],
            unprompted: Setting::Args(&["--auto"]),
            no_instructions: Setting::Env(&[("OPENCODE_DISABLE_PROJECT_CONFIG", "1")]),
            model: Setting::Args(&["--model", "{id}"]),
            keep_env: &[],
            usage: Some(Usage {
                when: &[("/type", "step-finish")],
                at: "/tokens",
                keys: ["/input", "/output", "/cache/read", "/cache/write"],
                text: None,
                sum: true,
            }),
            verified: docs!("https://opencode.ai/docs/cli/"),
        },
    },
    Harness {
        name: "pi",
        kind: Kind::Rendered,
        user_dir: Some("~/.pi/agent/agents"),
        repo_dir: Some(".pi/agents"),
        file_format: Some("md"),
        spawn_tool: "subagent tool",
        model_pin: true,
        env: EnvRule::Prefix("PI_"),
        delegation: "Needs pi-subagents (`pi install npm:pi-subagents`). Start its `worker` agent with the `subagent` tool, give it the absolute brief path and nothing else, and wait for its reply: what `submit` printed. Do not hand a brief to pi-subagents' bundled `reviewer` agent: it is read-only and cannot write the answer or run `submit`.",
        verified: VERIFIED,
        launch: Launch {
            program: Some("pi"),
            argv: &[
                "--mode",
                "json",
                "--no-session",
                "--no-extensions",
                "--no-skills",
                "--no-prompt-templates",
                "--no-themes",
                "--no-mcp",
                "--no-approve",
                "{unprompted}",
                "{no_instructions}",
                "{model}",
                "--",
                "{prompt}",
            ],
            // Pi asks no approval before a tool runs
            unprompted: Setting::NotNeeded,
            no_instructions: Setting::Args(&["--no-context-files"]),
            model: Setting::Args(&["--model", "{id}"]),
            keep_env: &["PI_CODING_AGENT_DIR"],
            usage: Some(Usage {
                when: &[("/type", "message_end"), ("/message/role", "assistant")],
                at: "/message/usage",
                keys: ["/input", "/output", "/cacheRead", "/cacheWrite"],
                text: Some("/message/content"),
                sum: true,
            }),
            verified: LAUNCH_VERIFIED,
        },
    },
    Harness {
        name: "hermes",
        kind: Kind::Briefed,
        user_dir: None,
        repo_dir: None,
        file_format: None,
        spawn_tool: "delegate_task",
        model_pin: false,
        env: EnvRule::Prefix("HERMES_"),
        delegation: "`delegate_task(tasks=[{goal: \"Read <absolute brief path> and follow it\", context: \"<repo path>\"}])`; children run in the background by default, wait for the completion message; `role` is ignored by the source.",
        verified: VERIFIED,
        launch: Launch {
            program: Some("hermes"),
            argv: &[
                "chat",
                "--oneshot",
                "--in",
                "{dir}",
                "{unprompted}",
                "{no_instructions}",
                "{model}",
                "--format",
                "stream-json",
                "-q",
                "{prompt}",
            ],
            unprompted: Setting::Args(&["--yolo"]),
            no_instructions: Setting::Args(&["--ignore-rules"]),
            model: Setting::Args(&["-m", "{id}"]),
            keep_env: &[],
            usage: Some(Usage {
                when: &[("/type", "result")],
                at: "/tokens",
                keys: ["/input", "/output", "/cache_read", "/cache_write"],
                text: None,
                sum: false,
            }),
            verified: docs!("https://github.com/NousResearch/hermes-agent"),
        },
    },
    Harness {
        name: "zeroclaw",
        kind: Kind::Briefed,
        user_dir: None,
        repo_dir: None,
        file_format: None,
        spawn_tool: "spawn_subagent, delegate",
        model_pin: true,
        env: EnvRule::Unverified,
        delegation: "`spawn_subagent{prompt: \"Read <absolute brief path> and follow it\"}` (same agent, fresh session, blocks; the risk profile must not exclude it), or `delegate{agent, prompt, context}` to a same-profile agent in bounded mode.",
        verified: VERIFIED,
        // `--agent <alias>` is required and the no-prompts setting is a config key only
        launch: Launch {
            program: Some("zeroclaw"),
            argv: &[
                "agent",
                "{unprompted}",
                "{no_instructions}",
                "{model}",
                "-m",
                "{prompt}",
            ],
            model: Setting::Args(&["--model", "{id}"]),
            verified: docs!("https://github.com/zeroclaw-labs/zeroclaw"),
            ..UNKNOWN_LAUNCH
        },
    },
    Harness {
        name: "borg",
        kind: Kind::Briefed,
        user_dir: None,
        repo_dir: None,
        file_format: None,
        spawn_tool: "spawn_agent",
        model_pin: true,
        env: EnvRule::Unverified,
        delegation: "`spawn_agent(role=\"coder\", blocking=true, message=\"Read <absolute brief path> and follow it\", context=\"<task>\")`, or a fresh `borg ask` session per brief; run inside the repo workspace (`-w` or `borg acp`), because `borg ask` defaults to the daemon's cwd.",
        verified: VERIFIED,
        launch: Launch {
            program: Some("borg"),
            argv: &[
                "ask",
                "--json",
                "{unprompted}",
                "{no_instructions}",
                "{prompt}",
            ],
            // no approval prompts by design; the model is a setting, not a flag
            unprompted: Setting::NotNeeded,
            verified: "UNVERIFIED (borg CLI docs, 2026-11-13)",
            ..UNKNOWN_LAUNCH
        },
    },
];

/// The first harness whose verified env rule matches the current environment.
pub fn detect() -> Option<&'static Harness> {
    let names: Vec<String> = std::env::vars_os()
        .map(|(k, _)| k.to_string_lossy().into_owned())
        .collect();
    HARNESSES.iter().find(|h| match h.env {
        EnvRule::Prefix(p) => names.iter().any(|n| n.starts_with(p)),
        EnvRule::Unverified => false,
    })
}

pub fn by_name(name: &str) -> anyhow::Result<&'static Harness> {
    HARNESSES
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| {
            let valid = HARNESSES
                .iter()
                .map(|h| h.name)
                .collect::<Vec<_>>()
                .join(", ");
            CliError::new(
                "harness_unknown",
                format!("unknown harness `{name}`; valid: {valid}"),
                format!("use one of: {valid}"),
            )
            .into()
        })
}

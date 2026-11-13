// The per-harness facts table (DESIGN.md Facts, "Harness table"). Rules never name a harness; this does.
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
}

/// Used when no harness is detected or the one named is not in the table.
pub const GENERIC_DELEGATION: &str = "Start a fresh-context, general-purpose subagent that can run shell and write files, give it the handoff stewards printed (`Read <absolute brief path> and follow it.`) unchanged as its whole task, and wait for its reply: what `submit` printed.";

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
        delegation: "Run `stewards render --harness claude-code` once (add `--repo-level` for a team). Then start the role's agent with the Agent tool, give it the handoff unchanged as its whole task, and wait for its reply: what `submit` printed.",
        verified: VERIFIED,
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
        delegation: "Run `stewards render --harness cursor --repo-level` once. Then start the role's agent with the Task tool, give it the handoff unchanged as its whole task, and wait for its reply: what `submit` printed.",
        verified: VERIFIED,
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
        delegation: "Run `stewards render --harness codex` once (add `--repo-level` for a team). Then start the role's agent with the spawn tool, give it the handoff unchanged as its whole task, and wait for its reply: what `submit` printed.",
        verified: VERIFIED,
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
        delegation: "Run `stewards render --harness copilot --repo-level` once. Then start the role's agent with `runSubagent`, give it the handoff unchanged as its whole task, and wait for its reply: what `submit` printed.",
        verified: VERIFIED,
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
        delegation: "Run `stewards render --harness gemini` once (add `--repo-level` for a team). Then call the role's agent with `@name`, give it the handoff unchanged as its whole task, and wait for its reply: what `submit` printed.",
        verified: VERIFIED,
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
        delegation: "Run `stewards render --harness opencode` once (add `--repo-level` for a team). Then start the role's agent with the Task tool, give it the handoff unchanged as its whole task, and wait for its reply: what `submit` printed.",
        verified: VERIFIED,
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
        delegation: "Needs pi-subagents (`pi install npm:pi-subagents`). Start its `worker` agent with the `subagent` tool, give it the handoff unchanged as its whole task, and wait for its reply: what `submit` printed. Do not hand a brief to pi-subagents' bundled `reviewer` agent: it is read-only and cannot write the answer or run `submit`.",
        verified: VERIFIED,
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

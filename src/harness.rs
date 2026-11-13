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
    pub model_pin: bool,
    pub env: EnvRule,
    pub verified: &'static str,
}

/// How to delegate a brief, the same in every harness, known or not.
pub const DELEGATION: &str = "Start a fresh-context, general-purpose subagent that can run shell and write files, give it the handoff stewards printed (``Read `<absolute brief path>` and follow it.``) unchanged as its whole task, and wait for its reply: what `submit` printed.";

const VERIFIED: &str = "2026-10-21";

pub static HARNESSES: [Harness; 10] = [
    Harness {
        name: "claude-code",
        kind: Kind::Rendered,
        user_dir: Some("~/.claude/agents"),
        repo_dir: Some(".claude/agents"),
        file_format: Some("md"),
        model_pin: true,
        env: EnvRule::Prefix("CLAUDE_CODE"),
        verified: VERIFIED,
    },
    Harness {
        name: "cursor",
        kind: Kind::Rendered,
        user_dir: None,
        repo_dir: Some(".cursor/agents"),
        file_format: Some("md"),
        model_pin: true,
        env: EnvRule::Unverified,
        verified: VERIFIED,
    },
    Harness {
        name: "codex",
        kind: Kind::Rendered,
        user_dir: Some("~/.codex/agents"),
        repo_dir: Some(".codex/agents"),
        file_format: Some("toml"),
        model_pin: true,
        env: EnvRule::Prefix("CODEX_"),
        verified: VERIFIED,
    },
    Harness {
        name: "copilot",
        kind: Kind::Rendered,
        user_dir: None,
        repo_dir: Some(".github/agents"),
        file_format: Some("agent.md"),
        model_pin: true,
        env: EnvRule::Unverified,
        verified: VERIFIED,
    },
    Harness {
        name: "gemini",
        kind: Kind::Rendered,
        user_dir: Some("~/.gemini/agents"),
        repo_dir: Some(".gemini/agents"),
        file_format: Some("md"),
        model_pin: true,
        env: EnvRule::Unverified,
        verified: VERIFIED,
    },
    Harness {
        name: "opencode",
        kind: Kind::Rendered,
        user_dir: Some("~/.config/opencode/agents"),
        repo_dir: Some(".opencode/agents"),
        file_format: Some("md"),
        model_pin: true,
        env: EnvRule::Unverified,
        verified: VERIFIED,
    },
    Harness {
        name: "pi",
        kind: Kind::Rendered,
        user_dir: Some("~/.pi/agent/agents"),
        repo_dir: Some(".pi/agents"),
        file_format: Some("md"),
        model_pin: true,
        env: EnvRule::Prefix("PI_"),
        verified: VERIFIED,
    },
    Harness {
        name: "hermes",
        kind: Kind::Briefed,
        user_dir: None,
        repo_dir: None,
        file_format: None,
        model_pin: false,
        env: EnvRule::Prefix("HERMES_"),
        verified: VERIFIED,
    },
    Harness {
        name: "zeroclaw",
        kind: Kind::Briefed,
        user_dir: None,
        repo_dir: None,
        file_format: None,
        model_pin: true,
        env: EnvRule::Unverified,
        verified: VERIFIED,
    },
    Harness {
        name: "borg",
        kind: Kind::Briefed,
        user_dir: None,
        repo_dir: None,
        file_format: None,
        model_pin: true,
        env: EnvRule::Unverified,
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

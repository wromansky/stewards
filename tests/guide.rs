use std::path::Path;
use std::process::Command;

use serde_json::Value;
use stewards::guide::{block, upsert_block};
use tempfile::TempDir;

/// Runs `stewards guide <args>` with an empty environment plus `env`, outside any git repo.
fn guide(env: &[(&str, &str)], args: &[&str]) -> (Value, Option<i32>) {
    let dir = TempDir::new().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_stewards"))
        .arg("guide")
        .args(args)
        .env_clear()
        .envs(env.iter().copied())
        .current_dir(dir.path())
        .output()
        .unwrap();
    let v = serde_json::from_slice(&out.stdout).expect("stdout is JSON");
    (v, out.status.code())
}

fn detected(var: &str) -> Value {
    guide(&[(var, "1")], &[]).0["harness"].clone()
}

#[test]
fn claude_code_is_detected_from_env() {
    assert_eq!(detected("CLAUDE_CODE_ENTRYPOINT"), "claude-code");
}

#[test]
fn codex_is_detected_from_env() {
    assert_eq!(detected("CODEX_HOME"), "codex");
}

#[test]
fn pi_is_detected_from_env() {
    assert_eq!(detected("PI_CODING_AGENT"), "pi");
}

#[test]
fn hermes_is_detected_from_env() {
    let (v, _) = guide(&[("HERMES_HOME", "/x")], &[]);
    assert_eq!(v["harness"], "hermes");
    assert!(v["delegation"].as_str().unwrap().contains("delegate_task"));
}

#[test]
fn unverified_harness_is_not_detected() {
    for var in [
        "CURSOR_TRACE_ID",
        "GEMINI_CLI",
        "OPENCODE",
        "ZEROCLAW_HOME",
        "BORG_HOME",
    ] {
        assert_eq!(detected(var), Value::Null, "{var}");
    }
}

#[test]
fn nothing_detected_prints_the_generic_paragraph() {
    let (v, code) = guide(&[], &[]);
    assert_eq!(code, Some(0));
    assert_eq!(v["harness"], Value::Null);
    assert!(
        v["delegation"]
            .as_str()
            .unwrap()
            .starts_with("Start a fresh-context subagent that can run shell")
    );
}

#[test]
fn guide_has_the_block_sections_as_named_fields() {
    let (v, _) = guide(&[], &[]);
    assert!(v["stewards"]["version"].is_string());
    assert!(
        v["ritual"]
            .as_str()
            .unwrap()
            .starts_with("Ritual\n- Entering")
    );
    assert!(v["roles"].as_str().unwrap().starts_with("Roles ("));
    assert!(v["editing"].as_str().unwrap().starts_with("Editing\n"));
    assert!(v["hygiene"].as_str().unwrap().starts_with("Hygiene:"));
    assert!(v.get("errors").is_none() && v.get("glossary").is_none());
}

#[test]
fn harness_flag_forces_a_harness() {
    let (v, _) = guide(&[("HERMES_HOME", "/x")], &["--harness", "pi"]);
    assert_eq!(v["harness"], "pi");
}

#[test]
fn unknown_harness_flag_is_harness_unknown() {
    let (v, code) = guide(&[], &["--harness", "emacs"]);
    assert_eq!(code, Some(2));
    assert_eq!(v["error"]["code"], "harness_unknown");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("claude-code")
    );
}

#[test]
fn all_names_every_harness() {
    let (v, _) = guide(&[], &["--all"]);
    let names: Vec<_> = v["all_delegations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["harness"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "claude-code",
            "cursor",
            "codex",
            "copilot",
            "gemini",
            "opencode",
            "pi",
            "hermes",
            "zeroclaw",
            "borg"
        ]
    );
}

#[test]
fn errors_lists_every_code_in_the_spec_table() {
    let (v, _) = guide(&[], &["--errors"]);
    let codes: Vec<_> = v["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["code"].as_str().unwrap())
        .collect();
    assert_eq!(
        codes,
        [
            "not_a_git_repo",
            "no_stewards_dir",
            "schema_too_new",
            "invalid_toml",
            "unknown_unit",
            "unknown_role",
            "answer_parse_failed",
            "answer_dead_anchor",
            "answer_too_long",
            "command_not_allowed",
            "command_timeout",
            "workspace_repo_missing",
            "baml_runtime_unavailable",
            "no_api_key",
            "tracker_auth",
            "harness_unknown"
        ]
    );
}

#[test]
fn glossary_lists_terms() {
    let (v, _) = guide(&[], &["--glossary"]);
    let terms = v["glossary"].as_array().unwrap();
    assert!(terms.iter().any(|t| t["term"] == "stamp"));
}

#[test]
fn human_renders_markdown_without_envelope() {
    let dir = TempDir::new().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_stewards"))
        .args(["guide", "--human", "--errors"])
        .env_clear()
        .current_dir(dir.path())
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.starts_with("# stewards guide"));
    assert!(text.contains("| `harness_unknown` |"));
}

#[test]
fn block_is_bracketed_by_its_markers() {
    assert!(block().starts_with("<!-- stewards:start -->\n## Stewards\n"));
    assert!(block().ends_with("\n<!-- stewards:end -->"));
}

fn agents_md(dir: &Path) -> std::path::PathBuf {
    dir.join("AGENTS.md")
}

#[test]
fn upsert_creates_a_missing_file() {
    let dir = TempDir::new().unwrap();
    assert!(upsert_block(&agents_md(dir.path())).unwrap());
    let text = std::fs::read_to_string(agents_md(dir.path())).unwrap();
    assert_eq!(text, format!("{}\n", block()));
}

#[test]
fn upsert_puts_the_block_on_top_and_keeps_other_content() {
    let dir = TempDir::new().unwrap();
    std::fs::write(agents_md(dir.path()), "# Mine\n\nrules\n").unwrap();
    assert!(upsert_block(&agents_md(dir.path())).unwrap());
    let text = std::fs::read_to_string(agents_md(dir.path())).unwrap();
    assert_eq!(text, format!("{}\n\n# Mine\n\nrules\n", block()));
}

#[test]
fn upsert_replaces_an_older_block_in_place() {
    let dir = TempDir::new().unwrap();
    let old = "# Top\n\n<!-- stewards:start -->\nold text\n<!-- stewards:end -->\n\n# Bottom\n";
    std::fs::write(agents_md(dir.path()), old).unwrap();
    assert!(upsert_block(&agents_md(dir.path())).unwrap());
    let text = std::fs::read_to_string(agents_md(dir.path())).unwrap();
    assert_eq!(text, format!("# Top\n\n{}\n\n# Bottom\n", block()));
}

#[test]
fn second_upsert_changes_nothing() {
    let dir = TempDir::new().unwrap();
    std::fs::write(agents_md(dir.path()), "# Mine\n").unwrap();
    upsert_block(&agents_md(dir.path())).unwrap();
    let first = std::fs::read(agents_md(dir.path())).unwrap();
    assert!(!upsert_block(&agents_md(dir.path())).unwrap());
    assert_eq!(std::fs::read(agents_md(dir.path())).unwrap(), first);
}

#[test]
fn block_equals_the_block_in_the_design_record() {
    let design = include_str!("../DESIGN.md");
    let start = design.find("<!-- stewards:start -->").unwrap();
    let end_marker = "<!-- stewards:end -->";
    let end = start + design[start..].find(end_marker).unwrap() + end_marker.len();
    assert_eq!(&design[start..end], block());
}

#[test]
fn briefed_delegation_text_matches_facts_word_for_word() {
    let (v, _) = guide(&[], &["--all"]);
    let text = |name: &str| {
        v["all_delegations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["harness"] == name)
            .unwrap()["text"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert_eq!(
        text("hermes"),
        r#"`delegate_task(tasks=[{goal: "Read <absolute brief path> and follow it", context: "<repo path>"}])`; children run in the background by default, wait for the completion message; `role` is ignored by the source."#
    );
    assert_eq!(
        text("zeroclaw"),
        r#"`spawn_subagent{prompt: "Read <absolute brief path> and follow it"}` (same agent, fresh session, blocks; the risk profile must not exclude it), or `delegate{agent, prompt, context}` to a same-profile agent in bounded mode."#
    );
    assert_eq!(
        text("borg"),
        r#"`spawn_agent(role="coder", blocking=true, message="Read <absolute brief path> and follow it", context="<task>")`, or a fresh `borg ask` session per brief; run inside the repo workspace (`-w` or `borg acp`), because `borg ask` defaults to the daemon's cwd."#
    );
}

#[test]
fn zeroclaw_and_borg_delegation_name_their_spawn_tool() {
    let zeroclaw = guide(&[], &["--harness", "zeroclaw"]).0;
    assert!(
        zeroclaw["delegation"]
            .as_str()
            .unwrap()
            .contains("spawn_subagent")
    );
    let borg = guide(&[], &["--harness", "borg"]).0;
    assert!(borg["delegation"].as_str().unwrap().contains("spawn_agent"));
}

#[test]
fn guide_output_names_all_six_roles() {
    let (v, _) = guide(&[], &[]);
    let roles = v["roles"].as_str().unwrap();
    for role in [
        "maintainer",
        "tester",
        "documenter",
        "validator",
        "architect",
        "developer",
    ] {
        assert!(roles.contains(&format!("- {role}:")), "{role}");
    }
}

#[test]
fn upsert_keeps_crlf_content_byte_for_byte() {
    let dir = TempDir::new().unwrap();
    let old = "# Mine\r\n\r\nrules\r\n";
    std::fs::write(agents_md(dir.path()), old).unwrap();
    assert!(upsert_block(&agents_md(dir.path())).unwrap());
    let text = std::fs::read(agents_md(dir.path())).unwrap();
    assert!(text.ends_with(old.as_bytes()));
    assert_eq!(text.len(), block().len() + 2 + old.len());
}

#[test]
fn upsert_keeps_content_without_trailing_newline() {
    let dir = TempDir::new().unwrap();
    std::fs::write(agents_md(dir.path()), "# Mine\nno newline").unwrap();
    assert!(upsert_block(&agents_md(dir.path())).unwrap());
    let text = std::fs::read_to_string(agents_md(dir.path())).unwrap();
    assert_eq!(text, format!("{}\n\n# Mine\nno newline", block()));
}

#[test]
fn upsert_replacing_a_block_keeps_crlf_and_missing_final_newline_outside_it() {
    let dir = TempDir::new().unwrap();
    let old = "# Top\r\n<!-- stewards:start -->\r\nold\r\n<!-- stewards:end -->\r\nend";
    std::fs::write(agents_md(dir.path()), old).unwrap();
    assert!(upsert_block(&agents_md(dir.path())).unwrap());
    let text = std::fs::read_to_string(agents_md(dir.path())).unwrap();
    assert_eq!(text, format!("# Top\r\n{}\r\nend", block()));
}

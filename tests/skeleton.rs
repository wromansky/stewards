mod common;

use std::path::Path;

use common::{fixture, git, stewards};
use serde_json::Value;
use stewards::model::{Anchor, CommandEntry, GapKind, TrackerKind, load_repo};
use stewards::output::CliError;

fn load_err(root: &Path) -> CliError {
    let e = load_repo(root).unwrap_err();
    e.downcast_ref::<CliError>().expect("a CliError").clone()
}

fn json_error(args: &[&str]) -> (Value, Option<i32>) {
    let repo = fixture("single");
    let out = stewards(repo.path(), args);
    let v: Value = serde_json::from_slice(&out.stdout).expect("stdout is JSON");
    (v, out.status.code())
}

#[test]
fn an_error_prints_the_error_envelope() {
    // unknown_role is a permanent error; stubs disappear as commands are built.
    let (v, code) = json_error(&["brief", "--role", "bogus"]);
    assert_eq!(code, Some(2));
    assert_eq!(v["stewards"]["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(v["stewards"]["schema"], 1);
    assert_eq!(v["error"]["code"], "unknown_role");
    assert!(v["error"]["message"].is_string());
    assert!(v["error"]["fix"].is_string());
}

#[test]
fn clap_errors_are_usage_errors() {
    for (cmd, arg) in [
        ("route", "<PATHS>"),
        ("stamp", "<UNIT>"),
        ("review", "--pr"),
        ("render", "--harness"),
        ("completions", "<SHELL>"),
    ] {
        let (v, code) = json_error(&[cmd]);
        assert_eq!(code, Some(2), "{cmd}");
        assert_eq!(v["error"]["code"], "usage", "{cmd}");
        assert!(
            v["error"]["message"].as_str().unwrap().contains(arg),
            "{cmd}: {v}"
        );
    }
}

#[test]
fn missing_c_dir_is_a_usage_error() {
    let (v, code) = json_error(&["-C", "/no/such/dir", "check"]);
    assert_eq!(code, Some(2));
    assert_eq!(v["error"]["code"], "usage");
    assert_eq!(v["error"]["fix"], "pass an existing directory");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .starts_with("-C /no/such/dir: ")
    );
}

#[test]
fn human_errors_print_error_and_fix_lines() {
    let repo = fixture("single");
    let out = stewards(repo.path(), &["brief", "--role", "bogus", "--human"]);
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(stdout.lines().any(|l| l.starts_with("error:")), "{stdout}");
    assert!(stdout.lines().any(|l| l.starts_with("fix:")), "{stdout}");
}

#[test]
fn bad_role_is_unknown_role() {
    let (v, code) = json_error(&["pack", "api", "--role", "boss"]);
    assert_eq!(code, Some(2));
    assert_eq!(v["error"]["code"], "unknown_role");
}

#[test]
fn load_repo_outside_git_is_not_a_git_repo() {
    let dir = tempfile::TempDir::new().unwrap();
    assert_eq!(load_err(dir.path()).code, "not_a_git_repo");
}

#[test]
fn load_repo_without_stewards_is_no_stewards_dir() {
    let dir = tempfile::TempDir::new().unwrap();
    git(dir.path(), &["init", "-q"]);
    assert_eq!(load_err(dir.path()).code, "no_stewards_dir");
}

#[test]
fn invalid_toml_names_file_and_line() {
    let repo = fixture("single");
    std::fs::write(
        repo.path().join(".stewards/units/api.toml"),
        "unit = \"api\"\n[[interface]]\nid = = 3\n",
    )
    .unwrap();
    let e = load_err(repo.path());
    assert_eq!(e.code, "invalid_toml");
    assert!(
        e.message.contains(".stewards/units/api.toml:3"),
        "{}",
        e.message
    );
    assert_eq!(e.fix, "fix the file at .stewards/units/api.toml:3");
}

#[test]
fn newer_schema_is_schema_too_new() {
    let repo = fixture("single");
    std::fs::write(
        repo.path().join(".stewards/repo.toml"),
        "schema = 2\nname = \"single\"\n",
    )
    .unwrap();
    assert_eq!(load_err(repo.path()).code, "schema_too_new");
}

#[test]
fn bad_unit_name_is_invalid_toml() {
    let repo = fixture("single");
    std::fs::write(
        repo.path().join(".stewards/units.toml"),
        "[[unit]]\nname = \"api\"\npaths = []\n\n[[unit]]\nname = \"Bad.Name\"\npaths = []\n",
    )
    .unwrap();
    let e = load_err(repo.path());
    assert_eq!(e.code, "invalid_toml");
    assert!(
        e.message.contains(".stewards/units.toml:6"),
        "{}",
        e.message
    );
}

#[test]
fn anchor_parse_splits_at_the_first_double_colon() {
    let a = Anchor::parse("a.rs::m::f").unwrap();
    assert_eq!((a.path.as_str(), a.name.as_str()), ("a.rs", "m::f"));
}

#[test]
fn single_fixture_loads() {
    let repo = fixture("single");
    let loaded = load_repo(repo.path()).unwrap();
    let names: Vec<_> = loaded.units.iter().map(|u| u.name.as_str()).collect();
    assert_eq!(names, ["api", "store"]);
    assert_eq!(loaded.cards.len(), 2);
    assert_eq!(loaded.tests.len(), 2);
    assert_eq!(loaded.repo.flows[0].units, ["api", "store"]);
    assert_eq!(loaded.repo.budgets.developer, 20000);
    assert_eq!(loaded.cards["store"].interface.len(), 2);
}

#[test]
fn optional_schema_shapes_load() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();
    git(root, &["init", "-q"]);
    std::fs::create_dir_all(root.join(".stewards/units")).unwrap();
    std::fs::write(
        root.join(".stewards/repo.toml"),
        r#"schema = 1
name = "shapes"
[commands]
test = ["pytest", { cmd = "cargo test", timeout_seconds = 30 }]
[[flow]]
name = "f"
units = ["a"]
entry = "a.py::f"
check = "pytest -k f"
[tracker]
kind = "gitlab"
project = "group/project"
"#,
    )
    .unwrap();
    std::fs::write(
        root.join(".stewards/units.toml"),
        "[[unit]]\nname = \"a\"\npaths = [\"a.py\"]\n",
    )
    .unwrap();
    std::fs::write(
        root.join(".stewards/units/a.toml"),
        r#"unit = "a"
[reviewed]
commit = "c1"
fingerprint = "sha256:f1"
tree = "t1"
tester = "t2"
documenter = "t3"
[[gap]]
kind = "bug"
title = "count is off"
repro = "pytest -k count"
severity = "high"
issue = "https://example.invalid/1"
"#,
    )
    .unwrap();

    let loaded = load_repo(root).unwrap();
    let repo = &loaded.repo;
    assert!(matches!(&repo.commands.test[..], [
        CommandEntry::Plain(p),
        CommandEntry::Timed { cmd, timeout_seconds: 30 },
    ] if p == "pytest" && cmd == "cargo test"));
    assert_eq!(repo.flows[0].check.as_deref(), Some("pytest -k f"));
    let tracker = repo.tracker.as_ref().unwrap();
    assert_eq!(
        (tracker.kind, tracker.project.as_str()),
        (TrackerKind::Gitlab, "group/project")
    );

    let card = &loaded.cards["a"];
    let r = card.reviewed.as_ref().unwrap();
    assert_eq!(
        (r.commit.as_str(), r.fingerprint.as_str(), r.tree.as_str()),
        ("c1", "sha256:f1", "t1")
    );
    assert_eq!(
        (r.tester.as_deref(), r.documenter.as_deref()),
        (Some("t2"), Some("t3"))
    );
    let gap = &card.gaps[0];
    assert_eq!(gap.kind, GapKind::Bug);
    assert_eq!(gap.repro.as_deref(), Some("pytest -k count"));
    assert_eq!(gap.severity.as_deref(), Some("high"));
    assert_eq!(gap.issue.as_deref(), Some("https://example.invalid/1"));
}

#[test]
fn every_command_parses_its_flags() {
    let repo = fixture("single");
    let dir = repo.path().to_str().unwrap();
    let commands: &[&[&str]] = &[
        &[
            "guide",
            "--harness",
            "pi",
            "--all",
            "--errors",
            "--glossary",
        ],
        &["init", "--draft", "--hooks", "--ci"],
        &["status", "--telemetry"],
        &["route", "src/api/items.py", "Makefile"],
        &[
            "pack",
            "api",
            "--role",
            "maintainer",
            "--diff",
            "--task",
            "t",
        ],
        &["pack", "--role", "architect"],
        &[
            "brief", "api", "--role", "tester", "--task", "t", "--stdout",
        ],
        &["brief", "--role", "architect"],
        &["submit", "api", "--role", "developer", "--file", "a.json"],
        &["submit", "--role", "architect", "--file", "a.json"],
        &["stamp", "api"],
        &["next", "--task", "t", "--no-brief", "--no-run", "--pr", "7"],
        &["check"],
        &["validate", "--report", "bug.json", "--export"],
        &["review", "--pr", "7"],
        &["debt"],
        &["docs", "api"],
        &["render", "--harness", "pi", "--repo-level"],
        &[
            "eval",
            "--harness",
            "pi",
            "--arm",
            "full",
            "--task",
            "1",
            "--max-cost",
            "2.5",
        ],
        &["completions", "bash"],
        &["--repo", "single", "--direct", "-C", dir, "check"],
    ];
    for args in commands {
        let out = stewards(Path::new("/"), args);
        let v: Value = serde_json::from_slice(&out.stdout).expect("stdout is JSON");
        // Built commands answer for real (run from `/` they mostly error); the point is that clap accepted the flags.
        assert_ne!(v["error"]["code"], "usage", "{args:?}: {v}");
    }
}

// Writes and reads under a repo never pass through a symlink.
#![cfg(unix)]
#[allow(dead_code)] // each test crate uses a subset of the helpers
mod common;

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::{fixture, run};
use stewards::model::load_repo;
use stewards::output::CliError;
use stewards::workspace::Repo;
use tempfile::TempDir;

const SECRET: &[u8] = b"ssh-ed25519 AAAA user@host\n";

fn code_of<T>(r: anyhow::Result<T>) -> &'static str {
    r.err()
        .expect("an error")
        .downcast_ref::<CliError>()
        .expect("a CliError")
        .code
}

/// A repo with no `.stewards/`, rooted at `<tmp>/repo`, so `<tmp>` holds what is outside it.
fn bare_repo(tmp: &TempDir) -> Repo {
    let root = tmp.path().join("repo");
    fs::create_dir(&root).unwrap();
    Repo {
        name: "repo".into(),
        root,
        loaded: None,
    }
}

/// Moves `.stewards/<rel>` of `repo` to `outside/<rel>` and links the old path to it.
fn move_out_and_link(repo: &Path, outside: &Path, rel: &str) {
    let from = repo.join(".stewards").join(rel);
    let to = outside.join(rel);
    fs::create_dir_all(to.parent().unwrap()).unwrap();
    fs::rename(&from, &to).unwrap();
    symlink(&to, &from).unwrap();
}

/// Every file under `dir` with its bytes, sorted by path.
fn snapshot_dir(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            out.extend(snapshot_dir(&p));
        } else {
            out.push((p.display().to_string(), fs::read(&p).unwrap()));
        }
    }
    out.sort();
    out
}

fn assert_stamp_is_unsafe_path(repo: &Path, outside: &Path) {
    let before = snapshot_dir(outside);
    let (code, v) = run(repo, &["stamp", "api"]);
    assert_eq!(
        (code, &v["error"]["code"]),
        (2, &"unsafe_path".into()),
        "{v}"
    );
    let fix = v["error"]["fix"].as_str().unwrap();
    assert!(fix.contains("stewards never writes through links"), "{fix}");
    assert_eq!(snapshot_dir(outside), before);
}

#[test]
fn stamp_through_a_symlinked_card_is_unsafe_path() {
    let repo = fixture("single");
    let outside = TempDir::new().unwrap();
    move_out_and_link(repo.path(), outside.path(), "units/api.toml");
    assert_stamp_is_unsafe_path(repo.path(), outside.path());
}

#[test]
fn stamp_through_a_symlinked_units_dir_is_unsafe_path() {
    let repo = fixture("single");
    let outside = TempDir::new().unwrap();
    move_out_and_link(repo.path(), outside.path(), "units");
    assert_stamp_is_unsafe_path(repo.path(), outside.path());
}

#[test]
fn stamp_through_a_symlinked_stewards_dir_is_unsafe_path() {
    let repo = fixture("single");
    let outside = TempDir::new().unwrap();
    let real = outside.path().join("stewards");
    fs::rename(repo.path().join(".stewards"), &real).unwrap();
    symlink(&real, repo.path().join(".stewards")).unwrap();
    assert_stamp_is_unsafe_path(repo.path(), outside.path());
}

#[test]
fn write_through_a_symlinked_file_or_directory_is_refused() {
    let tmp = TempDir::new().unwrap();
    let repo = bare_repo(&tmp);
    let target = tmp.path().join("authorized_keys");
    fs::write(&target, SECRET).unwrap();
    symlink(&target, repo.root.join("card.toml")).unwrap();
    symlink(tmp.path(), repo.root.join("dir")).unwrap();

    let e = repo.write("card.toml", b"pwned").unwrap_err();
    let e = e.downcast_ref::<CliError>().unwrap();
    assert_eq!(e.code, "unsafe_path");
    assert!(e.message.contains("card.toml"), "{}", e.message);
    assert_eq!(
        code_of(repo.write("dir/authorized_keys", b"pwned")),
        "unsafe_path"
    );
    assert_eq!(fs::read(&target).unwrap(), SECRET);
    assert!(
        fs::symlink_metadata(repo.root.join("card.toml"))
            .unwrap()
            .is_symlink()
    );
}

#[test]
fn write_rejects_paths_that_are_not_plain_names() {
    let tmp = TempDir::new().unwrap();
    let repo = bare_repo(&tmp);
    let abs = tmp.path().join("abs").display().to_string();
    for rel in ["../x", abs.as_str(), "a/./b", "", "a//b"] {
        assert_eq!(code_of(repo.write(rel, b"x")), "unsafe_path", "{rel}");
    }
    assert!(!tmp.path().join("x").exists() && !tmp.path().join("abs").exists());
    assert!(!repo.root.join("a").exists());
}

#[test]
fn write_creates_missing_parent_directories() {
    let tmp = TempDir::new().unwrap();
    let repo = bare_repo(&tmp);
    repo.write(".stewards/briefs/api.md", b"brief").unwrap();
    assert_eq!(
        fs::read(repo.root.join(".stewards/briefs/api.md")).unwrap(),
        b"brief"
    );
}

#[test]
fn write_over_a_regular_file_replaces_it_and_leaves_no_temporary_file() {
    let tmp = TempDir::new().unwrap();
    let repo = bare_repo(&tmp);
    fs::write(repo.root.join("card.toml"), "old and longer").unwrap();
    repo.write("card.toml", b"new").unwrap();
    assert_eq!(fs::read(repo.root.join("card.toml")).unwrap(), b"new");
    assert_eq!(fs::read_dir(&repo.root).unwrap().count(), 1);
}

#[test]
fn append_extends_an_existing_file_and_creates_a_new_one() {
    let tmp = TempDir::new().unwrap();
    let repo = bare_repo(&tmp);
    fs::write(repo.root.join("log"), "a\n").unwrap();
    repo.append("log", b"b\n").unwrap();
    repo.append("t/new.jsonl", b"c\n").unwrap();
    assert_eq!(fs::read(repo.root.join("log")).unwrap(), b"a\nb\n");
    assert_eq!(fs::read(repo.root.join("t/new.jsonl")).unwrap(), b"c\n");
}

#[test]
fn append_through_a_symlink_is_refused() {
    let tmp = TempDir::new().unwrap();
    let repo = bare_repo(&tmp);
    let target = tmp.path().join("authorized_keys");
    fs::write(&target, SECRET).unwrap();
    symlink(&target, repo.root.join("log")).unwrap();
    assert_eq!(code_of(repo.append("log", b"pwned")), "unsafe_path");
    assert_eq!(fs::read(&target).unwrap(), SECRET);
}

#[test]
fn remove_of_a_missing_file_is_ok_and_of_a_symlink_is_refused() {
    let tmp = TempDir::new().unwrap();
    let repo = bare_repo(&tmp);
    repo.remove("gone/brief.md").unwrap();
    let target = tmp.path().join("authorized_keys");
    fs::write(&target, SECRET).unwrap();
    let link = repo.root.join("brief.md");
    symlink(&target, &link).unwrap();
    assert_eq!(code_of(repo.remove("brief.md")), "unsafe_path");
    assert!(fs::symlink_metadata(&link).unwrap().is_symlink());
    assert_eq!(fs::read(&target).unwrap(), SECRET);
}

#[test]
fn load_repo_refuses_a_symlinked_repo_toml() {
    let repo = fixture("single");
    let outside = TempDir::new().unwrap();
    move_out_and_link(repo.path(), outside.path(), "repo.toml");
    let before = snapshot_dir(outside.path());
    assert_eq!(code_of(load_repo(repo.path())), "unsafe_path");
    assert_eq!(snapshot_dir(outside.path()), before);
}

#[test]
fn load_repo_refuses_a_symlinked_card() {
    let repo = fixture("single");
    let outside = TempDir::new().unwrap();
    move_out_and_link(repo.path(), outside.path(), "units/store.toml");
    let before = snapshot_dir(outside.path());
    assert_eq!(code_of(load_repo(repo.path())), "unsafe_path");
    assert_eq!(snapshot_dir(outside.path()), before);
}

#[test]
fn load_repo_refuses_a_symlinked_stewards_dir() {
    let repo = fixture("single");
    let outside = TempDir::new().unwrap();
    let real = outside.path().join("stewards");
    fs::rename(repo.path().join(".stewards"), &real).unwrap();
    symlink(&real, repo.path().join(".stewards")).unwrap();
    let before = snapshot_dir(outside.path());
    assert_eq!(code_of(load_repo(repo.path())), "unsafe_path");
    assert_eq!(snapshot_dir(outside.path()), before);
}

#[test]
fn write_over_a_file_keeps_its_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::TempDir::new().unwrap();
    let repo = Repo {
        name: "r".into(),
        root: dir.path().to_path_buf(),
        loaded: None,
    };
    repo.write("a/card.toml", b"one").unwrap();
    let path = dir.path().join("a/card.toml");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    repo.write("a/card.toml", b"two").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"two");
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}
/// Links `repo/rel` to a new file `outside/name` holding SECRET; returns the target.
fn link_to_secret(repo: &Path, rel: &str, outside: &Path) -> std::path::PathBuf {
    let target = outside.join("secret");
    fs::write(&target, SECRET).unwrap();
    let link = repo.join(rel);
    fs::create_dir_all(link.parent().unwrap()).unwrap();
    symlink(&target, &link).unwrap();
    target
}

fn assert_unsafe(repo: &Path, args: &[&str]) {
    let (code, v) = run(repo, args);
    assert_eq!(
        (code, &v["error"]["code"]),
        (2, &"unsafe_path".into()),
        "{v}"
    );
}

#[test]
fn brief_through_a_symlinked_brief_file_is_unsafe_path() {
    let repo = fixture("single");
    let outside = TempDir::new().unwrap();
    let rel = ".stewards/briefs/api-maintainer.md";
    let target = link_to_secret(repo.path(), rel, outside.path());
    assert_unsafe(repo.path(), &["brief", "api", "--role", "maintainer"]);
    assert_eq!(fs::read(&target).unwrap(), SECRET);
}

#[test]
fn next_through_a_symlinked_brief_file_is_unsafe_path() {
    let repo = fixture("single");
    let outside = TempDir::new().unwrap();
    let rel = ".stewards/briefs/api-maintainer.md";
    let target = link_to_secret(repo.path(), rel, outside.path());
    assert_unsafe(repo.path(), &["next"]);
    assert_eq!(fs::read(&target).unwrap(), SECRET);
}

#[test]
fn brief_through_a_symlinked_briefs_dir_is_unsafe_path() {
    let repo = fixture("single");
    let outside = TempDir::new().unwrap();
    fs::write(outside.path().join("keep"), SECRET).unwrap();
    symlink(outside.path(), repo.path().join(".stewards/briefs")).unwrap();
    let before = snapshot_dir(outside.path());
    assert_unsafe(repo.path(), &["brief", "api", "--role", "maintainer"]);
    assert_unsafe(repo.path(), &["next"]);
    assert_eq!(snapshot_dir(outside.path()), before);
}

#[test]
fn docs_through_a_symlinked_page_is_unsafe_path() {
    let repo = fixture("single");
    let outside = TempDir::new().unwrap();
    let target = link_to_secret(repo.path(), "docs/units/api.md", outside.path());
    assert_unsafe(repo.path(), &["docs", "api"]);
    assert_eq!(fs::read(&target).unwrap(), SECRET);
}

/// A committed repo with no `.stewards/`, at `<tmp>/repo`.
fn uninitialized(tmp: &TempDir) -> std::path::PathBuf {
    let root = tmp.path().join("repo");
    fs::create_dir_all(root.join("src/app")).unwrap();
    fs::write(root.join("src/app/main.py"), "x = 1\n").unwrap();
    common::git(&root, &["init", "-q"]);
    common::git(&root, &["add", "-A"]);
    common::git(
        &root,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t.invalid",
            "commit",
            "-q",
            "--no-gpg-sign",
            "-m",
            "x",
        ],
    );
    root
}

#[test]
fn init_through_a_symlinked_agents_md_is_unsafe_path() {
    let tmp = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let root = uninitialized(&tmp);
    let target = link_to_secret(&root, "AGENTS.md", outside.path());
    assert_unsafe(&root, &["init"]);
    assert_eq!(fs::read(&target).unwrap(), SECRET);
    // refused before the first write, so removing the link is enough to rerun
    assert!(!root.join(".stewards").exists());
    fs::remove_file(root.join("AGENTS.md")).unwrap();
    let (code, v) = run(&root, &["init"]);
    assert_eq!(code, 0, "{v}");
}

#[test]
fn init_through_a_symlinked_gitignore_is_unsafe_path() {
    let tmp = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let root = uninitialized(&tmp);
    let target = link_to_secret(&root, ".gitignore", outside.path());
    assert_unsafe(&root, &["init"]);
    assert_eq!(fs::read(&target).unwrap(), SECRET);
}

const TELEMETRY_LINE: &str = "{\"role\":\"maintainer\",\"bytes\":10,\"pulled_anchors\":[]}\n";

#[test]
fn status_telemetry_reads_only_regular_files() {
    let repo = fixture("single");
    let outside = TempDir::new().unwrap();
    let real = outside.path().join("real.jsonl");
    fs::write(&real, TELEMETRY_LINE).unwrap();
    let dir = repo.path().join(".stewards/telemetry");
    fs::create_dir_all(dir.join("sub.jsonl")).unwrap();
    symlink(&real, dir.join("link.jsonl")).unwrap();
    let (_, v) = run(repo.path(), &["status", "--telemetry"]);
    assert!(v["telemetry"].is_null(), "{v}");
    fs::write(dir.join("2026-11.jsonl"), TELEMETRY_LINE).unwrap();
    let (_, v) = run(repo.path(), &["status", "--telemetry"]);
    assert_eq!(v["telemetry"]["submits"]["maintainer"], 1, "{v}");
}

#[test]
fn status_telemetry_does_not_follow_a_symlinked_dir() {
    let repo = fixture("single");
    let outside = TempDir::new().unwrap();
    fs::write(outside.path().join("2026-11.jsonl"), TELEMETRY_LINE).unwrap();
    symlink(outside.path(), repo.path().join(".stewards/telemetry")).unwrap();
    let (_, v) = run(repo.path(), &["status", "--telemetry"]);
    assert!(v["telemetry"].is_null(), "{v}");
}

/// A committed copy of the single fixture at `<tmp>/<name>`.
fn single_at(tmp: &TempDir, name: &str) -> PathBuf {
    let root = tmp.path().join(name);
    fs::create_dir(&root).unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/single");
    common::copy_dir(&fixture, &root);
    common::git(&root, &["init", "-q"]);
    common::git(&root, &["add", "-A"]);
    common::git(
        &root,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t.invalid",
            "commit",
            "-q",
            "--no-gpg-sign",
            "-m",
            "x",
        ],
    );
    root
}

/// The command of the brief's `Run` code span, whatever the length of its fence.
fn printed_command(brief: &str) -> String {
    let rest = &brief[brief.rfind(". Run `").unwrap() + ". Run ".len()..];
    let fence = "`".repeat(rest.chars().take_while(|&c| c == '`').count());
    let body = &rest[fence.len()..];
    let body = &body[..body.find(&fence).unwrap()];
    let unpadded = body.strip_prefix(' ').and_then(|b| b.strip_suffix(' '));
    unpadded.unwrap_or(body).to_string()
}

/// Writes the api explain brief in a repo at `<tmp>/<name>`, then runs the brief's submit command
/// through `sh -c` from `<tmp>`: submit succeeds against that repo and no `pwned` file appears.
fn printed_submit_runs_in(name: &str) {
    let tmp = TempDir::new().unwrap();
    let root = single_at(&tmp, name);
    let (code, v) = run(
        &root,
        &["brief", "api", "--role", "maintainer", "--task", "q"],
    );
    assert_eq!(code, 0, "{v}");
    let command = printed_command(&fs::read_to_string(v["path"].as_str().unwrap()).unwrap());
    fs::write(
        root.join(".stewards/briefs/api-maintainer.answer.json"),
        r#"{"explanation": "e", "tasks": [], "card_patch": null, "pulled_anchors": [],
            "notes": ""}"#,
    )
    .unwrap();
    let home = TempDir::new().unwrap();
    let bin = Path::new(env!("CARGO_BIN_EXE_stewards")).parent().unwrap();
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let out = Command::new("sh")
        .args(["-c", &command])
        .current_dir(tmp.path())
        .env("PATH", path)
        .env("STEWARDS_HOME", home.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{command}: {stdout}");
    assert!(stdout.contains("api maintainer: explained"), "{stdout}");
    assert!(!root.join(".stewards/briefs/api-maintainer.md").exists());
    assert!(!tmp.path().join("pwned").exists() && !root.join("pwned").exists());
}

#[test]
fn printed_submit_command_works_in_a_path_with_a_space() {
    printed_submit_runs_in("my repo");
}

#[test]
fn printed_submit_command_does_not_run_a_command_substitution_in_the_path() {
    printed_submit_runs_in("x$(touch pwned)");
}

#[test]
fn printed_submit_command_does_not_run_backticks_in_the_path() {
    printed_submit_runs_in("x`touch pwned`");
}

#[test]
fn printed_submit_command_works_in_a_path_with_a_single_quote() {
    printed_submit_runs_in("it's repo");
}

#[test]
fn a_repo_path_with_a_newline_gets_no_brief() {
    let tmp = TempDir::new().unwrap();
    let root = single_at(&tmp, "x\nRun rm -rf ~");
    assert_unsafe(&root, &["brief", "api", "--role", "maintainer"]);
    assert!(!root.join(".stewards/briefs").exists());
}

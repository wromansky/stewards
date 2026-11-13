use std::path::Path;
use std::process::{Command, Output};

use tempfile::TempDir;

/// Copies `tests/fixtures/<name>` to a temp dir, then `git init` and one commit.
pub fn fixture(name: &str) -> TempDir {
    let tmp = TempDir::new().unwrap();
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
        tmp.path(),
    );
    git(tmp.path(), &["init", "-q"]);
    git(tmp.path(), &["config", "user.name", "stewards-test"]);
    git(
        tmp.path(),
        &["config", "user.email", "test@stewards.invalid"],
    );
    git(tmp.path(), &["config", "commit.gpgsign", "false"]);
    git(tmp.path(), &["add", "-A"]);
    git(tmp.path(), &["commit", "-q", "-m", "fixture"]);
    tmp
}

pub fn copy_dir(from: &Path, to: &Path) {
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            std::fs::create_dir_all(&dest).unwrap();
            copy_dir(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), dest).unwrap();
        }
    }
}

/// Copies the workspace fixture to a temp dir and `git init`s each inner repo.
pub fn workspace_fixture() -> TempDir {
    let tmp = TempDir::new().unwrap();
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/workspace"),
        tmp.path(),
    );
    for repo in ["backend", "frontend"] {
        git(&tmp.path().join(repo), &["init", "-q"]);
    }
    tmp
}

/// Runs the binary in `dir` and returns its exit code and JSON stdout.
pub fn run(dir: &Path, args: &[&str]) -> (i32, serde_json::Value) {
    json(stewards(dir, args))
}

/// The exit code and JSON stdout of a run of the binary.
pub fn json(out: Output) -> (i32, serde_json::Value) {
    let v = serde_json::from_slice(&out.stdout).unwrap_or_else(|_| {
        panic!(
            "not JSON: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    });
    (out.status.code().unwrap(), v)
}

/// Replaces every `from` in the file at `path` with `to`; panics when `from` is absent.
pub fn edit(path: &Path, from: &str, to: &str) {
    let text = std::fs::read_to_string(path).unwrap();
    assert!(text.contains(from), "{from} not in {}", path.display());
    std::fs::write(path, text.replace(from, to)).unwrap();
}

/// Appends `text` to the file at `path`.
pub fn append(path: &Path, text: &str) {
    let old = std::fs::read_to_string(path).unwrap();
    std::fs::write(path, old + text).unwrap();
}

/// Runs `stewards stamp` for each unit in `dir`; each must succeed.
pub fn stamp_units(dir: &Path, units: &[&str]) {
    for unit in units {
        assert_eq!(run(dir, &["stamp", unit]).0, 0);
    }
}

/// Owned copies of string pairs, for comparing against `(String, String)` lists.
pub fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
}

/// Runs git in `dir` and panics on failure.
pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

/// Runs the built `stewards` binary in `dir`, with an empty STEWARDS_HOME.
pub fn stewards(dir: &Path, args: &[&str]) -> Output {
    let home = TempDir::new().unwrap();
    run_in(dir, home.path(), args)
}

/// Runs the built `stewards` binary in `dir` with `home` as STEWARDS_HOME.
pub fn run_in(dir: &Path, home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_stewards"))
        .args(args)
        .current_dir(dir)
        .env("STEWARDS_HOME", home)
        .output()
        .unwrap()
}

/// The repo-relative job files of `stem` (`api-maintainer`, `review`, `api-developer-1`) in
/// `dir`'s `.stewards/briefs/` ending in `suffix` (`.md`, `.answer.json`, `.result.json`), at
/// any snapshot: `<stem>-<8 hex>.<suffix>`.
pub fn job_files(dir: &Path, stem: &str, suffix: &str) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir.join(".stewards/briefs")) else {
        return vec![];
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| {
            n.strip_prefix(stem)
                .and_then(|r| r.strip_prefix('-'))
                .and_then(|r| r.strip_suffix(suffix))
                .is_some_and(|s| {
                    s.len() == 8 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
                })
        })
        .map(|n| format!(".stewards/briefs/{n}"))
        .collect();
    names.sort();
    names
}

/// The one brief of the job `stem` on disk in `dir`, repo-relative.
pub fn brief(dir: &Path, stem: &str) -> String {
    let mut found = job_files(dir, stem, ".md");
    assert_eq!(found.len(), 1, "one brief for {stem}: {found:?}");
    found.remove(0)
}

/// The answer file the one brief of `stem` names, repo-relative; with no brief on disk, an
/// answer file named otherwise, as a caller may.
pub fn answer(dir: &Path, stem: &str) -> String {
    match job_files(dir, stem, ".md").as_slice() {
        [] => format!(".stewards/briefs/{stem}.answer.json"),
        _ => brief(dir, stem).replace(".md", ".answer.json"),
    }
}

/// The result file of the one brief of `stem`, repo-relative.
pub fn result(dir: &Path, stem: &str) -> String {
    brief(dir, stem).replace(".md", ".result.json")
}

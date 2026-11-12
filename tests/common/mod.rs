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
    let out = stewards(dir, args);
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

/// Runs the built `stewards` binary in `dir`.
pub fn stewards(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_stewards"))
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

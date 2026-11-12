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

fn copy_dir(from: &Path, to: &Path) {
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

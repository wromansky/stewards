#[allow(dead_code)] // shared helpers; this file uses only `git`
mod common;

use std::path::Path;

use common::git as run_git;
use stewards::git;
use tempfile::TempDir;

fn repo() -> TempDir {
    let tmp = TempDir::new().unwrap();
    run_git(tmp.path(), &["init", "-q"]);
    run_git(tmp.path(), &["config", "user.name", "stewards-test"]);
    run_git(
        tmp.path(),
        &["config", "user.email", "test@stewards.invalid"],
    );
    run_git(tmp.path(), &["config", "commit.gpgsign", "false"]);
    tmp
}

fn write(root: &Path, name: &str, text: &str) {
    std::fs::write(root.join(name), text).unwrap();
}

fn paths(tree: &[(String, String)]) -> Vec<&str> {
    tree.iter().map(|(p, _)| p.as_str()).collect()
}

#[test]
fn snapshot_has_modified_tracked_and_untracked_but_not_ignored() {
    let r = repo();
    let p = r.path();
    write(p, ".gitignore", "ignored.txt\n");
    write(p, "tracked.txt", "one\n");
    run_git(p, &["add", "-A"]);
    run_git(p, &["commit", "-q", "-m", "init"]);
    write(p, "tracked.txt", "two\n");
    write(p, "new.txt", "new\n");
    write(p, "ignored.txt", "x\n");
    let tree = git::snapshot(p).unwrap();
    let entries = git::ls_tree(p, &tree).unwrap();
    assert_eq!(paths(&entries), [".gitignore", "new.txt", "tracked.txt"]);
    let blob = &entries[2].1;
    assert_eq!(run_git(p, &["cat-file", "-p", blob]), "two\n");
}

#[test]
fn snapshot_leaves_git_status_byte_identical() {
    let r = repo();
    let p = r.path();
    write(p, "a.txt", "a\n");
    run_git(p, &["add", "a.txt"]);
    run_git(p, &["commit", "-q", "-m", "init"]);
    write(p, "a.txt", "changed\n");
    write(p, "b.txt", "b\n");
    let before = run_git(p, &["status", "--porcelain"]);
    git::snapshot(p).unwrap();
    assert_eq!(run_git(p, &["status", "--porcelain"]), before);
}

#[test]
fn snapshot_works_with_no_commits() {
    let r = repo();
    write(r.path(), "a.txt", "a\n");
    let tree = git::snapshot(r.path()).unwrap();
    assert_eq!(paths(&git::ls_tree(r.path(), &tree).unwrap()), ["a.txt"]);
}

#[test]
fn snapshot_works_in_a_linked_worktree() {
    let r = repo();
    let p = r.path();
    write(p, "a.txt", "a\n");
    run_git(p, &["add", "a.txt"]);
    run_git(p, &["commit", "-q", "-m", "init"]);
    let linked = TempDir::new().unwrap();
    let dir = linked.path().join("wt");
    run_git(
        p,
        &[
            "worktree",
            "add",
            "-q",
            dir.to_str().unwrap(),
            "-b",
            "other",
        ],
    );
    write(&dir, "b.txt", "b\n");
    let tree = git::snapshot(&dir).unwrap();
    assert_eq!(
        paths(&git::ls_tree(&dir, &tree).unwrap()),
        ["a.txt", "b.txt"]
    );
}

#[test]
fn snapshot_error_carries_git_stderr() {
    let dir = TempDir::new().unwrap();
    let e = git::snapshot(dir.path()).unwrap_err();
    assert!(e.to_string().contains("not a git repository"), "{e}");
}

#[test]
fn two_snapshots_of_an_unchanged_tree_are_equal() {
    let r = repo();
    write(r.path(), "a.txt", "a\n");
    assert_eq!(
        git::snapshot(r.path()).unwrap(),
        git::snapshot(r.path()).unwrap()
    );
}

#[test]
fn has_object_is_false_for_a_bogus_sha() {
    let r = repo();
    assert!(!git::has_object(r.path(), &"0".repeat(40)));
}

#[test]
fn has_object_is_true_for_a_snapshot_tree() {
    let r = repo();
    write(r.path(), "a.txt", "a\n");
    let tree = git::snapshot(r.path()).unwrap();
    assert!(git::has_object(r.path(), &tree));
}

#[test]
fn is_repo_tells_repos_from_plain_dirs() {
    assert!(git::is_repo(repo().path()));
    assert!(!git::is_repo(TempDir::new().unwrap().path()));
}

#[test]
fn ls_files_lists_tracked_files_only() {
    let r = repo();
    write(r.path(), "a.txt", "a\n");
    write(r.path(), "b.txt", "b\n");
    run_git(r.path(), &["add", "a.txt"]);
    assert_eq!(git::ls_files(r.path()).unwrap(), ["a.txt"]);
}

#[test]
fn diff_names_between_two_snapshots() {
    let r = repo();
    let p = r.path();
    write(p, "keep.txt", "k\n");
    write(p, "edit.txt", "e\n");
    write(p, "gone.txt", "g\n");
    let from = git::snapshot(p).unwrap();
    write(p, "edit.txt", "e2\n");
    write(p, "added.txt", "a\n");
    std::fs::remove_file(p.join("gone.txt")).unwrap();
    let to = git::snapshot(p).unwrap();
    assert_eq!(
        git::diff_names(p, &from, &to).unwrap(),
        ["added.txt", "edit.txt", "gone.txt"]
    );
}

#[test]
fn diff_file_shows_the_change_with_context() {
    let r = repo();
    let p = r.path();
    write(p, "a.txt", "1\n2\n3\n4\n5\n");
    let from = git::snapshot(p).unwrap();
    write(p, "a.txt", "1\n2\nthree\n4\n5\n");
    let to = git::snapshot(p).unwrap();
    let d = git::diff_file(p, &from, &to, "a.txt").unwrap();
    assert!(d.contains("-3\n+three\n"), "{d}");
    assert!(d.contains(" 2\n"), "{d}");
}

#[test]
fn diff_added_lines_gives_new_file_line_numbers() {
    let r = repo();
    let p = r.path();
    write(p, "a.txt", "1\n2\n3\n4\n5\n");
    write(p, "gone.txt", "x\n");
    let from = git::snapshot(p).unwrap();
    write(p, "a.txt", "1\nTWO\n2b\n3\n5\n6\n");
    write(p, "b.txt", "++ plus\nlast\n");
    std::fs::remove_file(p.join("gone.txt")).unwrap();
    let to = git::snapshot(p).unwrap();
    let got = git::diff_added_lines(p, &from, &to).unwrap();
    let want: Vec<(String, u32, String)> = [
        ("a.txt", 2, "TWO"),
        ("a.txt", 3, "2b"),
        ("a.txt", 6, "6"),
        ("b.txt", 1, "++ plus"),
        ("b.txt", 2, "last"),
    ]
    .iter()
    .map(|(f, n, t)| (f.to_string(), *n, t.to_string()))
    .collect();
    assert_eq!(got, want);
}

#[test]
fn head_is_the_commit_sha() {
    let r = repo();
    write(r.path(), "a.txt", "a\n");
    run_git(r.path(), &["add", "a.txt"]);
    run_git(r.path(), &["commit", "-q", "-m", "init"]);
    assert_eq!(
        git::head(r.path()).unwrap(),
        run_git(r.path(), &["rev-parse", "HEAD"]).trim()
    );
}

#[test]
fn task_text_is_branch_then_last_commit_subject() {
    let r = repo();
    let p = r.path();
    run_git(p, &["checkout", "-q", "-b", "feature/x"]);
    write(p, "a.txt", "a\n");
    run_git(p, &["add", "a.txt"]);
    run_git(p, &["commit", "-q", "-m", "add a thing", "-m", "body"]);
    assert_eq!(git::task_text(p).unwrap(), "feature/x\nadd a thing");
}

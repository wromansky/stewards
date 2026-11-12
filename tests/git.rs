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
fn has_object_is_false_for_a_blob() {
    let r = repo();
    write(r.path(), "a.txt", "a\n");
    let tree = git::snapshot(r.path()).unwrap();
    let (_, blob) = &git::ls_tree(r.path(), &tree).unwrap()[0];
    assert!(!git::has_object(r.path(), blob));
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
fn diff_file_takes_a_glob_character_name_literally() {
    let r = repo();
    let p = r.path();
    write(p, "*", "star\n");
    write(p, ".env", "TOKEN=old\n");
    let from = git::snapshot(p).unwrap();
    write(p, "*", "star changed\n");
    write(p, ".env", "TOKEN=leaked\n");
    let to = git::snapshot(p).unwrap();
    let d = git::diff_file(p, &from, &to, "*").unwrap();
    assert!(d.contains("+star changed"), "{d}");
    assert!(!d.contains("TOKEN"), "{d}");
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
fn diff_hunks_gives_new_side_ranges_and_pure_deletions_as_count_zero() {
    let r = repo();
    let p = r.path();
    write(p, "a.txt", "1\n2\n3\n4\n5\n");
    write(p, "gone.txt", "x\n");
    let from = git::snapshot(p).unwrap();
    write(p, "a.txt", "1\nTWO\n2b\n3\n5\n6\n");
    write(p, "b.txt", "++ plus\nlast\n");
    std::fs::remove_file(p.join("gone.txt")).unwrap();
    let to = git::snapshot(p).unwrap();
    let got = git::diff_hunks(p, &from, &to).unwrap();
    // `4` deleted after new line 4; the deleted file has no new side
    let want: Vec<(String, u32, u32)> = [
        ("a.txt", 2, 2),
        ("a.txt", 4, 0),
        ("a.txt", 6, 1),
        ("b.txt", 1, 2),
    ]
    .iter()
    .map(|(f, s, c)| (f.to_string(), *s, *c))
    .collect();
    assert_eq!(got, want);
}

#[test]
fn blob_text_reads_a_snapshot_file() {
    let r = repo();
    let p = r.path();
    write(p, "a.txt", "hello\n");
    let tree = git::snapshot(p).unwrap();
    let sha = &git::ls_tree(p, &tree).unwrap()[0].1;
    assert_eq!(git::blob_text(p, sha).unwrap(), "hello\n");
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

#[test]
fn leading_dash_revs_are_errors_and_write_nothing() {
    let r = repo();
    let p = r.path();
    write(p, "a.txt", "a\n");
    let tree = git::snapshot(p).unwrap();
    let out = TempDir::new().unwrap().path().join("pwned");
    let evil = format!("--output={}", out.display());
    assert!(git::ls_tree(p, &evil).is_err());
    assert!(git::diff_names(p, &evil, &tree).is_err());
    assert!(git::diff_names(p, &tree, &evil).is_err());
    assert!(git::diff_file(p, &evil, &tree, "a.txt").is_err());
    assert!(git::diff_file(p, &tree, &evil, "a.txt").is_err());
    assert!(git::diff_added_lines(p, &evil, &tree).is_err());
    assert!(git::diff_added_lines(p, &tree, &evil).is_err());
    assert!(!out.exists());
}

#[test]
fn diff_added_lines_handles_a_path_with_a_space() {
    let r = repo();
    let p = r.path();
    let from = git::snapshot(p).unwrap();
    write(p, "sp ace.txt", "hi\n");
    let to = git::snapshot(p).unwrap();
    assert_eq!(
        git::diff_added_lines(p, &from, &to).unwrap(),
        [("sp ace.txt".to_string(), 1, "hi".to_string())]
    );
}

#[test]
fn diff_added_lines_skips_a_c_quoted_path_and_keeps_the_rest() {
    let r = repo();
    let p = r.path();
    let from = git::snapshot(p).unwrap();
    write(p, "a.txt", "first\n");
    write(p, "q\"uote.txt", "hi\n");
    write(p, "z.txt", "last\n");
    let to = git::snapshot(p).unwrap();
    assert_eq!(
        git::diff_added_lines(p, &from, &to).unwrap(),
        [
            ("a.txt".to_string(), 1, "first".to_string()),
            ("z.txt".to_string(), 1, "last".to_string()),
        ]
    );
}

#[test]
fn task_text_before_the_first_commit_is_the_branch_alone() {
    let r = repo();
    run_git(r.path(), &["checkout", "-q", "-b", "fresh"]);
    assert_eq!(git::task_text(r.path()).unwrap(), "fresh");
}

#[test]
fn task_text_on_a_detached_head_names_head() {
    let r = repo();
    let p = r.path();
    write(p, "a.txt", "a\n");
    run_git(p, &["add", "a.txt"]);
    run_git(p, &["commit", "-q", "-m", "init"]);
    run_git(p, &["checkout", "-q", "--detach"]);
    assert_eq!(git::task_text(p).unwrap(), "HEAD\ninit");
}

#[test]
fn task_text_outside_a_repo_is_an_error() {
    assert!(git::task_text(TempDir::new().unwrap().path()).is_err());
}

const TEMP_INDEX_CHILD: &str = "STEWARDS_TEMP_INDEX_CHILD";

/// Runs `snapshot_refuses_a_symlink_at_its_temp_index_path_child` alone in a fresh process, so
/// its first temp index name is `stewards-index-<child pid>-0` under a private TMPDIR.
#[cfg(unix)]
#[test]
fn snapshot_refuses_a_symlink_at_its_temp_index_path() {
    let tmp = TempDir::new().unwrap();
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "snapshot_refuses_a_symlink_at_its_temp_index_path_child",
            "--ignored",
            "--test-threads=1",
        ])
        .env(TEMP_INDEX_CHILD, "1")
        .env("TMPDIR", tmp.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{stdout}");
    assert!(stdout.contains("1 passed"), "{stdout}");
}

#[cfg(unix)]
#[test]
#[ignore = "run by snapshot_refuses_a_symlink_at_its_temp_index_path"]
fn snapshot_refuses_a_symlink_at_its_temp_index_path_child() {
    if std::env::var_os(TEMP_INDEX_CHILD).is_none() {
        return;
    }
    let r = repo();
    write(r.path(), "a.txt", "a\n");
    run_git(r.path(), &["add", "a.txt"]);
    let tmp = std::env::temp_dir();
    let victim = tmp.join("victim");
    std::fs::write(&victim, "precious").unwrap();
    let planted = tmp.join(format!("stewards-index-{}-0", std::process::id()));
    std::os::unix::fs::symlink(&victim, &planted).unwrap();
    assert!(git::snapshot(r.path()).is_err());
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "precious");
    assert!(planted.is_symlink());
}

#[test]
fn snapshot_sees_a_same_size_edit_made_in_the_same_second_as_the_last_add() {
    // Regression: the copied index looked newer than every file, so git trusted the stat data
    // and the snapshot kept the old blob. Everything here shares one mtime, as a fast edit does.
    let r = repo();
    let p = r.path();
    run_git(p, &["config", "core.trustctime", "false"]);
    let then = std::time::SystemTime::now() - std::time::Duration::from_secs(10);
    let set_mtime = |path: &Path| {
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(then)
            .unwrap()
    };
    write(p, "tracked.txt", "one\n");
    set_mtime(&p.join("tracked.txt"));
    run_git(p, &["add", "-A"]);
    run_git(p, &["commit", "-q", "-m", "init"]);
    set_mtime(&p.join(".git/index"));
    write(p, "tracked.txt", "two\n");
    set_mtime(&p.join("tracked.txt"));
    let tree = git::snapshot(p).unwrap();
    let entries = git::ls_tree(p, &tree).unwrap();
    assert_eq!(run_git(p, &["cat-file", "-p", &entries[0].1]), "two\n");
}

#[test]
fn recent_commit_files_lists_files_per_commit_newest_first() {
    let r = repo();
    let p = r.path();
    assert!(git::recent_commit_files(p, 10).unwrap().is_empty());
    write(p, "a.txt", "a\n");
    write(p, "b.txt", "b\n");
    run_git(p, &["add", "-A"]);
    run_git(p, &["commit", "-q", "-m", "one"]);
    write(p, "b.txt", "b2\n");
    run_git(p, &["commit", "-q", "-am", "two"]);
    assert_eq!(
        git::recent_commit_files(p, 10).unwrap(),
        [
            vec!["b.txt".to_string()],
            vec!["a.txt".into(), "b.txt".into()]
        ]
    );
    assert_eq!(git::recent_commit_files(p, 1).unwrap().len(), 1);
}

#[test]
fn origin_url_is_none_without_an_origin_remote() {
    let r = repo();
    assert_eq!(git::origin_url(r.path()), None);
    run_git(r.path(), &["remote", "add", "upstream", "https://h/u.git"]);
    assert_eq!(git::origin_url(r.path()), None);
}

#[test]
fn origin_url_is_the_origin_remote_url() {
    let r = repo();
    run_git(
        r.path(),
        &["remote", "add", "origin", "https://h/o/name.git"],
    );
    assert_eq!(
        git::origin_url(r.path()).as_deref(),
        Some("https://h/o/name.git")
    );
}

#[test]
#[ignore = "bug: git prints the remote name for an empty url, so origin_url returns Some(\"origin\")"]
fn origin_url_is_none_when_the_url_is_empty() {
    let r = repo();
    run_git(r.path(), &["config", "remote.origin.url", ""]);
    assert_eq!(git::origin_url(r.path()), None);
}

#[test]
fn origin_url_is_none_outside_a_repo() {
    assert_eq!(git::origin_url(TempDir::new().unwrap().path()), None);
}

use std::path::Path;

use stewards::anchors::{Resolution, resolve, source_window};
use stewards::model::Anchor;

fn anchor(path: &str, name: &str) -> Anchor {
    Anchor {
        path: path.into(),
        name: name.into(),
    }
}

fn repo_with(file: &str, content: &[u8]) -> tempfile::TempDir {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(tmp.path().join(file), content).unwrap();
    tmp
}

fn window(root: &Path, name: &str, before: usize, after: usize) -> Option<String> {
    source_window(root, &anchor("f.txt", name), before, after)
}

#[test]
fn found_reports_the_one_based_line() {
    let t = repo_with("f.txt", b"one\ntwo\nfn target() {}\n");
    assert_eq!(
        resolve(t.path(), &anchor("f.txt", "target")),
        Resolution::Found { line: 3 }
    );
}

#[test]
fn name_only_inside_a_longer_identifier_is_dead_name() {
    let t = repo_with("f.txt", b"fn target_two() {}\nlet my_target = 1;\n");
    assert_eq!(
        resolve(t.path(), &anchor("f.txt", "target")),
        Resolution::DeadName
    );
}

#[test]
fn matching_is_case_sensitive() {
    let t = repo_with("f.txt", b"fn Target() {}\n");
    assert_eq!(
        resolve(t.path(), &anchor("f.txt", "target")),
        Resolution::DeadName
    );
}

#[test]
fn a_whole_word_after_a_partial_one_is_found() {
    let t = repo_with("f.txt", b"target_two\ntarget\n");
    assert_eq!(
        resolve(t.path(), &anchor("f.txt", "target")),
        Resolution::Found { line: 2 }
    );
}

#[test]
fn missing_file_is_dead_path() {
    let t = repo_with("f.txt", b"x");
    assert_eq!(
        resolve(t.path(), &anchor("nope.txt", "x")),
        Resolution::DeadPath
    );
}

#[test]
fn rust_name_with_double_colon_resolves() {
    let t = repo_with(
        "f.txt",
        b"impl Router {\n    fn x() {}\n}\n// Router::owner here\n",
    );
    assert_eq!(
        resolve(t.path(), &anchor("f.txt", "Router::owner")),
        Resolution::Found { line: 4 }
    );
}

#[test]
fn name_at_file_start_and_end() {
    let t = repo_with("f.txt", b"start\nmiddle\nend");
    assert_eq!(
        resolve(t.path(), &anchor("f.txt", "start")),
        Resolution::Found { line: 1 }
    );
    assert_eq!(
        resolve(t.path(), &anchor("f.txt", "end")),
        Resolution::Found { line: 3 }
    );
}

#[test]
fn window_clamps_at_file_start() {
    let t = repo_with("f.txt", b"a\nb\nc\nd\n");
    assert_eq!(window(t.path(), "a", 5, 1).unwrap(), "a\nb");
}

#[test]
fn window_clamps_at_file_end() {
    let t = repo_with("f.txt", b"a\nb\nc\nd\n");
    assert_eq!(window(t.path(), "d", 1, 5).unwrap(), "c\nd");
}

#[test]
fn window_takes_lines_around_the_match() {
    let t = repo_with("f.txt", b"a\nb\nc\nd\ne\n");
    assert_eq!(window(t.path(), "c", 1, 1).unwrap(), "b\nc\nd");
}

#[test]
fn window_of_a_dead_anchor_is_none() {
    let t = repo_with("f.txt", b"a\n");
    assert_eq!(window(t.path(), "zzz", 1, 1), None);
    assert_eq!(source_window(t.path(), &anchor("no.txt", "a"), 1, 1), None);
}

#[test]
fn invalid_utf8_does_not_panic() {
    let t = repo_with("f.txt", b"\xff\xfe\nname\n\xc0");
    assert_eq!(
        resolve(t.path(), &anchor("f.txt", "name")),
        Resolution::Found { line: 2 }
    );
    assert_eq!(window(t.path(), "name", 0, 0).unwrap(), "name");
}

#[test]
fn absolute_path_is_dead_path_and_has_no_window() {
    let outside = repo_with("secret.txt", b"BEGIN\n");
    let abs = outside
        .path()
        .join("secret.txt")
        .to_string_lossy()
        .into_owned();
    let root = repo_with("f.txt", b"x");
    let a = anchor(&abs, "BEGIN");
    assert_eq!(resolve(root.path(), &a), Resolution::DeadPath);
    assert_eq!(source_window(root.path(), &a, 1, 1), None);
}

#[test]
fn parent_dir_path_is_dead_path_and_has_no_window() {
    let outer = repo_with("secret.txt", b"BEGIN\n");
    let inner = outer.path().join("repo");
    std::fs::create_dir(&inner).unwrap();
    let a = anchor("../secret.txt", "BEGIN");
    assert_eq!(resolve(&inner, &a), Resolution::DeadPath);
    assert_eq!(source_window(&inner, &a, 1, 1), None);
}

#[test]
fn empty_name_is_dead_name() {
    let t = repo_with("f.txt", b"abc\n");
    let a = anchor("f.txt", "");
    assert_eq!(resolve(t.path(), &a), Resolution::DeadName);
    assert_eq!(source_window(t.path(), &a, 1, 1), None);
}

#[test]
fn empty_file_is_dead_name_without_panic() {
    let t = repo_with("f.txt", b"");
    assert_eq!(
        resolve(t.path(), &anchor("f.txt", "x")),
        Resolution::DeadName
    );
    assert_eq!(window(t.path(), "x", 1, 1), None);
    assert_eq!(window(t.path(), "", 1, 1), None);
}

#[test]
fn match_overlapping_a_rejected_match_is_found() {
    let t = repo_with("f.txt", b"ax-x-x\n");
    assert_eq!(
        resolve(t.path(), &anchor("f.txt", "x-x")),
        Resolution::Found { line: 1 }
    );
}

#[test]
fn multibyte_neighbours_are_not_word_characters() {
    let t = repo_with("f.txt", "\u{e9}name\nname\u{e9}\n".as_bytes());
    assert_eq!(
        resolve(t.path(), &anchor("f.txt", "name")),
        Resolution::Found { line: 1 }
    );
    let t = repo_with("f.txt", "name\u{e9}\n".as_bytes());
    assert_eq!(
        resolve(t.path(), &anchor("f.txt", "name")),
        Resolution::Found { line: 1 }
    );
}

#[cfg(unix)]
mod symlinks {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn symlinked_file_pointing_outside_the_repo_is_dead_path() {
        let outside = repo_with("secret.txt", b"BEGIN\n");
        let root = repo_with("f.txt", b"x");
        symlink(
            outside.path().join("secret.txt"),
            root.path().join("link.txt"),
        )
        .unwrap();
        let a = anchor("link.txt", "BEGIN");
        assert_eq!(resolve(root.path(), &a), Resolution::DeadPath);
        assert_eq!(source_window(root.path(), &a, 1, 1), None);
    }

    #[test]
    fn symlinked_file_pointing_inside_the_repo_is_dead_path() {
        let root = repo_with(".env", b"TOKEN=1\n");
        symlink(".env", root.path().join("link.txt")).unwrap();
        let a = anchor("link.txt", "TOKEN");
        assert_eq!(resolve(root.path(), &a), Resolution::DeadPath);
        assert_eq!(source_window(root.path(), &a, 1, 1), None);
    }

    #[test]
    fn real_file_under_a_symlinked_directory_is_dead_path() {
        let outside = repo_with("secret.txt", b"BEGIN\n");
        let root = repo_with("f.txt", b"x");
        symlink(outside.path(), root.path().join("dir")).unwrap();
        let a = anchor("dir/secret.txt", "BEGIN");
        assert_eq!(resolve(root.path(), &a), Resolution::DeadPath);
        assert_eq!(source_window(root.path(), &a, 1, 1), None);
    }
}

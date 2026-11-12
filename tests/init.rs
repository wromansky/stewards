#[allow(dead_code)] // each test crate uses a subset of the helpers
mod common;

use std::path::{Path, PathBuf};

use common::{git, stewards};
use serde_json::Value;
use stewards::guide;
use stewards::model::load_repo;
use tempfile::TempDir;

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn commit(dir: &Path, files: &[String], text: &str) {
    for f in files {
        write(dir, f, text);
    }
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", text]);
}

/// A repo named `proj` with one commit of `initial`, then one commit per round touching its files.
fn build(initial: &[String], rounds: &[Vec<String>]) -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("proj");
    std::fs::create_dir(&dir).unwrap();
    git(&dir, &["init", "-q"]);
    git(&dir, &["config", "user.name", "stewards-test"]);
    git(&dir, &["config", "user.email", "test@stewards.invalid"]);
    git(&dir, &["config", "commit.gpgsign", "false"]);
    commit(&dir, initial, "initial");
    for (i, round) in rounds.iter().enumerate() {
        commit(&dir, round, &format!("round {i}"));
    }
    (tmp, dir)
}

fn s(items: &[&str]) -> Vec<String> {
    items.iter().map(|i| i.to_string()).collect()
}

/// `n` files in `dir`.
fn files_in(dir: &str, n: usize) -> Vec<String> {
    (0..n).map(|i| format!("{dir}/f{i}.py")).collect()
}

/// Two commits per dir, each touching only that dir's first file.
fn solo_rounds(dirs: &[&str]) -> Vec<Vec<String>> {
    (0..2)
        .flat_map(|_| dirs.iter().map(|d| vec![format!("{d}/f0.py")]))
        .collect()
}

fn init(dir: &Path) -> Value {
    let out = stewards(dir, &["init"]);
    serde_json::from_slice(&out.stdout).expect("stdout is JSON")
}

fn units(v: &Value) -> Vec<(String, Vec<String>)> {
    v["units"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| {
            let paths = u["paths"].as_array().unwrap();
            (
                u["name"].as_str().unwrap().into(),
                paths.iter().map(|p| p.as_str().unwrap().into()).collect(),
            )
        })
        .collect()
}

fn unit(name: &str, paths: &[&str]) -> (String, Vec<String>) {
    (name.into(), s(paths))
}

#[test]
fn single_manifest_repo_splits_independent_dirs_and_joins_mirrored_tests() {
    let mut initial = s(&["pyproject.toml", "README.md"]);
    for d in ["src/a", "src/b"] {
        initial.extend(files_in(d, 3));
    }
    initial.extend(files_in("tests/a", 1));
    let (_tmp, dir) = build(&initial, &solo_rounds(&["src/a", "src/b"]));
    let v = init(&dir);
    assert_eq!(
        units(&v),
        [
            unit("a", &["src/a/**", "tests/a/**"]),
            unit("b", &["src/b/**"])
        ]
    );
    assert_eq!(
        v["unowned"],
        serde_json::json!(["README.md", "pyproject.toml"])
    );
}

#[test]
fn monorepo_gets_units_per_manifest_root() {
    let mut initial = s(&["web/package.json", "svc/Cargo.toml"]);
    let dirs = ["web/src/ui", "web/src/net", "svc/src/core", "svc/src/db"];
    for d in dirs {
        initial.extend(files_in(d, 3));
    }
    let (_tmp, dir) = build(&initial, &solo_rounds(&dirs));
    assert_eq!(
        units(&init(&dir)),
        [
            unit("core", &["svc/src/core/**"]),
            unit("db", &["svc/src/db/**"]),
            unit("net", &["web/src/net/**"]),
            unit("ui", &["web/src/ui/**"]),
        ]
    );
}

#[test]
fn a_repeated_unit_name_in_a_second_root_takes_the_root_name() {
    let mut initial = s(&["web/package.json", "svc/Cargo.toml"]);
    let dirs = ["web/src/core", "web/src/ui", "svc/src/core", "svc/src/db"];
    for d in dirs {
        initial.extend(files_in(d, 3));
    }
    let (_tmp, dir) = build(&initial, &solo_rounds(&dirs));
    let names: Vec<String> = units(&init(&dir)).into_iter().map(|u| u.0).collect();
    assert_eq!(names, ["core", "db", "web-core", "ui"]);
}

#[test]
fn dirs_that_always_change_together_merge_and_an_independent_one_stays() {
    let mut initial = s(&["pyproject.toml"]);
    for d in ["src/a", "src/b", "src/c"] {
        initial.extend(files_in(d, 3));
    }
    let mut rounds: Vec<Vec<String>> = (0..3).map(|_| s(&["src/a/f0.py", "src/b/f0.py"])).collect();
    rounds.extend(solo_rounds(&["src/c"]));
    let (_tmp, dir) = build(&initial, &rounds);
    assert_eq!(
        units(&init(&dir)),
        [
            unit("a", &["src/a/**", "src/b/**"]),
            unit("c", &["src/c/**"])
        ]
    );
}

#[test]
fn a_two_file_candidate_merges_into_its_most_co_changed_sibling() {
    let mut initial = s(&["pyproject.toml"]);
    for d in ["src/a", "src/b"] {
        initial.extend(files_in(d, 3));
    }
    initial.extend(files_in("src/c", 2));
    let mut rounds = solo_rounds(&["src/a", "src/b", "src/c"]);
    rounds.push(s(&["src/b/f0.py", "src/c/f0.py"]));
    let (_tmp, dir) = build(&initial, &rounds);
    assert_eq!(
        units(&init(&dir)),
        [
            unit("a", &["src/a/**"]),
            unit("b", &["src/b/**", "src/c/**"])
        ]
    );
}

#[test]
fn more_than_eight_candidates_keep_the_eight_largest_and_fold_the_rest_into_misc() {
    let dirs: Vec<String> = (0..10).map(|i| format!("src/d{i}")).collect();
    let mut initial = s(&["pyproject.toml"]);
    for (i, d) in dirs.iter().enumerate() {
        initial.extend(files_in(d, if i < 8 { 4 } else { 3 }));
    }
    let dir_refs: Vec<&str> = dirs.iter().map(String::as_str).collect();
    let (_tmp, dir) = build(&initial, &solo_rounds(&dir_refs));
    let got = units(&init(&dir));
    assert_eq!(got.len(), 9);
    assert_eq!(got[8], unit("proj-misc", &["src/d8/**", "src/d9/**"]));
}

#[test]
fn a_dir_that_is_mostly_generated_files_is_skipped() {
    let mut initial = s(&["pyproject.toml", "src/gen/x.py"]);
    initial.extend(s(&[
        "src/gen/a_pb2.py",
        "src/gen/b_pb2.py",
        "src/gen/c_pb2.py",
    ]));
    for d in ["src/a", "src/b"] {
        initial.extend(files_in(d, 3));
    }
    let (_tmp, dir) = build(&initial, &solo_rounds(&["src/a", "src/b"]));
    assert_eq!(
        units(&init(&dir)),
        [unit("a", &["src/a/**"]), unit("b", &["src/b/**"])]
    );
}

#[test]
fn a_repo_with_no_source_dirs_becomes_one_unit() {
    let (_tmp, dir) = build(&s(&["README.md", "main.py"]), &[]);
    let v = init(&dir);
    assert_eq!(units(&v), [unit("proj", &["**"])]);
    assert_eq!(v["unowned"], serde_json::json!([]));
}

#[test]
fn written_files_load_and_every_name_passes_the_name_rule() {
    let mut initial = s(&["pyproject.toml"]);
    initial.extend(files_in("src/Web UI", 3));
    initial.extend(files_in("src/api", 3));
    initial.extend(files_in("tests/api", 1));
    let (_tmp, dir) = build(&initial, &solo_rounds(&["src/Web UI", "src/api"]));
    init(&dir);
    let loaded = load_repo(&dir).unwrap();
    let names: Vec<&str> = loaded.units.iter().map(|u| u.name.as_str()).collect();
    assert_eq!(names, ["web-ui", "api"]);
    assert_eq!(loaded.repo.name, "proj");
    assert_eq!(loaded.repo.test_roots, ["tests"]);
    assert_eq!(loaded.repo.commands.test.len(), 1);
}

#[test]
fn gitignore_lines_are_appended_only_when_missing() {
    let (_tmp, dir) = build(&s(&["main.py"]), &[]);
    write(&dir, ".gitignore", "node_modules\n.stewards/briefs/");
    let v = init(&dir);
    assert_eq!(
        std::fs::read_to_string(dir.join(".gitignore")).unwrap(),
        "node_modules\n.stewards/briefs/\n.stewards/telemetry/\n"
    );
    assert!(
        v["wrote"]
            .as_array()
            .unwrap()
            .contains(&".gitignore".into())
    );
}

#[test]
fn gitignore_lines_written_with_a_leading_or_without_a_trailing_slash_are_present() {
    let (_tmp, dir) = build(&s(&["main.py"]), &[]);
    let old = "/.stewards/telemetry/\n/.stewards/briefs\n";
    write(&dir, ".gitignore", old);
    let v = init(&dir);
    assert_eq!(
        std::fs::read_to_string(dir.join(".gitignore")).unwrap(),
        old
    );
    assert!(
        !v["wrote"]
            .as_array()
            .unwrap()
            .contains(&".gitignore".into())
    );
}

#[test]
fn the_repo_name_is_the_origin_url_last_segment_when_there_is_an_origin() {
    let (_tmp, dir) = build(&s(&["main.py"]), &[]);
    git(
        &dir,
        &[
            "remote",
            "add",
            "origin",
            "https://example.invalid/acme/widgets.git",
        ],
    );
    init(&dir);
    assert_eq!(load_repo(&dir).unwrap().repo.name, "widgets");
}

#[test]
fn agents_md_gets_the_block() {
    let (_tmp, dir) = build(&s(&["main.py"]), &[]);
    let v = init(&dir);
    let text = std::fs::read_to_string(dir.join("AGENTS.md")).unwrap();
    assert!(text.starts_with(guide::block()));
    assert!(v["wrote"].as_array().unwrap().contains(&"AGENTS.md".into()));
}

#[test]
fn a_second_init_refuses_and_changes_nothing() {
    let (_tmp, dir) = build(&s(&["main.py"]), &[]);
    init(&dir);
    let read = |f: &str| std::fs::read_to_string(dir.join(f)).unwrap();
    let before: Vec<String> = [
        ".stewards/units.toml",
        ".stewards/repo.toml",
        "AGENTS.md",
        ".gitignore",
    ]
    .iter()
    .map(|f| read(f))
    .collect();
    write(&dir, ".stewards/units.toml", "# edited by hand\n");
    let out = stewards(&dir, &["init"]);
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(v["error"]["code"], "already_initialized");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("units.toml")
    );
    assert_eq!(read(".stewards/units.toml"), "# edited by hand\n");
    let after: Vec<String> = [".stewards/repo.toml", "AGENTS.md", ".gitignore"]
        .iter()
        .map(|f| read(f))
        .collect();
    assert_eq!(after, before[1..]);
}

#[test]
fn hooks_and_ci_are_not_implemented() {
    let (_tmp, dir) = build(&s(&["main.py"]), &[]);
    for flag in ["--hooks", "--ci"] {
        let out = stewards(&dir, &["init", flag]);
        let v: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(v["error"]["code"], "not_implemented", "{flag}");
    }
    assert!(!dir.join(".stewards").exists());
}

#[test]
fn a_file_belongs_to_the_nearest_manifest_root() {
    let mut initial = s(&["Cargo.toml", "tools/Cargo.toml"]);
    let dirs = ["core", "util", "tools/src/x", "tools/src/y"];
    for d in dirs {
        initial.extend(files_in(d, 3));
    }
    let (_tmp, dir) = build(&initial, &solo_rounds(&dirs));
    assert_eq!(
        units(&init(&dir)),
        [
            unit("core", &["core/**"]),
            unit("util", &["util/**"]),
            unit("x", &["tools/src/x/**"]),
            unit("y", &["tools/src/y/**"]),
        ]
    );
}

#[test]
fn vendored_dirs_are_ignored_but_a_file_named_build_is_kept() {
    let mut initial = s(&[
        "pyproject.toml",
        "src/a/build",
        "vendor/lib/package.json",
        "node_modules/m/pyproject.toml",
    ]);
    initial.extend(files_in("src/a", 2));
    initial.extend(files_in("src/b", 3));
    let (_tmp, dir) = build(&initial, &solo_rounds(&["src/a", "src/b"]));
    assert_eq!(
        units(&init(&dir)),
        [unit("a", &["src/a/**"]), unit("b", &["src/b/**"])]
    );
}

#[test]
fn a_chain_of_co_changes_merges_all_three_dirs() {
    let mut initial = s(&["pyproject.toml"]);
    for d in ["src/a", "src/b", "src/c", "src/d"] {
        initial.extend(files_in(d, 3));
    }
    let mut rounds = vec![
        s(&["src/a/f0.py", "src/b/f0.py"]),
        s(&["src/b/f0.py", "src/c/f0.py"]),
    ];
    rounds.extend(solo_rounds(&["src/d"]));
    let (_tmp, dir) = build(&initial, &rounds);
    assert_eq!(
        units(&init(&dir)),
        [
            unit("a", &["src/a/**", "src/b/**", "src/c/**"]),
            unit("d", &["src/d/**"])
        ]
    );
}

#[test]
fn a_root_with_one_candidate_becomes_one_unit() {
    let mut initial = s(&["pyproject.toml"]);
    initial.extend(files_in("src/a", 3));
    let (_tmp, dir) = build(&initial, &[]);
    assert_eq!(units(&init(&dir)), [unit("proj", &["**"])]);
}

#[test]
fn a_root_of_only_generated_dirs_does_not_own_them() {
    let mut initial = s(&[
        "pyproject.toml",
        "main.py",
        "src/gen/x.py",
        "src/gen/a_pb2.py",
        "src/gen/b_pb2.py",
        "src/gen/c_pb2.py",
    ]);
    initial.extend(files_in("tests/gen", 5));
    let (_tmp, dir) = build(&initial, &[]);
    let v = init(&dir);
    assert_eq!(
        units(&v),
        [unit("proj", &["main.py", "pyproject.toml", "tests/**"])]
    );
    let unowned = v["unowned"].as_array().unwrap();
    assert!(unowned.contains(&"src/gen/a_pb2.py".into()));
    assert!(unowned.contains(&"src/gen/x.py".into()));
}

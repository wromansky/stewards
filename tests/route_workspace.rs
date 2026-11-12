#[allow(dead_code)] // helpers shared across test files
mod common;

use std::path::Path;

use common::{fixture, git, stewards};
use tempfile::TempDir;

/// The workspace fixture with `git init` in each repo and one unit in `backend` (repo name `api-server`).
fn workspace_fixture() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/workspace");
    std::fs::copy(src.join("stewards.toml"), tmp.path().join("stewards.toml")).unwrap();
    for repo in ["backend", "frontend"] {
        let dest = tmp.path().join(repo);
        let status = std::process::Command::new("cp")
            .args(["-R"])
            .arg(src.join(repo))
            .arg(&dest)
            .status()
            .unwrap();
        assert!(status.success());
        git(&dest, &["init", "-q"]);
    }
    std::fs::write(
        tmp.path().join("backend/.stewards/units.toml"),
        "[[unit]]\nname = \"core\"\npurpose = \"the server\"\npaths = [\"src/**\"]\n",
    )
    .unwrap();
    tmp
}

fn route(dir: &Path, paths: &[&str]) -> serde_json::Value {
    let mut args = vec!["route"];
    args.extend(paths);
    let out = stewards(dir, &args);
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn workspace_root_routes_a_backend_file_to_repo_slash_unit() {
    let dir = workspace_fixture();
    let v = route(dir.path(), &["backend/src/main.py", "frontend/src/app.js"]);
    assert_eq!(v["routes"][0]["unit"], "api-server/core");
    assert_eq!(v["routes"][0]["path"], "backend/src/main.py");
    assert!(v["routes"][1]["unit"].is_null(), "{v}");
}

#[test]
fn sibling_repo_path_is_null_not_an_error() {
    let dir = workspace_fixture();
    let v = route(
        &dir.path().join("backend"),
        &["../frontend/src/app.js", "src/main.py"],
    );
    assert!(v["error"].is_null(), "{v}");
    assert!(v["routes"][0]["unit"].is_null(), "{v}");
    assert_eq!(v["routes"][1]["unit"], "api-server/core");
}

#[test]
fn path_outside_every_repo_is_path_outside_repo() {
    let dir = workspace_fixture();
    let v = route(dir.path(), &["stewards.toml"]);
    assert_eq!(v["error"]["code"], "path_outside_repo", "{v}");
}

#[test]
fn single_repo_unit_has_no_repo_prefix() {
    let repo = fixture("single");
    let v = route(repo.path(), &["src/api/items.py"]);
    assert_eq!(v["routes"][0]["unit"], "api");
}

/// A workspace listing `outer` and the repo `outer/inner` nested in it, each initialized with a
/// unit `core` owning `src/**`.
fn nested_workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("stewards.toml"),
        "repos = [\"outer\", \"outer/inner\"]\n",
    )
    .unwrap();
    for (dir, name) in [("outer", "outer"), ("outer/inner", "inner")] {
        let root = tmp.path().join(dir);
        std::fs::create_dir_all(root.join(".stewards")).unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/x.py"), "x = 1\n").unwrap();
        std::fs::write(
            root.join(".stewards/repo.toml"),
            format!("schema = 1\nname = \"{name}\"\n"),
        )
        .unwrap();
        std::fs::write(
            root.join(".stewards/units.toml"),
            "[[unit]]\nname = \"core\"\npaths = [\"src/**\"]\n",
        )
        .unwrap();
        git(&root, &["init", "-q"]);
    }
    tmp
}

fn run(dir: &Path, args: &[&str]) -> (i32, serde_json::Value) {
    let out = stewards(dir, args);
    (
        out.status.code().unwrap(),
        serde_json::from_slice(&out.stdout).unwrap(),
    )
}

#[test]
fn route_picks_the_inner_of_two_nested_repos() {
    let dir = nested_workspace();
    let v = route(dir.path(), &["outer/inner/src/x.py", "outer/src/x.py"]);
    assert_eq!(v["routes"][0]["unit"], "inner/core", "{v}");
    assert_eq!(v["routes"][1]["unit"], "outer/core", "{v}");
}

#[test]
fn stamp_inside_a_nested_repo_writes_the_inner_card() {
    let dir = nested_workspace();
    let (code, v) = run(&dir.path().join("outer/inner/src"), &["stamp", "core"]);
    assert_eq!(code, 0, "{v}");
    assert!(
        dir.path()
            .join("outer/inner/.stewards/units/core.toml")
            .is_file()
    );
    assert!(!dir.path().join("outer/.stewards/units/core.toml").exists());
}

#[test]
fn check_with_repo_naming_an_uninitialized_repo_is_no_stewards_dir() {
    let dir = workspace_fixture();
    let (code, v) = run(dir.path(), &["check", "--repo", "frontend"]);
    assert_eq!(code, 2);
    assert_eq!(v["error"]["code"], "no_stewards_dir", "{v}");
}

#[test]
fn check_in_a_workspace_skips_an_uninitialized_sibling_and_prefixes_units() {
    let dir = workspace_fixture();
    let (code, v) = run(dir.path(), &["check"]);
    assert_eq!(code, 1, "{v}");
    let failures: Vec<_> = v["failures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["kind"].as_str().unwrap(), f["unit"].as_str().unwrap()))
        .collect();
    assert_eq!(failures, [("unreviewed", "api-server/core")]);
}

#[test]
fn glob_with_an_escaped_bracket_matches_literally() {
    let unit = stewards::model::UnitDef {
        name: "page".into(),
        purpose: String::new(),
        paths: vec![r"app/\[id\]/**".into()],
    };
    let r = stewards::route::Router::new(&[unit], "docs/units").unwrap();
    assert_eq!(r.owner("app/[id]/page.js"), Some("page"));
    assert_eq!(r.owner("app/i/page.js"), None);
}

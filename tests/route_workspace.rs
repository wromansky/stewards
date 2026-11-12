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

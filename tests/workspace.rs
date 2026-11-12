#[allow(dead_code)] // helpers shared across test files
mod common;

use common::{fixture, workspace_fixture};
use stewards::output::CliError;
use stewards::workspace::{Workspace, discover};
use tempfile::TempDir;

fn error_of(r: anyhow::Result<Workspace>) -> CliError {
    r.err()
        .expect("an error")
        .downcast_ref::<CliError>()
        .expect("a CliError")
        .clone()
}

#[test]
fn single_repo_is_found_from_a_subdirectory() {
    let repo = fixture("single");
    let ws = discover(&repo.path().join("src/api"), None).unwrap();
    assert!(!ws.is_workspace);
    assert_eq!(ws.repos.len(), 1);
    assert_eq!(ws.repos[0].root, repo.path());
    assert_eq!(ws.repos[0].name, "single");
    assert!(ws.repos[0].loaded.is_some());
}

#[test]
fn workspace_lists_both_repos_with_one_unloaded() {
    let dir = workspace_fixture();
    let ws = discover(dir.path(), None).unwrap();
    assert!(ws.is_workspace);
    let found: Vec<_> = ws
        .repos
        .iter()
        .map(|r| (r.name.as_str(), r.loaded.is_some()))
        .collect();
    assert_eq!(found, [("api-server", true), ("frontend", false)]);
}

#[test]
fn workspace_is_found_from_inside_a_repo() {
    let dir = workspace_fixture();
    let ws = discover(&dir.path().join("frontend/src"), None).unwrap();
    assert!(ws.is_workspace);
    assert_eq!(ws.repos.len(), 2);
}

#[test]
fn missing_listed_path_is_workspace_repo_missing() {
    let dir = workspace_fixture();
    std::fs::write(
        dir.path().join("stewards.toml"),
        "repos = [\"backend\", \"gone\"]\n",
    )
    .unwrap();
    let e = error_of(discover(dir.path(), None));
    assert_eq!(e.code, "workspace_repo_missing");
    assert!(e.message.contains("gone"), "{}", e.message);
}

#[test]
fn only_selects_one_repo() {
    let dir = workspace_fixture();
    let ws = discover(dir.path(), Some("frontend")).unwrap();
    assert!(ws.is_workspace);
    assert_eq!(ws.repos.len(), 1);
    assert_eq!(ws.repos[0].name, "frontend");
}

#[test]
fn only_with_unknown_name_lists_the_available_repos() {
    let dir = workspace_fixture();
    let e = error_of(discover(dir.path(), Some("nope")));
    assert_eq!(e.code, "unknown_repo");
    assert!(e.message.contains("api-server, frontend"), "{}", e.message);
}

#[test]
fn display_name_is_prefixed_only_in_a_workspace() {
    let dir = workspace_fixture();
    let ws = discover(dir.path(), None).unwrap();
    assert_eq!(ws.display_name(&ws.repos[0], "api"), "api-server/api");

    let repo = fixture("single");
    let ws = discover(repo.path(), None).unwrap();
    assert_eq!(ws.display_name(&ws.repos[0], "api"), "api");
}

#[test]
fn outside_any_git_repo_is_not_a_git_repo() {
    let dir = TempDir::new().unwrap();
    assert_eq!(error_of(discover(dir.path(), None)).code, "not_a_git_repo");
}

#[test]
fn loaded_card_returns_the_stored_card_or_an_empty_one() {
    let repo = fixture("single");
    std::fs::remove_file(repo.path().join(".stewards/units/store.toml")).unwrap();
    let ws = discover(repo.path(), None).unwrap();
    let loaded = ws.repos[0].loaded.as_ref().unwrap();

    let api = loaded.card("api");
    assert_eq!(api.unit, "api");
    assert_eq!(api.interface.len(), loaded.cards["api"].interface.len());

    let store = loaded.card("store");
    assert_eq!(store.unit, "store");
    assert!(store.reviewed.is_none());
    assert!(store.interface.is_empty() && store.anchors.is_empty() && store.gaps.is_empty());
    assert!(store.doc.is_none());
    assert!(!loaded.cards.contains_key("store"));
}

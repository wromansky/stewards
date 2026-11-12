#[allow(dead_code)] // each test crate uses a subset of the helpers
mod common;

use std::path::Path;

use common::{fixture, git, stewards};
use serde_json::Value;

fn run(dir: &Path, args: &[&str]) -> Value {
    let out = stewards(dir, args);
    serde_json::from_slice(&out.stdout).unwrap()
}

fn golden(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/docs")
            .join(name),
    )
    .unwrap()
}

fn read(dir: &Path, rel: &str) -> String {
    std::fs::read_to_string(dir.join(rel)).unwrap()
}

#[test]
fn unit_page_matches_golden() {
    let tmp = fixture("single");
    run(tmp.path(), &["docs"]);
    assert_eq!(read(tmp.path(), "docs/units/api.md"), golden("api.md"));
}

#[test]
fn index_matches_golden() {
    let tmp = fixture("single");
    run(tmp.path(), &["docs"]);
    assert_eq!(
        read(tmp.path(), "docs/units/README.md"),
        golden("README.md")
    );
}

#[test]
fn dead_anchor_renders_without_a_link_and_succeeds() {
    let tmp = fixture("single");
    let card = tmp.path().join(".stewards/units/store.toml");
    let text = std::fs::read_to_string(&card).unwrap();
    std::fs::write(
        &card,
        text.replace("db.py::save_item", "gone.py::save_item"),
    )
    .unwrap();
    let out = stewards(tmp.path(), &["docs", "store"]);
    assert!(out.status.success());
    let page = read(tmp.path(), "docs/units/store.md");
    assert!(page.contains("| `src/store/gone.py::save_item` |"));
    assert!(page.contains("[`src/store/db.py::count_items`](../../src/store/db.py#L"));
}

#[test]
fn missing_prose_prints_the_not_written_line() {
    let tmp = fixture("single");
    run(tmp.path(), &["docs", "api"]);
    let page = read(tmp.path(), "docs/units/api.md");
    assert!(page.contains("## How it works\n\n_Not written yet._\n"));
    assert!(page.contains("## How to change it\n\n_Not written yet._\n"));
    assert!(!page.contains("## Overview\n\n_Not written yet._"));
}

#[test]
fn second_render_writes_nothing() {
    let tmp = fixture("single");
    let first = run(tmp.path(), &["docs"]);
    assert_eq!(first["wrote"].as_array().unwrap().len(), 3);
    let second = run(tmp.path(), &["docs"]);
    assert_eq!(second["wrote"], serde_json::json!([]));
}

#[test]
fn one_unit_writes_its_page_and_the_index_only() {
    let tmp = fixture("single");
    let report = run(tmp.path(), &["docs", "api"]);
    assert_eq!(
        report["wrote"],
        serde_json::json!(["docs/units/api.md", "docs/units/README.md"])
    );
    assert!(!tmp.path().join("docs/units/store.md").exists());
}

#[test]
fn unknown_unit_is_an_error() {
    let tmp = fixture("single");
    let out = stewards(tmp.path(), &["docs", "nope"]);
    assert_eq!(out.status.code(), Some(2));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["error"]["code"], "unknown_unit");
}

#[test]
fn nothing_outside_docs_path_changes() {
    let tmp = fixture("single");
    run(tmp.path(), &["docs"]);
    let status = git(tmp.path(), &["status", "--porcelain"]);
    assert_eq!(status.trim(), "?? docs/");
}

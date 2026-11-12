#[allow(dead_code)] // each test crate uses a subset of the helpers
mod common;

use std::path::Path;

use common::{fixture, git, stewards};
use serde_json::Value;

fn debt(dir: &Path) -> Vec<Value> {
    let out = stewards(dir, &["debt"]);
    assert!(out.status.success(), "{out:?}");
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    v["units"].as_array().unwrap().clone()
}

fn items_of(units: &[Value], unit: Value) -> &[Value] {
    units
        .iter()
        .find(|u| u["unit"] == unit)
        .map_or(&[], |u| u["items"].as_array().unwrap())
}

fn add(dir: &Path, path: &str, content: &[u8]) {
    let file = dir.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, content).unwrap();
    git(dir, &["add", "-A"]);
}

#[test]
fn finds_code_and_markdown_comments_under_their_units() {
    let tmp = fixture("single");
    let units = debt(tmp.path());
    let api = items_of(&units, "api".into());
    assert_eq!(api.len(), 1);
    assert_eq!(api[0]["path"], "src/api/items.py");
    assert_eq!(api[0]["line"], 14);
    assert_eq!(
        api[0]["text"],
        "# simplified: returns every item; paginate once lists pass 1000"
    );
    let store = items_of(&units, "store".into());
    assert_eq!(store.len(), 1);
    assert_eq!(store[0]["path"], "src/store/README.md");
    assert_eq!(store[0]["line"], 5);
}

#[test]
fn simplified_without_a_marker_before_it_is_ignored() {
    let tmp = fixture("single");
    add(tmp.path(), "src/api/plain.py", b"simplified: no comment\n");
    assert_eq!(items_of(&debt(tmp.path()), "api".into()).len(), 1);
}

#[test]
fn marker_after_simplified_does_not_count() {
    let tmp = fixture("single");
    add(tmp.path(), "src/api/late.py", b"x = 1\nsimplified: x # y\n");
    assert_eq!(items_of(&debt(tmp.path()), "api".into()).len(), 1);
}

#[test]
fn match_is_case_insensitive() {
    let tmp = fixture("single");
    add(tmp.path(), "src/api/case.py", b"x = 1\n# SIMPLIFIED: x\n");
    let units = debt(tmp.path());
    let hit = items_of(&units, "api".into())
        .iter()
        .find(|i| i["path"] == "src/api/case.py")
        .unwrap();
    assert_eq!(hit["line"], 2);
}

#[test]
fn binary_file_is_skipped() {
    let tmp = fixture("single");
    add(tmp.path(), "src/api/blob.bin", b"\0\n# simplified: x\n");
    assert_eq!(items_of(&debt(tmp.path()), "api".into()).len(), 1);
}

#[test]
fn unowned_file_lands_under_the_null_unit() {
    let tmp = fixture("single");
    add(tmp.path(), "Makefile", b"all:\n\t# simplified: x\n");
    let units = debt(tmp.path());
    let loose = items_of(&units, Value::Null);
    assert_eq!(loose.len(), 1);
    assert_eq!(loose[0]["path"], "Makefile");
    assert_eq!(loose[0]["line"], 2);
}

#[test]
fn stewards_dir_is_skipped() {
    let tmp = fixture("single");
    add(tmp.path(), ".stewards/note.md", b"# simplified: x\n");
    assert!(items_of(&debt(tmp.path()), Value::Null).is_empty());
}

#[test]
fn human_output_groups_by_unit() {
    let tmp = fixture("single");
    let out = stewards(tmp.path(), &["debt", "--human"]);
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("## api\n\n- src/api/items.py:14: # simplified:"));
}

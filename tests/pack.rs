#[allow(dead_code)] // each test crate uses a subset of the helpers
mod common;

use std::fs;
use std::path::Path;

use common::{fixture, stewards};
use serde_json::Value;
use stewards::model::Role;
use stewards::pack::pack;
use stewards::workspace::discover;

fn run(dir: &Path, args: &[&str]) -> (i32, Value) {
    let out = stewards(dir, args);
    let v = serde_json::from_slice(&out.stdout).unwrap_or_else(|_| {
        panic!(
            "not JSON: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    });
    (out.status.code().unwrap(), v)
}

fn pack_json(dir: &Path, args: &[&str]) -> Value {
    let (code, v) = run(dir, args);
    assert_eq!(code, 0, "{v}");
    v
}

fn edit(path: &Path, from: &str, to: &str) {
    let text = fs::read_to_string(path).unwrap();
    assert!(text.contains(from), "{from} not in {}", path.display());
    fs::write(path, text.replace(from, to)).unwrap();
}

fn append(path: &Path, text: &str) {
    let old = fs::read_to_string(path).unwrap();
    fs::write(path, old + text).unwrap();
}

/// Asserts that exactly the `present` sections of the pack `v` are non-null.
fn assert_sections(v: &Value, present: &[&str]) {
    const ALL: [&str; 19] = [
        "purpose",
        "interface",
        "anchors",
        "scenarios",
        "test_command",
        "primary_paths",
        "test_commands",
        "gaps",
        "flows",
        "decisions",
        "units",
        "doc",
        "changed_files",
        "hunks",
        "other_changed_files",
        "memory",
        "bug",
        "dev_task",
        "repro_result",
    ];
    for s in ALL {
        assert_eq!(
            !v[s].is_null(),
            present.contains(&s),
            "section `{s}` in {v}"
        );
    }
}

#[test]
fn maintainer_pack_has_purpose_interface_anchors_gaps_flows() {
    let repo = fixture("single");
    let v = pack_json(repo.path(), &["pack", "api", "--role", "maintainer"]);
    assert_sections(&v, &["purpose", "interface", "anchors", "gaps", "flows"]);
    assert_eq!(v["anchors"][0]["name"], "router");
    assert_eq!(v["anchors"][0]["line"], 3);
    assert!(
        v["anchors"][0]["source"]
            .as_str()
            .unwrap()
            .contains("router = []")
    );
    assert_eq!(v["flows"][0]["name"], "create-item");
}

#[test]
fn tester_pack_has_interface_source_scenarios_and_test_command() {
    let repo = fixture("single");
    let v = pack_json(repo.path(), &["pack", "api", "--role", "tester"]);
    assert_sections(
        &v,
        &[
            "interface",
            "anchors",
            "scenarios",
            "test_command",
            "primary_paths",
        ],
    );
    assert_eq!(v["anchors"][0]["name"], "create_item");
    assert!(
        v["anchors"][0]["source"]
            .as_str()
            .unwrap()
            .contains("def create_item")
    );
    assert_eq!(v["scenarios"][0]["id"], "create-ok");
    assert_eq!(v["test_command"], "pytest tests/api");
}

#[test]
fn documenter_pack_has_purpose_interface_doc_flows() {
    let repo = fixture("single");
    let v = pack_json(repo.path(), &["pack", "api", "--role", "documenter"]);
    assert_sections(&v, &["purpose", "interface", "doc", "flows"]);
    assert_eq!(v["doc"]["overview"], "The HTTP surface for items.");
}

#[test]
fn validator_pack_has_purpose_interface_paths_and_test_commands() {
    let repo = fixture("single");
    let v = pack_json(repo.path(), &["pack", "api", "--role", "validator"]);
    assert_sections(
        &v,
        &[
            "purpose",
            "interface",
            "primary_paths",
            "test_command",
            "test_commands",
        ],
    );
    assert_eq!(v["test_commands"], serde_json::json!(["pytest"]));
}

#[test]
fn architect_pack_has_every_unit_flows_decisions_and_no_source() {
    let repo = fixture("single");
    let v = pack_json(repo.path(), &["pack", "--role", "architect"]);
    assert_sections(&v, &["units", "flows", "decisions"]);
    assert_eq!(
        v["units"],
        serde_json::json!([
            {"name": "api", "purpose": "HTTP surface", "interface_ids": ["create-item"]},
            {"name": "store", "purpose": "item persistence", "interface_ids": ["save-item", "count-items"]},
        ])
    );
    assert_eq!(v["decisions"][0]["title"], "store has no HTTP knowledge");
    assert!(!v.to_string().contains("save_item("));
}

#[test]
fn developer_pack_is_not_implemented() {
    let repo = fixture("single");
    let (code, v) = run(repo.path(), &["pack", "api", "--role", "developer"]);
    assert_eq!(code, 2);
    assert_eq!(v["error"]["code"], "not_implemented");
}

#[test]
fn architect_with_a_unit_is_a_usage_error() {
    let repo = fixture("single");
    let (code, v) = run(repo.path(), &["pack", "api", "--role", "architect"]);
    assert_eq!(code, 2);
    assert_eq!(v["error"]["code"], "usage");
}

#[test]
fn maintainer_without_a_unit_is_a_usage_error() {
    let repo = fixture("single");
    let (code, v) = run(repo.path(), &["pack", "--role", "maintainer"]);
    assert_eq!(code, 2);
    assert_eq!(v["error"]["code"], "usage");
}

#[test]
fn unknown_unit_is_unknown_unit() {
    let repo = fixture("single");
    let (code, v) = run(repo.path(), &["pack", "nope", "--role", "maintainer"]);
    assert_eq!(code, 2);
    assert_eq!(v["error"]["code"], "unknown_unit");
}

#[test]
fn dead_anchor_is_listed_with_null_line_and_source() {
    let repo = fixture("single");
    edit(
        &repo.path().join(".stewards/units/api.toml"),
        "name = \"router\"",
        "name = \"gone\"",
    );
    let v = pack_json(repo.path(), &["pack", "api", "--role", "maintainer"]);
    let a = &v["anchors"][0];
    assert_eq!(a["name"], "gone");
    assert!(a["line"].is_null() && a["source"].is_null(), "{a}");
}

#[test]
fn excluded_file_is_named_but_not_inlined() {
    let repo = fixture("single");
    edit(
        &repo.path().join(".stewards/repo.toml"),
        "exclude = [",
        "exclude = [\"items.py\", ",
    );
    let out = stewards(repo.path(), &["pack", "api", "--role", "maintainer"]);
    let text = String::from_utf8(out.stdout).unwrap();
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["anchors"][0]["path"], "src/api/items.py");
    assert!(v["anchors"][0]["source"].is_null());
    assert!(!text.contains("router = []"), "{text}");
    assert!(
        v["dropped"][0]
            .as_str()
            .unwrap()
            .contains("src/api/items.py::router")
    );
}

#[test]
fn diff_after_a_stamp_shows_hunks_restricts_anchors_and_lists_other_units_files() {
    let repo = fixture("single");
    let dir = repo.path();
    append(
        &dir.join(".stewards/units/api.toml"),
        "[[anchor]]\npath = \"tests/api/test_items.py\"\nname = \"test_create_ok\"\n",
    );
    for unit in ["api", "store"] {
        assert_eq!(run(dir, &["stamp", unit]).0, 0);
    }
    edit(&dir.join("src/api/items.py"), "router = []", "router = [1]");
    edit(&dir.join("src/store/db.py"), "_items = []", "_items = [0]");
    let v = pack_json(dir, &["pack", "api", "--role", "maintainer", "--diff"]);
    assert_eq!(v["changed_files"], serde_json::json!(["src/api/items.py"]));
    let paths: Vec<_> = v["anchors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, ["src/api/items.py"]);
    assert_eq!(v["hunks"][0]["path"], "src/api/items.py");
    assert!(
        v["hunks"][0]["diff"]
            .as_str()
            .unwrap()
            .contains("+router = [1]")
    );
    assert_eq!(
        v["other_changed_files"],
        serde_json::json!(["src/store/db.py"])
    );
}

#[test]
fn diff_with_no_stamp_equals_the_plain_pack() {
    let repo = fixture("single");
    let args = ["pack", "api", "--role", "maintainer", "--task", "t"];
    let plain = pack_json(repo.path(), &args);
    let diff = pack_json(repo.path(), &[&args[..], &["--diff"]].concat());
    assert_eq!(plain, diff);
}

#[test]
fn fit_shrinks_in_the_fixed_order() {
    let repo = fixture("single");
    let dir = repo.path();
    append(
        &dir.join(".stewards/units/api.toml"),
        "[[anchor]]\npath = \"src/api/items.py\"\nname = \"list_items\"\n",
    );
    assert_eq!(run(dir, &["stamp", "api"]).0, 0);
    let extra: String = (0..60)
        .map(|i| format!("EXTRA_{i} = \"a padded line to make the window and hunk long\"\n"))
        .collect();
    append(&dir.join("src/api/items.py"), &extra);

    let ws = discover(dir, None).unwrap();
    let mut p = pack(&ws.repos[0], Some("api"), Role::Maintainer, "t", true).unwrap();
    assert!(p.dropped.is_empty() && !p.over_budget);
    let steps = [
        "source windows halved",
        "source src/api/items.py::list_items",
        "source src/api/items.py::router",
        "gaps",
        "flows",
        "hunks truncated to 40 lines",
    ];
    for (i, _) in steps.iter().enumerate() {
        p.fit(p.bytes - 1);
        assert_eq!(p.dropped, steps[..=i], "after step {i}");
        assert!(!p.over_budget);
        assert_eq!(p.bytes, p.render().len());
    }
}

#[test]
fn never_dropped_sections_survive_a_tiny_budget() {
    let repo = fixture("single");
    let dir = repo.path();
    for unit in ["api", "store"] {
        assert_eq!(run(dir, &["stamp", unit]).0, 0);
    }
    edit(&dir.join("src/store/db.py"), "_items = []", "_items = [0]");
    let ws = discover(dir, None).unwrap();

    let mut p = pack(&ws.repos[0], Some("api"), Role::Maintainer, "t", true).unwrap();
    p.fit(1);
    assert!(p.over_budget);
    assert_eq!(p.bytes, p.render().len());
    assert_eq!(p.purpose.as_deref(), Some("HTTP surface"));
    assert_eq!(p.interface.as_ref().unwrap().len(), 1);
    assert_eq!(
        p.other_changed_files.as_ref().unwrap(),
        &["src/store/db.py"]
    );

    let mut t = pack(&ws.repos[0], Some("api"), Role::Tester, "t", false).unwrap();
    t.fit(1);
    assert!(t.over_budget);
    assert_eq!(t.scenarios.as_ref().unwrap().len(), 1);
}

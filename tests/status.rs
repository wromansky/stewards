#[allow(dead_code)] // each test crate uses a subset of the helpers
mod common;

use std::fs;

use common::{append, fixture, git, run, stewards, workspace_fixture};
use serde_json::Value;

/// The status entry of `unit`.
fn unit(v: &Value, unit: &str) -> Value {
    v["units"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["unit"] == unit)
        .unwrap_or_else(|| panic!("no unit {unit} in {v}"))
        .clone()
}

#[test]
fn fresh_fixture_is_unreviewed_with_counts() {
    let repo = fixture("single");
    let (code, v) = run(repo.path(), &["status"]);
    assert_eq!(code, 0);
    assert_eq!(v["uninitialized"], serde_json::json!([]));
    assert!(v.get("telemetry").is_none());
    let api = unit(&v, "api");
    assert_eq!(api["freshness"], "unreviewed");
    assert_eq!(api["purpose"], "HTTP surface");
    assert_eq!(
        api["paths"],
        serde_json::json!(["src/api/**", "tests/api/**"])
    );
    assert_eq!(
        (api["interfaces"].as_u64(), api["covered"].as_u64()),
        (Some(1), Some(1))
    );
    assert_eq!(
        (api["gaps"].as_u64(), api["bugs"].as_u64()),
        (Some(1), Some(0))
    );
    let store = unit(&v, "store");
    assert_eq!(store["freshness"], "unreviewed");
    assert_eq!(
        (store["interfaces"].as_u64(), store["covered"].as_u64()),
        (Some(2), Some(1))
    );
}

#[test]
fn debt_is_counted_per_unit() {
    let repo = fixture("single");
    let file = repo.path().join("src/store/db.py");
    append(&file, "# simplified: no index\n# simplified: no locking\n");
    git(repo.path(), &["add", "-A"]);
    let (_, v) = run(repo.path(), &["status"]);
    assert_eq!(unit(&v, "store")["debt"], 3); // two added, one in the fixture README
    assert_eq!(unit(&v, "api")["debt"], 1);
}

#[test]
fn stamped_unit_is_fresh() {
    let repo = fixture("single");
    assert_eq!(run(repo.path(), &["stamp", "api"]).0, 0);
    let (_, v) = run(repo.path(), &["status"]);
    assert_eq!(unit(&v, "api")["freshness"], "fresh");
    assert_eq!(unit(&v, "store")["freshness"], "unreviewed");
}

#[test]
fn edited_unit_is_stale_and_status_still_exits_zero() {
    let repo = fixture("single");
    assert_eq!(run(repo.path(), &["stamp", "api"]).0, 0);
    let file = repo.path().join("src/api/items.py");
    append(&file, "\n# changed\n");
    let (code, v) = run(repo.path(), &["status"]);
    assert_eq!(code, 0);
    assert_eq!(unit(&v, "api")["freshness"], "stale");
}

#[test]
fn bug_gap_is_counted_as_a_bug_not_a_gap() {
    let repo = fixture("single");
    let card = repo.path().join(".stewards/units/api.toml");
    append(
        &card,
        "\n[[gap]]\nkind = \"bug\"\ntitle = \"create drops tags\"\nnote = \"\"\n",
    );
    let (_, v) = run(repo.path(), &["status"]);
    let api = unit(&v, "api");
    assert_eq!(
        (api["gaps"].as_u64(), api["bugs"].as_u64()),
        (Some(1), Some(1))
    );
}

#[test]
fn telemetry_flag_adds_a_null_with_no_telemetry_files() {
    let repo = fixture("single");
    let (_, v) = run(repo.path(), &["status", "--telemetry"]);
    assert!(v["telemetry"].is_null());
    assert!(v.as_object().unwrap().contains_key("telemetry"));
}

#[test]
fn telemetry_flag_summarises_the_telemetry_files() {
    let repo = fixture("single");
    let dir = repo.path().join(".stewards/telemetry");
    fs::create_dir_all(&dir).unwrap();
    let line = |role: &str, bytes: u64, pulled: &[&str]| {
        serde_json::json!({"role": role, "bytes": bytes, "pulled_anchors": pulled}).to_string()
    };
    let lines = [
        line("maintainer", 100, &["a.py::x"]),
        line("maintainer", 301, &[]),
        line("tester", 50, &["a.py::x", "b.py::y"]),
    ];
    fs::write(dir.join("2026-10.jsonl"), format!("{}\n", lines[0])).unwrap();
    fs::write(
        dir.join("2026-11.jsonl"),
        format!("{}\n{}\n", lines[1], lines[2]),
    )
    .unwrap();
    let (_, v) = run(repo.path(), &["status", "--telemetry"]);
    assert_eq!(
        v["telemetry"],
        serde_json::json!({
            "submits": {"maintainer": 2, "tester": 1},
            "mean_pack_bytes": {"maintainer": 200, "tester": 50},
            "pulled_anchors": 3
        })
    );
}

#[test]
fn workspace_lists_uninitialized_repos_and_prefixes_units() {
    let tmp = workspace_fixture();
    fs::write(
        tmp.path().join("backend/.stewards/units.toml"),
        "[[unit]]\nname = \"core\"\npurpose = \"the server\"\npaths = [\"src/**\"]\n",
    )
    .unwrap();
    let (code, v) = run(tmp.path(), &["status"]);
    assert_eq!(code, 0);
    assert_eq!(v["uninitialized"], serde_json::json!(["frontend"]));
    assert_eq!(unit(&v, "api-server/core")["freshness"], "unreviewed");
    assert_eq!(v["units"].as_array().unwrap().len(), 1);
}

#[test]
fn human_prints_a_table() {
    let repo = fixture("single");
    let out = stewards(repo.path(), &["status", "--human"]);
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.starts_with("| unit | freshness |"), "{text}");
    assert!(text.contains("| store | unreviewed | 1/2 |"), "{text}");
}

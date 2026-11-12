#[allow(dead_code)] // each test crate uses a subset of the helpers
mod common;

use std::fs;
use std::path::Path;

use common::{fixture, git, stewards};
use serde_json::Value;
use tempfile::TempDir;

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
    let mut text = fs::read_to_string(&file).unwrap();
    text.push_str("# simplified: no index\n# simplified: no locking\n");
    fs::write(&file, text).unwrap();
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
    let mut text = fs::read_to_string(&file).unwrap();
    text.push_str("\n# changed\n");
    fs::write(&file, text).unwrap();
    let (code, v) = run(repo.path(), &["status"]);
    assert_eq!(code, 0);
    assert_eq!(unit(&v, "api")["freshness"], "stale");
}

#[test]
fn bug_gap_is_counted_as_a_bug_not_a_gap() {
    let repo = fixture("single");
    let card = repo.path().join(".stewards/units/api.toml");
    let mut text = fs::read_to_string(&card).unwrap();
    text.push_str("\n[[gap]]\nkind = \"bug\"\ntitle = \"create drops tags\"\nnote = \"\"\n");
    fs::write(&card, text).unwrap();
    let (_, v) = run(repo.path(), &["status"]);
    let api = unit(&v, "api");
    assert_eq!(
        (api["gaps"].as_u64(), api["bugs"].as_u64()),
        (Some(1), Some(1))
    );
}

#[test]
fn telemetry_flag_adds_a_null() {
    let repo = fixture("single");
    let (_, v) = run(repo.path(), &["status", "--telemetry"]);
    assert!(v["telemetry"].is_null());
    assert!(v.as_object().unwrap().contains_key("telemetry"));
}

#[test]
fn workspace_lists_uninitialized_repos_and_prefixes_units() {
    let tmp = TempDir::new().unwrap();
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/workspace");
    fs::copy(src.join("stewards.toml"), tmp.path().join("stewards.toml")).unwrap();
    for name in ["backend", "frontend"] {
        let dest = tmp.path().join(name);
        let status = std::process::Command::new("cp")
            .arg("-R")
            .arg(src.join(name))
            .arg(&dest)
            .status()
            .unwrap();
        assert!(status.success());
        git(&dest, &["init", "-q"]);
    }
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

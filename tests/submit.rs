#[allow(dead_code)] // each test crate uses a subset of the helpers
mod common;

use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use common::{edit, fixture};
use serde_json::{Value, json};
use stewards::brief::read_header;
use stewards::model::{Role, UnitCard};
use stewards::telemetry::rfc3339;
use tempfile::TempDir;

const CARD: &str = ".stewards/units/api.toml";
const ANSWER: &str = ".stewards/briefs/api-maintainer.answer.json";

/// A copy of the single fixture and an empty STEWARDS_HOME.
fn setup() -> (TempDir, TempDir) {
    (fixture("single"), TempDir::new().unwrap())
}

fn spawn(dir: &Path, home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_stewards"))
        .args(args)
        .current_dir(dir)
        .env("STEWARDS_HOME", home)
        .output()
        .unwrap()
}

fn run(dir: &Path, home: &Path, args: &[&str]) -> (i32, Value) {
    let out = spawn(dir, home, args);
    let v = serde_json::from_slice(&out.stdout).unwrap_or_else(|_| {
        panic!(
            "not JSON: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    });
    (out.status.code().unwrap(), v)
}

/// Writes the review brief for api with `next`.
fn review_job(dir: &Path, home: &Path) {
    let (code, v) = run(dir, home, &["next", "--task", "add a count endpoint"]);
    assert_eq!(code, 1, "{v}");
    assert!(dir.join(".stewards/briefs/api-maintainer.md").is_file());
}

/// Writes the explain brief for api with `brief`.
fn explain_job(dir: &Path, home: &Path) {
    let args = ["brief", "api", "--role", "maintainer", "--task", "how?"];
    assert_eq!(run(dir, home, &args).0, 0);
}

fn review(verdict: &str) -> Value {
    json!({"verdict": verdict, "findings": [], "card_patch": null, "tester_needed": false,
           "pulled_anchors": [], "notes": ""})
}

fn explain(explanation: &str) -> Value {
    json!({"explanation": explanation, "tasks": [], "card_patch": null,
           "pulled_anchors": [], "notes": ""})
}

fn submit_text(dir: &Path, home: &Path, text: &str) -> (i32, Value) {
    fs::write(dir.join(ANSWER), text).unwrap();
    run(
        dir,
        home,
        &["submit", "api", "--role", "maintainer", "--file", ANSWER],
    )
}

fn submit(dir: &Path, home: &Path, answer: &Value) -> (i32, Value) {
    submit_text(dir, home, &answer.to_string())
}

fn card(dir: &Path) -> UnitCard {
    toml::from_str(&fs::read_to_string(dir.join(CARD)).unwrap()).unwrap()
}

fn freshness(dir: &Path, home: &Path) -> String {
    let (_, v) = run(dir, home, &["status"]);
    let api = v["units"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["unit"] == "api");
    api.unwrap()["freshness"].as_str().unwrap().to_string()
}

/// Submits `answer` and asserts it is rejected with `code` naming `field`, writing nothing.
fn rejected(answer: &Value, code: &str, field: &str) {
    let (dir, home) = setup();
    review_job(dir.path(), home.path());
    let before = fs::read(dir.path().join(CARD)).unwrap();
    let (exit, v) = submit(dir.path(), home.path(), answer);
    assert_eq!(exit, 2, "{v}");
    assert_eq!(v["error"]["code"], code, "{v}");
    let fields: Vec<&str> = v["error"]["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["field"].as_str().unwrap())
        .collect();
    assert!(fields.contains(&field), "{v}");
    assert_eq!(fs::read(dir.path().join(CARD)).unwrap(), before);
    assert!(!dir.path().join(".stewards/telemetry").exists());
}

#[test]
fn approve_stamps_the_brief_snapshot_so_a_later_edit_stays_stale() {
    let (dir, home) = setup();
    review_job(dir.path(), home.path());
    edit(
        &dir.path().join("src/api/items.py"),
        "router = []",
        "router = [1]",
    );
    let (code, v) = submit(dir.path(), home.path(), &review("approve"));
    assert_eq!(code, 0, "{v}");
    let header = read_header(&dir.path().join(".stewards/briefs/api-maintainer.md")).unwrap();
    assert_eq!(card(dir.path()).reviewed.unwrap().tree, header.snapshot);
    assert_eq!(
        v["summary"],
        format!(
            "api maintainer: approve; card unchanged; stamped {}",
            &header.snapshot[..7]
        )
    );
    assert_eq!(freshness(dir.path(), home.path()), "stale");
}

#[test]
fn approve_after_an_edit_made_before_the_brief_is_fresh() {
    let (dir, home) = setup();
    edit(
        &dir.path().join("src/api/items.py"),
        "router = []",
        "router = [1]",
    );
    review_job(dir.path(), home.path());
    assert_eq!(submit(dir.path(), home.path(), &review("approve")).0, 0);
    assert_eq!(freshness(dir.path(), home.path()), "fresh");
}

#[test]
fn approve_with_a_finding_is_rejected_and_writes_nothing() {
    let mut a = review("approve");
    a["findings"] = json!([{"file": "src/api/items.py", "line": 4, "rung": "one_line",
                            "cut": "x", "replace_with": "y"}]);
    rejected(&a, "answer_parse_failed", "verdict");
}

#[test]
fn changes_does_not_stamp_and_names_the_developer_brief() {
    let (dir, home) = setup();
    review_job(dir.path(), home.path());
    let before = fs::read(dir.path().join(CARD)).unwrap();
    let mut a = review("changes");
    a["findings"] = json!([{"file": "src/api/items.py", "line": 4, "rung": "one_line",
                            "cut": "x", "replace_with": "y"}]);
    let (code, v) = submit(dir.path(), home.path(), &a);
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        v["summary"],
        "api maintainer: changes; 1 finding (one_line src/api/items.py:4); card unchanged; \
         next: stewards brief api --role developer"
    );
    assert_eq!(fs::read(dir.path().join(CARD)).unwrap(), before);
    assert_eq!(freshness(dir.path(), home.path()), "unreviewed");
}

#[test]
fn explain_prints_the_explanation_truncated_and_does_not_stamp() {
    let (dir, home) = setup();
    explain_job(dir.path(), home.path());
    let long = "é".repeat(2500);
    let (code, v) = submit(dir.path(), home.path(), &explain(&long));
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        v["summary"],
        "api maintainer: explained; card unchanged; 0 tasks"
    );
    assert_eq!(v["explanation"], "é".repeat(2000));
    assert!(v["tester_brief"].is_null());
    assert_eq!(freshness(dir.path(), home.path()), "unreviewed");

    let out = spawn(
        dir.path(),
        home.path(),
        &[
            "submit",
            "api",
            "--role",
            "maintainer",
            "--file",
            ANSWER,
            "--human",
        ],
    );
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        format!(
            "api maintainer: explained; card unchanged; 0 tasks\n{}\n",
            "é".repeat(2000)
        )
    );
}

#[test]
fn patch_adding_an_interface_keeps_the_rest_of_the_card() {
    let (dir, home) = setup();
    assert_eq!(run(dir.path(), home.path(), &["stamp", "api"]).0, 0);
    let before = card(dir.path());
    explain_job(dir.path(), home.path());
    let mut a = explain("short");
    a["card_patch"] = json!({"interface": [
        {"id": "create-item", "anchor": "src/api/items.py::create_item",
         "contract": "POST /items; 201 with body; 422 on invalid"},
        {"id": "list-items", "anchor": "src/api/items.py::list_items", "contract": "GET /items"}]});
    let (code, v) = submit(dir.path(), home.path(), &a);
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        v["summary"],
        "api maintainer: explained; card: +1 interface; 0 tasks"
    );
    let after = card(dir.path());
    assert_eq!(after.interface.len(), 2);
    assert_eq!(after.interface[1].id, "list-items");
    assert_eq!(after.anchors, before.anchors);
    assert_eq!(after.gaps.len(), 1);
    assert_eq!(after.gaps[0].title, before.gaps[0].title);
    assert_eq!(after.doc.unwrap().overview, before.doc.unwrap().overview);
    assert_eq!(after.reviewed.unwrap().tree, before.reviewed.unwrap().tree);
}

#[test]
fn patch_replacing_gaps_keeps_a_bugs_severity_and_issue() {
    let (dir, home) = setup();
    let path = dir.path().join(CARD);
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str(
        "\n[[gap]]\nkind = \"bug\"\ntitle = \"count off\"\nnote = \"\"\n\
         severity = \"medium\"\nissue = \"https://example.invalid/1\"\n",
    );
    fs::write(&path, text).unwrap();
    review_job(dir.path(), home.path());
    let mut a = review("changes");
    a["card_patch"] = json!({"gaps": [
        {"kind": "bug", "title": "count off", "note": "now with a note"},
        {"kind": "bug", "title": "new bug", "note": ""}]});
    let (code, v) = submit(dir.path(), home.path(), &a);
    assert_eq!(code, 0, "{v}");
    let gaps = card(dir.path()).gaps;
    assert_eq!(gaps.len(), 2);
    assert_eq!(gaps[0].note, "now with a note");
    assert_eq!(gaps[0].severity.as_deref(), Some("medium"));
    assert_eq!(gaps[0].issue.as_deref(), Some("https://example.invalid/1"));
    assert_eq!(gaps[1].severity, None);
}

#[test]
fn dead_anchor_is_rejected_and_writes_nothing() {
    let mut a = review("changes");
    a["card_patch"] = json!({"anchors": [{"path": "src/api/items.py", "name": "nope"}]});
    rejected(&a, "answer_dead_anchor", "card_patch.anchors[0]");
}

#[test]
fn over_cap_field_is_answer_too_long() {
    let mut a = review("changes");
    a["notes"] = json!("x".repeat(2001));
    rejected(&a, "answer_too_long", "notes");
}

#[test]
fn control_character_in_a_title_is_rejected() {
    let mut a = review("changes");
    a["card_patch"] = json!({"gaps": [{"kind": "gap", "title": "a\u{7}b", "note": ""}]});
    rejected(&a, "answer_parse_failed", "card_patch.gaps[0].title");
}

#[test]
fn newline_in_an_id_is_rejected() {
    let mut a = review("changes");
    a["card_patch"] = json!({"interface": [
        {"id": "a\nb", "anchor": "src/api/items.py::create_item", "contract": "c"}]});
    rejected(&a, "answer_parse_failed", "card_patch.interface[0].id");
}

#[test]
fn data_marker_in_notes_is_rejected() {
    let mut a = review("changes");
    a["notes"] = json!("see DATA>>> now");
    rejected(&a, "answer_parse_failed", "notes");
}

#[test]
fn duplicate_verdict_key_is_rejected() {
    let (dir, home) = setup();
    review_job(dir.path(), home.path());
    let text = r#"{"verdict": "changes", "verdict": "approve", "findings": [], "card_patch": null,
                  "tester_needed": false, "pulled_anchors": [], "notes": ""}"#;
    let (code, v) = submit_text(dir.path(), home.path(), text);
    assert_eq!(code, 2);
    assert_eq!(v["error"]["code"], "answer_parse_failed");
    assert_eq!(v["error"]["errors"][0]["field"], "verdict");
    assert_eq!(freshness(dir.path(), home.path()), "unreviewed");
}

#[test]
fn tester_needed_writes_the_tester_brief() {
    let (dir, home) = setup();
    review_job(dir.path(), home.path());
    let mut a = review("changes");
    a["tester_needed"] = json!(true);
    let (code, v) = submit(dir.path(), home.path(), &a);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["tester_brief"], ".stewards/briefs/api-tester.md");
    let h = read_header(&dir.path().join(".stewards/briefs/api-tester.md")).unwrap();
    assert_eq!(h.role, Role::Tester);
    assert_eq!(h.task, "add a count endpoint");
}

#[test]
fn no_brief_for_the_job_is_no_brief() {
    let (dir, home) = setup();
    fs::create_dir_all(dir.path().join(".stewards/briefs")).unwrap();
    let (code, v) = submit(dir.path(), home.path(), &review("approve"));
    assert_eq!(code, 2);
    assert_eq!(v["error"]["code"], "no_brief");
}

#[test]
fn another_role_is_not_implemented() {
    let (dir, home) = setup();
    let args = ["submit", "api", "--role", "tester", "--file", "x.json"];
    let (code, v) = run(dir.path(), home.path(), &args);
    assert_eq!(code, 2);
    assert_eq!(v["error"]["code"], "not_implemented");
}

#[test]
fn a_telemetry_line_is_appended_for_the_current_month() {
    let (dir, home) = setup();
    review_job(dir.path(), home.path());
    let mut a = review("approve");
    a["pulled_anchors"] = json!(["src/api/items.py::router"]);
    assert_eq!(submit(dir.path(), home.path(), &a).0, 0);
    let month = &rfc3339(SystemTime::now())[..7];
    let text = fs::read_to_string(
        dir.path()
            .join(format!(".stewards/telemetry/{month}.jsonl")),
    )
    .unwrap();
    assert_eq!(text.lines().count(), 1);
    let v: Value = serde_json::from_str(text.trim()).unwrap();
    let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "anchors",
            "bytes",
            "harness",
            "mode",
            "pulled_anchors",
            "role",
            "task",
            "timestamp",
            "unit"
        ]
    );
    assert_eq!(v["role"], "maintainer");
    assert_eq!(v["mode"], "review");
    assert_eq!(v["task"], "add a count endpoint");
    assert_eq!(v["pulled_anchors"], json!(["src/api/items.py::router"]));
    let ts = v["timestamp"].as_str().unwrap();
    let shape: String = ts
        .chars()
        .map(|c| if c.is_ascii_digit() { '9' } else { c })
        .collect();
    assert_eq!(shape, "9999-99-99T99:99:99Z");
    assert!(ts.starts_with(month));
}

#[test]
fn status_telemetry_summarises_two_submits() {
    let (dir, home) = setup();
    review_job(dir.path(), home.path());
    let review_bytes = read_header(&dir.path().join(".stewards/briefs/api-maintainer.md"))
        .unwrap()
        .bytes;
    let mut a = review("changes");
    a["pulled_anchors"] = json!(["src/api/items.py::router"]);
    assert_eq!(submit(dir.path(), home.path(), &a).0, 0);
    explain_job(dir.path(), home.path());
    let explain_bytes = read_header(&dir.path().join(".stewards/briefs/api-maintainer.md"))
        .unwrap()
        .bytes;
    let mut a = explain("e");
    a["pulled_anchors"] = json!(["src/api/items.py::router", "src/api/items.py::list_items"]);
    assert_eq!(submit(dir.path(), home.path(), &a).0, 0);
    let (_, v) = run(dir.path(), home.path(), &["status", "--telemetry"]);
    assert_eq!(
        v["telemetry"],
        json!({"submits": {"maintainer": 2},
               "mean_pack_bytes": {"maintainer": (review_bytes + explain_bytes) / 2},
               "pulled_anchors": 3})
    );
}

#[cfg(unix)]
#[test]
fn a_symlinked_card_is_not_written_through() {
    let (dir, home) = setup();
    let outside = TempDir::new().unwrap();
    let target = outside.path().join("api.toml");
    fs::copy(dir.path().join(CARD), &target).unwrap();
    fs::remove_file(dir.path().join(CARD)).unwrap();
    std::os::unix::fs::symlink(&target, dir.path().join(CARD)).unwrap();
    let before = fs::read(&target).unwrap();
    explain_job(dir.path(), home.path());
    let mut a = explain("e");
    a["card_patch"] = json!({"anchors": []});
    let (code, v) = submit(dir.path(), home.path(), &a);
    assert_eq!(code, 2);
    assert_eq!(v["error"]["code"], "unsafe_path");
    assert_eq!(fs::read(&target).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn a_symlinked_telemetry_dir_gets_no_file() {
    let (dir, home) = setup();
    let outside = TempDir::new().unwrap();
    std::os::unix::fs::symlink(outside.path(), dir.path().join(".stewards/telemetry")).unwrap();
    explain_job(dir.path(), home.path());
    let (code, v) = submit(dir.path(), home.path(), &explain("e"));
    assert_eq!(code, 0, "{v}");
    assert!(v["warning"].as_str().unwrap().contains("symlink"), "{v}");
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
}

#[test]
fn rfc3339_converts_known_timestamps() {
    let at = |secs| rfc3339(UNIX_EPOCH + Duration::from_secs(secs));
    assert_eq!(at(0), "1970-01-01T00:00:00Z");
    assert_eq!(at(951_782_400), "2000-02-29T00:00:00Z");
    assert_eq!(at(1_709_208_000), "2024-02-29T12:00:00Z");
    assert_eq!(at(1_700_000_000), "2023-11-14T22:13:20Z");
    assert_eq!(at(4_102_444_799), "2099-12-31T23:59:59Z");
}

// `validate --report`: record a bug, run its repro, brief a validator.
#[allow(dead_code)] // each test crate uses a subset of the helpers
mod common;

use std::fs;
use std::path::Path;

use common::{append, edit, fixture, run};
use serde_json::{Value, json};
use stewards::model::UnitCard;

const CARD: &str = ".stewards/units/api.toml";

/// The single fixture with `sh` allowlisted and three repro scripts: `fail.sh` exits 1, `pass.sh`
/// exits 0, `mark.sh` creates `marker` and exits 1.
fn setup() -> tempfile::TempDir {
    let dir = fixture("single");
    let d = dir.path();
    edit(
        &d.join(".stewards/repo.toml"),
        "test = [\"pytest\"]",
        "test = [\"sh\"]",
    );
    fs::write(d.join("fail.sh"), "exit 1\n").unwrap();
    fs::write(d.join("pass.sh"), "exit 0\n").unwrap();
    fs::write(d.join("mark.sh"), "touch marker\nexit 1\n").unwrap();
    dir
}

fn card(dir: &Path) -> UnitCard {
    toml::from_str(&fs::read_to_string(dir.join(CARD)).unwrap()).unwrap()
}

fn report(unit: &str, title: &str, repro: &str, interface: Value) -> Value {
    json!({"unit": unit, "title": title, "repro": repro, "evidence": "expected 2, got 3",
           "interface": interface})
}

fn validate(dir: &Path, r: &Value) -> (i32, Value) {
    fs::write(dir.join("report.json"), r.to_string()).unwrap();
    run(dir, &["validate", "--report", "report.json"])
}

fn validator_briefs(dir: &Path) -> Vec<String> {
    common::job_files(dir, "api-validator", ".md")
}

/// A report that must be rejected: exit 2, `code`, the card as it was, no validator brief.
fn rejected(dir: &Path, r: &Value, code: &str) {
    let before = fs::read(dir.join(CARD)).unwrap();
    let (exit, v) = validate(dir, r);
    assert_eq!(exit, 2, "{v}");
    assert_eq!(v["error"]["code"], code, "{v}");
    assert!(
        v["error"]["fix"].as_str().unwrap().contains("\"unit\""),
        "{v}"
    );
    assert_eq!(fs::read(dir.join(CARD)).unwrap(), before);
    assert!(validator_briefs(dir).is_empty());
}

#[test]
fn a_failing_repro_is_recorded_without_severity_and_proven_in_the_brief() {
    let dir = setup();
    let d = dir.path();
    let (code, v) = validate(
        d,
        &report(
            "api",
            "count off by one",
            "sh fail.sh",
            json!("create-item"),
        ),
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        v["recorded"],
        json!({"unit": "api", "title": "count off by one"})
    );
    assert_eq!(v["reproduced"], true);
    assert_eq!(v["repro_result"]["exit_code"], 1);
    assert!(v["then"].as_str().unwrap().contains("stewards review"));
    let gap = card(d)
        .gaps
        .into_iter()
        .find(|g| g.title == "count off by one")
        .unwrap();
    assert_eq!(gap.severity, None);
    assert_eq!(gap.anchor.as_deref(), Some("src/api/items.py::create_item"));
    assert_eq!(gap.repro.as_deref(), Some("sh fail.sh"));
    let brief = fs::read_to_string(v["brief_path"].as_str().unwrap()).unwrap();
    assert!(brief.contains("\"reproduced\": true"), "{brief}");
    assert_eq!(
        v["handoff"],
        format!(
            "Read `{}` and follow it.",
            v["brief_path"].as_str().unwrap()
        )
    );
}

#[test]
fn a_passing_repro_is_not_reproduced() {
    let dir = setup();
    let (code, v) = validate(dir.path(), &report("api", "b", "sh pass.sh", json!(null)));
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["reproduced"], false);
    assert_eq!(v["repro_result"]["reason"], "the command exited 0");
    let brief = fs::read_to_string(v["brief_path"].as_str().unwrap()).unwrap();
    assert!(brief.contains("\"reproduced\": false"), "{brief}");
}

#[test]
fn bad_reports_are_rejected_and_write_nothing() {
    let dir = setup();
    let d = dir.path();
    rejected(
        d,
        &report("api", "b", "rm -rf x", json!(null)),
        "command_not_allowed",
    );
    rejected(
        d,
        &report("api", "b", "sh fail.sh", json!("no-such-id")),
        "answer_parse_failed",
    );
    rejected(
        d,
        &report("api", &"t".repeat(2000), "sh fail.sh", json!(null)),
        "answer_too_long",
    );
    let mut extra = report("api", "b", "sh fail.sh", json!(null));
    extra["severity"] = json!("high");
    rejected(d, &extra, "answer_parse_failed");
    let (exit, v) = validate(d, &report("nope", "b", "sh fail.sh", json!(null)));
    assert_eq!(
        (exit, &v["error"]["code"]),
        (2, &json!("unknown_unit")),
        "{v}"
    );
}

#[test]
fn a_probable_duplicate_is_not_added_and_writes_no_brief() {
    let dir = setup();
    let d = dir.path();
    let (_, first) = validate(d, &report("api", "one", "sh fail.sh", json!("create-item")));
    let brief = first["brief_path"].as_str().unwrap();
    fs::remove_file(brief).unwrap();
    let (code, v) = validate(d, &report("api", "two", "sh fail.sh", json!("create-item")));
    assert_eq!(code, 0, "{v}");
    assert!(
        v["duplicate"]
            .as_str()
            .unwrap()
            .contains("possible duplicate of \"one\""),
        "{v}"
    );
    assert!(
        v["brief_path"].is_null() && v["repro_result"].is_null(),
        "{v}"
    );
    assert!(validator_briefs(d).is_empty());
    assert!(card(d).gaps.iter().all(|g| g.title != "two"));
}

#[test]
fn the_same_title_updates_the_bug_and_keeps_its_severity() {
    let dir = setup();
    let d = dir.path();
    append(
        &d.join(CARD),
        "\n[[gap]]\nkind = \"bug\"\ntitle = \"count off by one\"\nnote = \"old\"\nrepro = \"sh pass.sh\"\nseverity = \"high\"\n",
    );
    let (code, v) = validate(
        d,
        &report("api", "count off by one", "sh fail.sh", json!(null)),
    );
    assert_eq!(code, 0, "{v}");
    let gaps = card(d).gaps;
    let bugs: Vec<_> = gaps
        .iter()
        .filter(|g| g.title == "count off by one")
        .collect();
    assert_eq!(bugs.len(), 1);
    assert_eq!(bugs[0].severity.as_deref(), Some("high"));
    assert_eq!(bugs[0].note, "expected 2, got 3");
    assert_eq!(bugs[0].repro.as_deref(), Some("sh fail.sh"));
}

#[test]
fn a_validator_submit_against_the_validate_brief_sets_severity() {
    let dir = setup();
    let d = dir.path();
    let (code, v) = validate(
        d,
        &report(
            "api",
            "count off by one",
            "sh fail.sh",
            json!("create-item"),
        ),
    );
    assert_eq!(code, 0, "{v}");
    let brief = common::brief(d, "api-validator");
    let answer = brief.replace(".md", ".answer.json");
    let a = json!({"real": true, "severity": "medium", "reason": "wrong count",
                   "evidence": ["exit 1"], "pulled_anchors": [], "notes": ""});
    fs::write(d.join(&answer), a.to_string()).unwrap();
    let (code, v) = run(
        d,
        &["submit", "api", "--role", "validator", "--file", &answer],
    );
    assert_eq!(code, 0, "{v}");
    let gap = card(d)
        .gaps
        .into_iter()
        .find(|g| g.title == "count off by one")
        .unwrap();
    assert_eq!(gap.severity.as_deref(), Some("medium"));
}

#[test]
fn next_roles_runs_the_repro_into_the_brief_and_no_brief_runs_nothing() {
    let dir = setup();
    let d = dir.path();
    append(
        &d.join(CARD),
        "\n[[gap]]\nkind = \"bug\"\ntitle = \"marks\"\nnote = \"n\"\nrepro = \"sh mark.sh\"\n",
    );
    let (_, v) = run(d, &["next", "--roles", "--no-brief", "--task", "t"]);
    assert!(v["jobs"].is_array(), "{v}");
    assert!(!d.join("marker").exists());
    let (code, v) = run(d, &["pack", "api", "--role", "validator", "--task", "t"]);
    assert_eq!(code, 0, "{v}");
    assert!(v["repro_result"].is_null());
    assert!(!d.join("marker").exists());
    run(d, &["next", "--roles", "--task", "t"]);
    assert!(d.join("marker").exists());
    let brief = fs::read_to_string(d.join(common::brief(d, "api-validator"))).unwrap();
    assert!(brief.contains("\"reproduced\": true"), "{brief}");
}

#[test]
fn validate_with_neither_flag_is_usage() {
    let dir = setup();
    let (code, v) = run(dir.path(), &["validate"]);
    assert_eq!(code, 2);
    assert_eq!(v["error"]["code"], "usage");
}

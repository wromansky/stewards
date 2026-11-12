// `submit` for the tester, documenter, validator, and architect.
#[allow(dead_code)] // each test crate uses a subset of the helpers
mod common;

use std::fs;
use std::path::Path;

use common::{append, edit, fixture, run, stamp_units, stewards};
use serde_json::{Value, json};
use stewards::model::{GapKind, TestCard, UnitCard, load_repo};

const CARD: &str = ".stewards/units/api.toml";
const TESTS: &str = ".stewards/units/api.test.toml";
const REPO_TOML: &str = ".stewards/repo.toml";
const BUG: &str = "\n[[gap]]\nkind = \"bug\"\ntitle = \"count off by one\"\nnote = \"expected 2, got 3\"\nrepro = \"pytest tests/api -k count\"\n";

/// `api-<role>`, or `architect`.
fn stem(role: &str) -> String {
    if role == "architect" {
        role.into()
    } else {
        format!("api-{role}")
    }
}

/// Writes the role's brief with the built `brief` command.
fn job(dir: &Path, role: &str) {
    let mut args = vec!["brief"];
    if role != "architect" {
        args.push("api");
    }
    args.extend(["--role", role, "--task", "add a count endpoint"]);
    let (code, v) = run(dir, &args);
    assert_eq!(code, 0, "{v}");
}

fn submit(dir: &Path, role: &str, answer: &Value) -> (i32, Value) {
    let file = format!(".stewards/briefs/{}.answer.json", stem(role));
    fs::write(dir.join(&file), answer.to_string()).unwrap();
    let mut args = vec!["submit"];
    if role != "architect" {
        args.push("api");
    }
    args.extend(["--role", role, "--file", &file]);
    run(dir, &args)
}

/// Writes the job's brief, submits `answer`, and asserts success and the brief's retirement.
fn submit_ok(dir: &Path, role: &str, answer: &Value) -> Value {
    job(dir, role);
    let (code, v) = submit(dir, role, answer);
    assert_eq!(code, 0, "{v}");
    let brief = dir.join(format!(".stewards/briefs/{}.md", stem(role)));
    assert!(!brief.exists(), "the {role} brief is retired");
    v
}

/// Writes the job's brief, submits `answer`, and asserts the error `code` naming `field` with
/// `file` unchanged and the brief kept.
fn submit_rejected(dir: &Path, role: &str, answer: &Value, code: &str, field: &str, file: &str) {
    job(dir, role);
    let before = fs::read(dir.join(file)).unwrap();
    let (exit, v) = submit(dir, role, answer);
    assert_eq!(exit, 2, "{v}");
    assert_eq!(v["error"]["code"], code, "{v}");
    let fields: Vec<&str> = v["error"]["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["field"].as_str().unwrap())
        .collect();
    assert!(fields.contains(&field), "{v}");
    assert_eq!(fs::read(dir.join(file)).unwrap(), before);
    assert!(
        dir.join(format!(".stewards/briefs/{}.md", stem(role)))
            .is_file()
    );
}

fn card(dir: &Path) -> UnitCard {
    toml::from_str(&fs::read_to_string(dir.join(CARD)).unwrap()).unwrap()
}

fn test_card(dir: &Path) -> TestCard {
    toml::from_str(&fs::read_to_string(dir.join(TESTS)).unwrap()).unwrap()
}

fn scenario(id: &str, interface: &str, command: Option<&str>) -> Value {
    json!({"id": id, "interface": interface, "given": "two items", "expect": "count is 2",
           "level": "integration", "command": command})
}

fn bug(title: &str, repro: &str, evidence: &str) -> Value {
    json!({"title": title, "repro": repro, "evidence": evidence, "interface": "create-item"})
}

fn tester(scenarios: Value, bugs: Value) -> Value {
    json!({"scenario_patch": scenarios, "results": [], "bugs": bugs, "tests_written": [],
           "pulled_anchors": [], "notes": ""})
}

fn documenter(overview: &str) -> Value {
    json!({"doc": {"overview": overview, "how_it_works": "Requests enter at create_item.",
                   "how_to_change": "Start at create_item; keep 201 on success."},
           "pulled_anchors": [], "notes": ""})
}

fn validator(real: bool) -> Value {
    json!({"real": real, "severity": "medium", "reason": "wrong count, no error",
           "evidence": ["expected 2, got 3"], "pulled_anchors": [], "notes": ""})
}

fn architect(flows: Value, decisions: Value, proposals: Value, plan: Value) -> Value {
    json!({"plan": plan, "flows": flows, "decisions": decisions, "unit_proposals": proposals,
           "explanation": "api then store.", "pulled_anchors": [], "notes": ""})
}

/// The jobs `next` lists for api, as role names.
fn api_jobs(dir: &Path) -> Vec<String> {
    let (_, v) = run(dir, &["next", "--no-brief", "--task", "t"]);
    v["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|j| j["unit"] == "api")
        .map(|j| j["role"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn tester_upserts_a_new_scenario_and_updates_one_by_id() {
    let dir = fixture("single");
    let d = dir.path();
    let mut changed = scenario("create-ok", "create-item", None);
    changed["expect"] = json!("201 with the id");
    let a = tester(
        json!([
            changed,
            scenario("count-ok", "create-item", Some("pytest tests/api"))
        ]),
        json!([]),
    );
    let v = submit_ok(d, "tester", &a);
    assert_eq!(
        v["summary"],
        "api tester: 2 scenarios upserted (create-ok, count-ok); 0/0 pass; 0 bugs; 0 tests written"
    );
    let t = test_card(d);
    assert_eq!(t.primary_paths, ["create-item"]);
    assert_eq!(t.test_command.as_deref(), Some("pytest tests/api"));
    let ids: Vec<&str> = t.scenarios.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["create-ok", "count-ok"]);
    assert_eq!(t.scenarios[0].expect, "201 with the id");
}

#[test]
fn tester_scenario_with_an_unknown_interface_is_rejected_and_writes_nothing() {
    let dir = fixture("single");
    let a = tester(json!([scenario("x", "no-such-id", None)]), json!([]));
    let field = "scenario_patch[0].interface";
    submit_rejected(
        dir.path(),
        "tester",
        &a,
        "answer_parse_failed",
        field,
        TESTS,
    );
    assert_eq!(
        fs::read(dir.path().join(CARD)).unwrap(),
        fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/single")
                .join(CARD)
        )
        .unwrap()
    );
}

#[test]
fn tester_bug_becomes_a_bug_gap_with_repro_and_note() {
    let dir = fixture("single");
    let d = dir.path();
    let mut a = tester(
        json!([scenario("count-ok", "create-item", None)]),
        json!([bug(
            "count off by one",
            "pytest tests/api -k count",
            "expected 2, got 3"
        )]),
    );
    a["results"] = json!([{"scenario": "count-ok", "pass": true, "output": "ok"},
                         {"scenario": "create-ok", "pass": true, "output": "ok"},
                         {"scenario": "a", "pass": true, "output": "ok"},
                         {"scenario": "b", "pass": true, "output": "ok"}]);
    a["tests_written"] = json!(["tests/api/test_count.py", "tests/api/test_items.py"]);
    let v = submit_ok(d, "tester", &a);
    assert_eq!(
        v["summary"],
        "api tester: 1 scenario upserted (count-ok); 4/4 pass; 1 bug (count off by one); 2 tests written"
    );
    let gap = card(d)
        .gaps
        .into_iter()
        .find(|g| g.title == "count off by one");
    let gap = gap.unwrap();
    assert_eq!(gap.kind, GapKind::Bug);
    assert_eq!(gap.repro.as_deref(), Some("pytest tests/api -k count"));
    assert_eq!(gap.note, "expected 2, got 3");
    assert!(gap.severity.is_none());
}

#[test]
fn tester_repeated_bug_title_updates_the_gap_and_keeps_severity_and_issue() {
    let dir = fixture("single");
    let d = dir.path();
    append(&d.join(CARD), BUG);
    edit(
        &d.join(CARD),
        "repro = \"pytest tests/api -k count\"\n",
        "repro = \"pytest tests/api -k count\"\nseverity = \"low\"\nissue = \"https://example.invalid/1\"\n",
    );
    let a = tester(
        json!([]),
        json!([bug(
            "count off by one",
            "pytest tests/api -k count_two",
            "got 4"
        )]),
    );
    submit_ok(d, "tester", &a);
    let gaps: Vec<_> = card(d)
        .gaps
        .into_iter()
        .filter(|g| g.title == "count off by one")
        .collect();
    assert_eq!(gaps.len(), 1);
    assert_eq!(
        gaps[0].repro.as_deref(),
        Some("pytest tests/api -k count_two")
    );
    assert_eq!(gaps[0].note, "got 4");
    assert_eq!(gaps[0].severity.as_deref(), Some("low"));
    assert_eq!(gaps[0].issue.as_deref(), Some("https://example.invalid/1"));
}

#[test]
fn tester_repro_outside_the_allowlist_is_command_not_allowed() {
    let dir = fixture("single");
    let d = dir.path();
    for repro in ["make test", "pytest-x tests/x"] {
        let a = tester(json!([]), json!([bug("b", repro, "e")]));
        submit_rejected(
            d,
            "tester",
            &a,
            "command_not_allowed",
            "bugs[0].repro",
            CARD,
        );
    }
    let a = tester(json!([]), json!([bug("b", "pytest tests/x", "e")]));
    submit_ok(d, "tester", &a);
}

#[test]
fn tester_sets_its_mark_and_keeps_fingerprint_and_tree() {
    let dir = fixture("single");
    let d = dir.path();
    stamp_units(d, &["api"]);
    let before = card(d).reviewed.unwrap();
    assert!(before.tester.is_none());
    submit_ok(d, "tester", &tester(json!([]), json!([])));
    let after = card(d).reviewed.unwrap();
    assert!(after.tester.is_some());
    assert_eq!(after.fingerprint, before.fingerprint);
    assert_eq!(after.tree, before.tree);
}

#[test]
fn after_a_tester_writes_a_new_test_next_lists_the_maintainer_not_the_tester() {
    let dir = fixture("single");
    let d = dir.path();
    stamp_units(d, &["api"]);
    job(d, "tester");
    fs::write(
        d.join("tests/api/test_count.py"),
        "def test_count():\n    pass\n",
    )
    .unwrap();
    let mut a = tester(json!([]), json!([]));
    a["tests_written"] = json!(["tests/api/test_count.py"]);
    let (code, v) = submit(d, "tester", &a);
    assert_eq!(code, 0, "{v}");
    let jobs = api_jobs(d);
    assert!(jobs.contains(&"maintainer".to_string()), "{jobs:?}");
    assert!(!jobs.contains(&"tester".to_string()), "{jobs:?}");
}

#[test]
fn tester_on_a_never_stamped_unit_leaves_reviewed_absent() {
    let dir = fixture("single");
    let d = dir.path();
    let a = tester(json!([]), json!([bug("b", "pytest tests/api", "e")]));
    submit_ok(d, "tester", &a);
    assert!(card(d).reviewed.is_none());
    assert!(
        !fs::read_to_string(d.join(CARD))
            .unwrap()
            .contains("[reviewed]")
    );
}

#[test]
fn documenter_writes_the_doc_renders_the_page_and_sets_its_mark() {
    let dir = fixture("single");
    let d = dir.path();
    stamp_units(d, &["api"]);
    edit(&d.join("src/api/items.py"), "router = []", "router = [1]");
    assert!(api_jobs(d).contains(&"documenter".to_string()));
    let v = submit_ok(d, "documenter", &documenter("Serves items over HTTP."));
    assert_eq!(
        v["summary"],
        "api documenter: doc updated; docs/units/api.md"
    );
    let doc = card(d).doc.unwrap();
    assert_eq!(doc.overview, "Serves items over HTTP.");
    assert_eq!(doc.how_it_works, "Requests enter at create_item.");
    assert_eq!(
        doc.how_to_change,
        "Start at create_item; keep 201 on success."
    );
    let page = fs::read_to_string(d.join("docs/units/api.md")).unwrap();
    assert!(page.contains("Serves items over HTTP."), "{page}");
    assert!(d.join("docs/units/README.md").is_file());
    assert!(card(d).reviewed.unwrap().documenter.is_some());
    assert!(!api_jobs(d).contains(&"documenter".to_string()));
}

#[test]
fn validator_real_sets_severity_and_keeps_repro_and_note() {
    let dir = fixture("single");
    let d = dir.path();
    append(&d.join(CARD), BUG);
    let v = submit_ok(d, "validator", &validator(true));
    assert_eq!(v["summary"], "api validator: count off by one: medium");
    let gap = card(d)
        .gaps
        .into_iter()
        .find(|g| g.title == "count off by one");
    let gap = gap.unwrap();
    assert_eq!(gap.severity.as_deref(), Some("medium"));
    assert_eq!(gap.repro.as_deref(), Some("pytest tests/api -k count"));
    assert_eq!(gap.note, "expected 2, got 3");
}

#[test]
fn validator_not_real_removes_the_gap() {
    let dir = fixture("single");
    let d = dir.path();
    append(&d.join(CARD), BUG);
    let v = submit_ok(d, "validator", &validator(false));
    assert_eq!(
        v["summary"],
        "api validator: count off by one: not real, removed"
    );
    let titles: Vec<String> = card(d).gaps.into_iter().map(|g| g.title).collect();
    assert_eq!(titles, ["no pagination on list"]);
}

#[test]
fn validator_whose_bug_is_no_longer_on_the_card_is_an_error() {
    let dir = fixture("single");
    let d = dir.path();
    append(&d.join(CARD), BUG);
    job(d, "validator");
    edit(&d.join(CARD), BUG, "\n");
    let before = fs::read(d.join(CARD)).unwrap();
    let (code, v) = submit(d, "validator", &validator(true));
    assert_eq!(code, 2, "{v}");
    assert_eq!(v["error"]["code"], "no_brief", "{v}");
    assert_eq!(fs::read(d.join(CARD)).unwrap(), before);
}

#[test]
fn architect_upserts_a_flow_and_a_decision_and_keeps_comments() {
    let dir = fixture("single");
    let d = dir.path();
    let comment = "# budgets are bytes, hand note  \n";
    edit(
        &d.join(REPO_TOML),
        "[budgets]\n",
        &format!("{comment}[budgets]\n"),
    );
    let flows = json!([
        {"name": "create-item", "units": ["api", "store"], "entry": "src/api/items.py::create_item",
         "check": "pytest tests/api -k create"},
        {"name": "list-items", "units": ["api"], "entry": "src/api/items.py::list_items"}
    ]);
    let decisions = json!([{"title": "api owns paging", "why": "store returns all rows"}]);
    let v = submit_ok(
        d,
        "architect",
        &architect(flows, decisions, json!([]), Value::Null),
    );
    assert_eq!(
        v["summary"],
        "architect: 2 flows upserted (create-item, list-items); 1 decision; 0 unit proposals"
    );
    assert_eq!(v["explanation"], "api then store.");
    let text = fs::read_to_string(d.join(REPO_TOML)).unwrap();
    assert!(text.contains(&format!("{comment}[budgets]\n")), "{text}");
    let repo = load_repo(d).unwrap().repo;
    let flows: Vec<(&str, Option<&str>)> = repo
        .flows
        .iter()
        .map(|f| (f.name.as_str(), f.check.as_deref()))
        .collect();
    assert_eq!(
        flows,
        [
            ("create-item", Some("pytest tests/api -k create")),
            ("list-items", None)
        ]
    );
    let titles: Vec<&str> = repo.decisions.iter().map(|d| d.title.as_str()).collect();
    assert_eq!(titles, ["store has no HTTP knowledge", "api owns paging"]);
}

#[test]
fn architect_flow_naming_an_unknown_unit_is_rejected() {
    let dir = fixture("single");
    let flows = json!([{"name": "f", "units": ["api", "nope"],
                        "entry": "src/api/items.py::create_item"}]);
    let a = architect(flows, json!([]), json!([]), Value::Null);
    let field = "flows[0].units[1]";
    submit_rejected(
        dir.path(),
        "architect",
        &a,
        "answer_parse_failed",
        field,
        REPO_TOML,
    );
}

#[test]
fn architect_on_an_inline_flow_array_is_invalid_toml_and_writes_nothing() {
    let dir = fixture("single");
    let d = dir.path();
    edit(
        &d.join(REPO_TOML),
        "[[flow]]\nname = \"create-item\"\nunits = [\"api\", \"store\"]\n\
         entry = \"src/api/items.py::create_item\"\ncheck = \"pytest tests -k create_item\"\n",
        "",
    );
    edit(
        &d.join(REPO_TOML),
        "schema = 1\n",
        "schema = 1\nflow = [{ name = \"create-item\", units = [\"api\"], \
         entry = \"src/api/items.py::create_item\" }]\n",
    );
    job(d, "architect");
    let before = fs::read(d.join(REPO_TOML)).unwrap();
    let flows = json!([{"name": "list-items", "units": ["api"],
                        "entry": "src/api/items.py::list_items"}]);
    let a = architect(flows, json!([]), json!([]), Value::Null);
    let (code, v) = submit(d, "architect", &a);
    assert_eq!(
        (code, &v["error"]["code"]),
        (2, &"invalid_toml".into()),
        "{v}"
    );
    let message = v["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("repo.toml") && message.contains("[[flow]]"),
        "{message}"
    );
    assert_eq!(fs::read(d.join(REPO_TOML)).unwrap(), before);
    assert!(d.join(".stewards/briefs/architect.md").is_file());
}

#[test]
fn architect_unit_proposals_are_printed_and_not_applied() {
    let dir = fixture("single");
    let d = dir.path();
    let units = fs::read(d.join(".stewards/units.toml")).unwrap();
    let proposals = json!([{"name": "auth", "purpose": "login", "paths": ["src/auth/**"]},
                           {"name": "jobs", "purpose": "workers", "paths": ["src/jobs/**"]}]);
    let v = submit_ok(
        d,
        "architect",
        &architect(json!([]), json!([]), proposals.clone(), Value::Null),
    );
    assert_eq!(v["unit_proposals"], proposals);
    assert_eq!(
        v["summary"],
        "architect: 0 flows upserted; 0 decisions; 2 unit proposals"
    );
    assert_eq!(fs::read(d.join(".stewards/units.toml")).unwrap(), units);
}

#[test]
fn architect_plan_is_accepted_and_reported_as_not_stored() {
    let dir = fixture("single");
    let d = dir.path();
    let plan = json!({"title": "add count", "steps": [
        {"unit": "api", "change": "add count", "interfaces": ["create-item"], "check": "pytest tests/api"}]});
    let v = submit_ok(
        d,
        "architect",
        &architect(json!([]), json!([]), json!([]), plan),
    );
    assert_eq!(
        v["summary"],
        "architect: 0 flows upserted; 0 decisions; 0 unit proposals; plan not stored yet"
    );
    assert!(!d.join(".stewards/plans").exists());
}

#[test]
fn no_role_changes_fingerprint_or_tree() {
    let dir = fixture("single");
    let d = dir.path();
    append(&d.join(CARD), BUG);
    stamp_units(d, &["api"]);
    let stamp = card(d).reviewed.unwrap();
    let answers = [
        (
            "tester",
            tester(json!([]), json!([bug("b", "pytest tests/api", "e")])),
        ),
        ("documenter", documenter("Serves items.")),
        ("validator", validator(true)),
        (
            "architect",
            architect(
                json!([]),
                json!([{"title": "t", "why": "w"}]),
                json!([]),
                Value::Null,
            ),
        ),
    ];
    for (role, answer) in answers {
        submit_ok(d, role, &answer);
        let now = card(d).reviewed.unwrap();
        assert_eq!(now.fingerprint, stamp.fingerprint, "{role}");
        assert_eq!(now.tree, stamp.tree, "{role}");
    }
}

#[test]
fn developer_submit_is_still_not_implemented() {
    let dir = fixture("single");
    let out = stewards(
        dir.path(),
        &["submit", "api", "--role", "developer", "--file", "x.json"],
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(v["error"]["code"], "not_implemented");
}

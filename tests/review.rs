// The per-change review: `next`'s one reviewer job, its brief, and `submit --role reviewer`.
#[allow(dead_code)] // each test crate uses a subset of the helpers
mod common;

use std::fs;
use std::path::Path;

use common::{append, edit, fixture, run, stamp_units};
use serde_json::{Value, json};
use stewards::brief::read_header;
use stewards::model::{TestCard, UnitCard, load_repo};
use stewards::pack::REVIEW_BUDGET;
use stewards::submit::record;
use stewards::workspace::Repo;

const BRIEF: &str = ".stewards/briefs/review.md";
const ANSWER: &str = ".stewards/briefs/review.answer.json";

/// The single fixture with both units stamped, then `create_item` (api) and `count_items`
/// (store) edited: both interfaces changed.
fn changed_both() -> tempfile::TempDir {
    let dir = fixture("single");
    let d = dir.path();
    stamp_units(d, &["api", "store"]);
    edit(
        &d.join("src/api/items.py"),
        "return 422, None",
        "return 400, None",
    );
    edit(
        &d.join("src/store/db.py"),
        "def count_items():\n    return len(_items)",
        "def count_items():\n    return sum(1 for _ in _items)",
    );
    dir
}

fn card(d: &Path, unit: &str) -> UnitCard {
    let text = fs::read_to_string(d.join(format!(".stewards/units/{unit}.toml"))).unwrap();
    toml::from_str(&text).unwrap()
}

fn test_card(d: &Path, unit: &str) -> TestCard {
    let text = fs::read_to_string(d.join(format!(".stewards/units/{unit}.test.toml"))).unwrap();
    toml::from_str(&text).unwrap()
}

fn check(pass: bool) -> Value {
    json!({"command": "pytest tests", "pass": pass, "output": "2 passed"})
}

fn approve(unit: &str) -> Value {
    json!({"unit": unit, "verdict": "approve", "findings": [], "card_patch": null,
           "check": check(true)})
}

/// An approval of `unit` that writes its docs, which a unit with an interface and none needs.
fn documented(unit: &str) -> Value {
    let mut a = approve(unit);
    a["doc"] = json!({"overview": "o", "how_it_works": "h", "how_to_change": "c"});
    a
}

fn answer(units: Value) -> Value {
    json!({"units": units, "pulled_anchors": [], "notes": ""})
}

/// Writes the review brief with `next`; returns its job.
fn review_job(d: &Path) -> Value {
    let (code, v) = run(d, &["next", "--task", "t"]);
    assert_eq!(code, 1, "{v}");
    let jobs = v["jobs"].as_array().unwrap();
    let job = jobs.iter().find(|j| j["role"] == "reviewer").unwrap();
    assert!(d.join(BRIEF).is_file());
    job.clone()
}

/// Writes `answer` with the brief's snapshot, as a reviewer copies it, unless it has one.
fn submit(d: &Path, answer: &Value) -> (i32, Value) {
    let mut answer = answer.clone();
    if let (None, Ok(h)) = (answer.get("snapshot"), read_header(&d.join(BRIEF))) {
        answer["snapshot"] = json!(h.snapshot);
    }
    fs::write(d.join(ANSWER), answer.to_string()).unwrap();
    run(d, &["submit", "--role", "reviewer", "--file", ANSWER])
}

/// The bytes of every file a reviewer submit may write for api and store.
fn written(d: &Path) -> Vec<Option<Vec<u8>>> {
    [
        ".stewards/units/api.toml",
        ".stewards/units/store.toml",
        ".stewards/units/api.test.toml",
        ".stewards/units/store.test.toml",
        ".stewards/briefs/api-tasks.json",
        ".stewards/briefs/store-tasks.json",
        "docs/units/api.md",
        "docs/units/store.md",
    ]
    .iter()
    .map(|rel| fs::read(d.join(rel)).ok())
    .collect()
}

/// Submits `answer` to a fresh review brief and asserts the error `code` naming `field`, with
/// nothing written and the brief kept.
fn rejected(d: &Path, answer: &Value, code: &str, field: &str) {
    review_job(d);
    let before = written(d);
    let (exit, v) = submit(d, answer);
    assert_eq!(exit, 2, "{v}");
    assert_eq!(v["error"]["code"], code, "{v}");
    let fields: Vec<&str> = v["error"]["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["field"].as_str().unwrap())
        .collect();
    assert!(fields.contains(&field), "{v}");
    assert_eq!(written(d), before);
    assert!(d.join(BRIEF).is_file());
}

fn jobs(d: &Path, args: &[&str]) -> (i32, Vec<(String, String)>) {
    let (code, v) = run(d, args);
    let jobs = v["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|j| {
            let unit = j["unit"].as_str().unwrap().to_string();
            (unit, j["role"].as_str().unwrap().to_string())
        })
        .collect();
    (code, jobs)
}

#[test]
fn next_lists_one_reviewer_job_naming_each_due_unit_and_why() {
    let dir = changed_both();
    let d = dir.path();
    let (code, v) = run(d, &["next", "--no-brief", "--task", "t"]);
    assert_eq!(code, 1, "{v}");
    let list = v["jobs"].as_array().unwrap();
    assert_eq!(list.len(), 1, "{v}");
    let job = &list[0];
    assert_eq!(
        (&job["unit"], &job["role"], &job["command"]),
        (
            &json!(""),
            &json!("reviewer"),
            &json!("stewards brief --role reviewer")
        )
    );
    assert_eq!(v["architect"], false);
    let reasons: Vec<&str> = job["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap())
        .collect();
    // every maintainer, tester, and documenter reason `--roles` gives, led by its unit
    let (_, roles) = run(d, &["next", "--roles", "--no-brief", "--task", "t"]);
    let mut expected = vec![];
    for j in roles["jobs"].as_array().unwrap() {
        if ["maintainer", "tester", "documenter"].contains(&j["role"].as_str().unwrap()) {
            for r in j["reasons"].as_array().unwrap() {
                let r = format!("{}: {}", j["unit"].as_str().unwrap(), r.as_str().unwrap());
                if !expected.contains(&r) {
                    expected.push(r);
                }
            }
        }
    }
    assert_eq!(reasons, expected);
    for r in [
        "api: stale",
        "api: interface changed: src/api/items.py::create_item",
        "store: stale",
        "store: interface changed: src/store/db.py::count_items",
    ] {
        assert!(reasons.contains(&r), "{r}: {reasons:?}");
    }
    assert!(
        job["changed"]
            .as_array()
            .unwrap()
            .contains(&json!("src/api/items.py"))
    );
}

#[test]
fn next_roles_lists_the_per_unit_jobs_and_the_architect() {
    let dir = changed_both();
    let (code, jobs) = jobs(dir.path(), &["next", "--roles", "--no-brief"]);
    assert_eq!(code, 1);
    for (unit, role) in [
        ("", "architect"),
        ("api", "maintainer"),
        ("api", "tester"),
        ("api", "documenter"),
        ("store", "maintainer"),
        ("store", "tester"),
        ("store", "documenter"),
    ] {
        assert!(
            jobs.contains(&(unit.into(), role.into())),
            "{unit} {role}: {jobs:?}"
        );
    }
}

#[test]
fn a_never_stamped_unit_keeps_its_maintainer_job_beside_the_review() {
    let dir = fixture("single");
    let d = dir.path();
    stamp_units(d, &["api"]);
    append(&d.join("src/api/items.py"), "\n# more\n");
    let (code, jobs) = jobs(d, &["next", "--no-brief"]);
    assert_eq!(code, 1);
    assert_eq!(
        jobs,
        [
            ("".to_string(), "reviewer".to_string()),
            ("store".to_string(), "maintainer".to_string())
        ]
    );
}

#[test]
fn a_stamped_unit_with_no_docs_is_due_in_both_flows() {
    let dir = fixture("single");
    let d = dir.path();
    stamp_units(d, &["api", "store"]);
    // store has an interface and no docs: a documenter job in both flows until it has some
    let (_, roles) = jobs(d, &["next", "--roles", "--no-brief"]);
    assert_eq!(roles, [("store".to_string(), "documenter".to_string())]);
    let (_, plain) = jobs(d, &["next", "--no-brief"]);
    assert_eq!(plain, [("".to_string(), "reviewer".to_string())]);
}

#[test]
fn review_brief_carries_each_units_hunks_card_scenarios_test_command_and_doc() {
    let dir = changed_both();
    let d = dir.path();
    review_job(d);
    let text = fs::read_to_string(d.join(BRIEF)).unwrap();
    let h = read_header(&d.join(BRIEF)).unwrap();
    assert_eq!(h.units, Some(vec!["api".to_string(), "store".to_string()]));
    assert!(h.unit.is_none());
    assert!(h.bytes <= REVIEW_BUDGET);
    for part in [
        "Role: reviewer.",
        "You review one change across the units below.",
        "each exactly once in `units` and no other: `api`, `store`.",
        "# Pack: api (reviewer)",
        "# Pack: store (reviewer)",
        "## Hunks",
        "+        return 400, None",
        "+    return sum(1 for _ in _items)",
        "## Interface",
        "POST /items; 201 with body; 422 on invalid",
        "## Anchors",
        "### `src/api/items.py::router`",
        "## Scenarios",
        "create-ok",
        "save-returns-id",
        "`pytest tests/api`",
        "`pytest tests/store`",
        "### Overview",
        "The HTTP surface for items.",
        "submit --role reviewer --file",
        "review.answer.json",
    ] {
        assert!(text.contains(part), "{part}");
    }
    // `brief --role reviewer` rebuilds the same brief
    let (code, v) = run(d, &["brief", "--role", "reviewer", "--task", "t"]);
    assert_eq!(code, 0, "{v}");
    // the snapshot differs: this fixture does not ignore the first brief
    let body = |t: &str| {
        let snapshot = read_header(&d.join(BRIEF)).unwrap().snapshot;
        t.split_once('\n').unwrap().1.replace(&snapshot, "SNAPSHOT")
    };
    let again = body(&fs::read_to_string(d.join(BRIEF)).unwrap());
    let snapshot = read_header(&d.join(BRIEF)).unwrap().snapshot;
    assert_eq!(again, body(&text.replace(&h.snapshot, &snapshot)));
    assert!(!text.contains("\n\n\nThe units under review"));
}

/// `n` bytes of added comment lines in `path`.
fn grow(path: &Path, n: usize) {
    append(
        path,
        &"# padding line for the review budget\n".repeat(n / 37 + 1),
    );
}

#[test]
fn units_past_the_review_budget_are_next_round_and_stay_due() {
    let dir = fixture("single");
    let d = dir.path();
    stamp_units(d, &["api", "store"]);
    grow(&d.join("src/api/items.py"), 30_000);
    grow(&d.join("src/store/db.py"), 30_000);
    let job = review_job(d);
    let h = read_header(&d.join(BRIEF)).unwrap();
    assert_eq!(h.units, Some(vec!["api".to_string()]));
    assert!(h.bytes <= REVIEW_BUDGET, "{}", h.bytes);
    let text = fs::read_to_string(d.join(BRIEF)).unwrap();
    assert!(text.contains("# Next round"));
    assert!(!text.contains("# Pack: store"));
    // api fits whole: not shrunk
    assert!(!text.contains("hunks truncated"));
    let reasons = job["reasons"].as_array().unwrap();
    assert!(
        reasons.contains(&json!("next round, over the review budget: store")),
        "{reasons:?}"
    );
    assert!(
        !reasons
            .iter()
            .any(|r| r.as_str().unwrap().starts_with("store:"))
    );
    let (code, v) = submit(d, &answer(json!([approve("api")])));
    assert_eq!(code, 0, "{v}");
    review_job(d);
    let h = read_header(&d.join(BRIEF)).unwrap();
    assert_eq!(h.units, Some(vec!["store".to_string()]));
}

#[test]
fn a_unit_alone_over_the_review_budget_is_shrunk_and_reviewed_alone() {
    let dir = fixture("single");
    let d = dir.path();
    stamp_units(d, &["api", "store"]);
    grow(&d.join("src/api/items.py"), 60_000);
    append(&d.join("src/store/db.py"), "\n# more\n");
    review_job(d);
    let h = read_header(&d.join(BRIEF)).unwrap();
    assert_eq!(h.units, Some(vec!["api".to_string()]));
    assert!(h.bytes <= REVIEW_BUDGET, "{}", h.bytes);
    let text = fs::read_to_string(d.join(BRIEF)).unwrap();
    assert!(text.contains("not in this review: store"));
    let (_, v) = run(d, &["pack", "--role", "reviewer", "--task", "t"]);
    let dropped = &v["units"][0]["dropped"];
    assert!(
        dropped
            .as_array()
            .unwrap()
            .contains(&json!("hunks truncated to 40 lines")),
        "{dropped}"
    );
    assert_eq!(v["next_round"], json!(["store"]));
}

#[test]
fn approving_every_unit_stamps_it_and_sets_both_marks_so_neither_next_lists_anything() {
    let dir = changed_both();
    let d = dir.path();
    review_job(d);
    let snapshot = read_header(&d.join(BRIEF)).unwrap().snapshot;
    let mut api = approve("api");
    api["scenario_patch"] = json!([{"id": "create-bad", "interface": "create-item",
        "given": "no name", "expect": "400", "level": "unit", "command": "pytest tests/api"}]);
    let mut store = approve("store");
    store["doc"] = json!({"overview": "Keeps items in memory.",
        "how_it_works": "save_item appends.", "how_to_change": "Start at save_item."});
    let (code, v) = submit(d, &answer(json!([api, store])));
    assert_eq!(code, 0, "{v}");
    let summary = v["summary"].as_str().unwrap();
    let short = &snapshot[..7];
    assert_eq!(
        summary,
        format!(
            "reviewer: api approve (card unchanged; 1 scenario upserted; stamped {short}), \
             store approve (card unchanged; doc updated; stamped {short})"
        )
    );
    for unit in ["api", "store"] {
        let r = card(d, unit).reviewed.unwrap();
        assert_eq!(r.tree, snapshot);
        assert_eq!(r.tester.as_deref(), Some(snapshot.as_str()));
        assert_eq!(r.documenter.as_deref(), Some(snapshot.as_str()));
    }
    // the scenario lands as the tester's would, the doc and its page as the documenter's would
    assert!(
        test_card(d, "api")
            .scenarios
            .iter()
            .any(|s| s.id == "create-bad")
    );
    assert_eq!(
        card(d, "store").doc.unwrap().overview,
        "Keeps items in memory."
    );
    let page = fs::read_to_string(d.join("docs/units/store.md")).unwrap();
    assert!(page.contains("Keeps items in memory."), "{page}");
    assert!(d.join("docs/units/README.md").is_file());
    assert!(!d.join(BRIEF).exists() && !d.join(ANSWER).exists());
    for args in [
        &["next", "--no-brief"][..],
        &["next", "--roles", "--no-brief"],
    ] {
        let (code, jobs) = jobs(d, args);
        assert_eq!((code, jobs), (0, vec![]), "{args:?}");
    }
    // telemetry records the reviewer like any role, with the units it covered
    let telemetry = fs::read_dir(d.join(".stewards/telemetry"))
        .unwrap()
        .map(|e| fs::read_to_string(e.unwrap().path()).unwrap())
        .collect::<String>();
    let line: Value = serde_json::from_str(telemetry.lines().last().unwrap()).unwrap();
    assert_eq!(
        (&line["role"], &line["units"]),
        (&json!("reviewer"), &json!(2))
    );
    assert!(line["unit"].is_null() && line["bytes"].as_u64().unwrap() > 0);
}

#[test]
fn changes_on_one_unit_stamps_the_other_and_writes_its_fix_task() {
    let dir = changed_both();
    let d = dir.path();
    review_job(d);
    let api = json!({"unit": "api", "verdict": "changes", "card_patch": null, "check": check(true),
        "findings": [{"file": "src/api/items.py", "line": 8, "rung": "untraced",
                      "cut": "the status change to 400", "replace_with": "keep 422"}]});
    let mut store = approve("store");
    store["doc"] = json!({"overview": "o", "how_it_works": "h", "how_to_change": "c"});
    let (code, v) = submit(d, &answer(json!([api, store])));
    assert_eq!(code, 0, "{v}");
    let summary = v["summary"].as_str().unwrap();
    assert!(
        summary.starts_with(
            "reviewer: api changes (1 finding; card unchanged; not stamped), store approve ("
        ),
        "{summary}"
    );
    assert_eq!(
        v["findings"],
        json!([{"unit": "api", "file": "src/api/items.py", "line": 8,
                "cut": "the status change to 400"}])
    );
    assert_ne!(
        card(d, "api").reviewed.unwrap().tree,
        card(d, "store").reviewed.unwrap().tree
    );
    let tasks = fs::read_to_string(d.join(".stewards/briefs/api-tasks.json")).unwrap();
    assert!(tasks.contains("src/api/items.py:8 untraced: cut the status change to 400"));
    assert!(!d.join(".stewards/briefs/store-tasks.json").exists());
    let (code, v) = run(d, &["brief", "api", "--role", "developer"]);
    assert_eq!(code, 0, "{v}");
    // api is still due, alone
    review_job(d);
    let h = read_header(&d.join(BRIEF)).unwrap();
    assert_eq!(h.units, Some(vec!["api".to_string()]));
}

#[test]
fn a_bug_without_severity_stays_unjudged_and_only_roles_lists_its_validator() {
    let dir = changed_both();
    let d = dir.path();
    review_job(d);
    let mut api = approve("api");
    api["bugs"] = json!([{"title": "400 for a missing name", "repro": "pytest tests/api",
                          "evidence": "the contract says 422", "interface": "create-item"}]);
    let mut store = approve("store");
    store["doc"] = json!({"overview": "o", "how_it_works": "h", "how_to_change": "c"});
    let (code, v) = submit(d, &answer(json!([api, store])));
    assert_eq!(code, 0, "{v}");
    let gap = card(d, "api")
        .gaps
        .into_iter()
        .find(|g| g.title == "400 for a missing name")
        .unwrap();
    assert!(gap.severity.is_none());
    assert_eq!(gap.anchor.as_deref(), Some("src/api/items.py::create_item"));
    assert_eq!(jobs(d, &["next", "--no-brief"]), (0, vec![]));
    assert_eq!(
        jobs(d, &["next", "--roles", "--no-brief"]).1,
        [("api".to_string(), "validator".to_string())]
    );
}

#[test]
fn a_missing_unit_is_rejected_and_nothing_is_written() {
    let dir = changed_both();
    rejected(
        dir.path(),
        &answer(json!([approve("api")])),
        "answer_parse_failed",
        "units",
    );
}

#[test]
fn a_unit_not_under_review_is_rejected_and_nothing_is_written() {
    let dir = fixture("single");
    let d = dir.path();
    stamp_units(d, &["api", "store"]);
    // store is due; api, with docs and no change, is not
    append(&d.join("src/store/db.py"), "\n# more\n");
    let a = answer(json!([approve("store"), approve("api")]));
    rejected(d, &a, "answer_parse_failed", "units[1].unit");
}

#[test]
fn a_unit_given_twice_is_rejected() {
    let dir = changed_both();
    let a = answer(json!([approve("api"), approve("store"), approve("api")]));
    rejected(dir.path(), &a, "answer_parse_failed", "units[2].unit");
}

#[test]
fn approving_a_unit_whose_check_failed_is_rejected() {
    let dir = changed_both();
    let mut api = approve("api");
    api["check"] = check(false);
    let a = answer(json!([api, approve("store")]));
    rejected(dir.path(), &a, "answer_parse_failed", "units[0].verdict");
}

#[test]
fn an_over_cap_field_is_rejected() {
    let dir = changed_both();
    let mut store = documented("store");
    store["check"]["output"] = json!("x".repeat(2001));
    let a = answer(json!([approve("api"), store]));
    rejected(dir.path(), &a, "answer_too_long", "units[1].check.output");
}

#[test]
fn a_finding_in_another_units_file_is_rejected() {
    let dir = changed_both();
    let api = json!({"unit": "api", "verdict": "changes", "card_patch": null, "check": check(true),
        "findings": [{"file": "src/store/db.py", "line": 1, "rung": "untraced",
                      "cut": "x", "replace_with": "y"}]});
    let a = answer(json!([api, approve("store")]));
    rejected(
        dir.path(),
        &a,
        "answer_parse_failed",
        "units[0].findings[0].file",
    );
}

#[test]
fn a_replayed_answer_finds_no_brief_and_writes_nothing() {
    let dir = changed_both();
    let d = dir.path();
    review_job(d);
    let a = answer(json!([approve("api"), documented("store")]));
    let (code, v) = submit(d, &a);
    assert_eq!(code, 0, "{v}");
    let after = written(d);
    let (code, v) = submit(d, &a);
    assert_eq!((code, &v["error"]["code"]), (2, &json!("no_brief")), "{v}");
    assert_eq!(written(d), after);
}

#[test]
fn a_card_changed_since_submit_read_it_is_card_changed_and_nothing_is_written() {
    let dir = changed_both();
    let d = dir.path();
    review_job(d);
    let header = read_header(&d.join(BRIEF)).unwrap();
    let files = [
        ".stewards/units/api.toml",
        ".stewards/units/store.toml",
        ".stewards/briefs/api-tasks.json",
        ".stewards/briefs/store-tasks.json",
    ]
    .map(String::from);
    let before = record(d, &files).unwrap();
    // another submit writes store's card in between
    append(
        &d.join(".stewards/units/store.toml"),
        "\n[[gap]]\nkind = \"gap\"\ntitle = \"g\"\nnote = \"n\"\n",
    );
    let repo = Repo {
        name: "single".into(),
        root: d.to_path_buf(),
        loaded: Some(load_repo(d).unwrap()),
    };
    let api_card = fs::read(d.join(".stewards/units/api.toml")).unwrap();
    let mut text = answer(json!([approve("api"), documented("store")]));
    text["snapshot"] = json!(header.snapshot);
    let text = text.to_string();
    let Err(e) = stewards::submit::reviewer(&repo, "", &header, &text, &before) else {
        panic!("a changed card is refused");
    };
    let e = e.downcast_ref::<stewards::output::CliError>().unwrap();
    assert_eq!(e.code, "card_changed");
    assert_eq!(
        fs::read(d.join(".stewards/units/api.toml")).unwrap(),
        api_card
    );
    assert!(card(d, "store").gaps.iter().any(|g| g.title == "g"));
    assert!(d.join(BRIEF).is_file());
}

#[test]
fn reviewer_takes_no_unit_in_brief_pack_and_submit() {
    let dir = changed_both();
    let d = dir.path();
    for args in [
        &["brief", "api", "--role", "reviewer"][..],
        &["pack", "api", "--role", "reviewer"],
        &["submit", "api", "--role", "reviewer", "--file", ANSWER],
    ] {
        let (code, v) = run(d, args);
        assert_eq!(
            (code, &v["error"]["code"]),
            (2, &json!("usage")),
            "{args:?}: {v}"
        );
    }
    let (code, v) = run(d, &["pack", "--role", "reviewer"]);
    assert_eq!(code, 0, "{v}");
    let units: Vec<&Value> = v["units"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| &p["unit"])
        .collect();
    assert_eq!(units, [&json!("api"), &json!("store")]);
    assert_eq!(v["next_round"], json!([]));
}

#[test]
fn brief_role_reviewer_with_nothing_due_is_usage() {
    let dir = fixture("single");
    let d = dir.path();
    stamp_units(d, &["api"]);
    let (code, v) = run(d, &["brief", "--role", "reviewer"]);
    assert_eq!((code, &v["error"]["code"]), (2, &json!("usage")), "{v}");
}

#[test]
fn approving_a_unit_with_an_interface_and_no_docs_without_a_doc_is_rejected() {
    let dir = changed_both();
    // store has an interface and no docs: `next` would list it forever
    let a = answer(json!([approve("api"), approve("store")]));
    rejected(dir.path(), &a, "answer_parse_failed", "units[1].doc");
}

#[test]
fn a_card_patch_renders_the_units_page_again() {
    let dir = changed_both();
    let d = dir.path();
    review_job(d);
    let mut api = approve("api");
    api["card_patch"] = json!({"interface": [{"id": "create-item",
        "anchor": "src/api/items.py::create_item", "contract": "POST /items; 400 on invalid"}]});
    let (code, v) = submit(d, &answer(json!([api, documented("store")])));
    assert_eq!(code, 0, "{v}");
    // api's answer has no doc, but its page shows the interface it changed
    let page = fs::read_to_string(d.join("docs/units/api.md")).unwrap();
    assert!(page.contains("400 on invalid"), "{page}");
}

/// Rewrites the units list of the review brief's header.
fn set_header_units(d: &Path, from: &str, to: &str) {
    let text = fs::read_to_string(d.join(BRIEF)).unwrap();
    let (first, rest) = text.split_once('\n').unwrap();
    assert!(first.contains(from), "{first}");
    fs::write(
        d.join(BRIEF),
        format!("{}\n{rest}", first.replace(from, to)),
    )
    .unwrap();
}

#[test]
fn a_header_listing_an_unstamped_unknown_or_repeated_unit_is_no_brief() {
    let dir = fixture("single");
    let d = dir.path();
    stamp_units(d, &["api"]);
    append(&d.join("src/api/items.py"), "\n# more\n");
    review_job(d);
    // store, never stamped, is not under review: its files are listed once for the brief
    let text = fs::read_to_string(d.join(BRIEF)).unwrap();
    assert_eq!(text.matches("Other changed files").count(), 1, "{text}");
    assert!(text.contains("# Other changed files\n\nIn units not under review"));
    for units in [
        r#"["api","store"]"#,
        r#"["api","nope"]"#,
        r#"["api","api"]"#,
    ] {
        review_job(d);
        set_header_units(d, r#""units":["api"]"#, &format!(r#""units":{units}"#));
        let before = written(d);
        let a = answer(json!([approve("api"), documented("store")]));
        let (code, v) = submit(d, &a);
        assert_eq!(
            (code, &v["error"]["code"]),
            (2, &json!("no_brief")),
            "{units}: {v}"
        );
        assert_eq!(written(d), before);
    }
}

#[test]
fn a_unit_whose_tester_mark_is_behind_its_stamp_shows_that_change_and_why() {
    let dir = fixture("single");
    let d = dir.path();
    stamp_units(d, &["api", "store"]);
    edit(
        &d.join("src/api/items.py"),
        "return 422, None",
        "return 400, None",
    );
    // a maintainer stamps the change; the tester mark stays at the old stamp
    stamp_units(d, &["api"]);
    let r = card(d, "api").reviewed.unwrap();
    assert_ne!(r.tester.as_deref(), Some(r.tree.as_str()));
    review_job(d);
    let text = fs::read_to_string(d.join(BRIEF)).unwrap();
    let api = &text[text.find("# Pack: api").unwrap()..text.find("# Pack: store").unwrap()];
    assert!(
        api.contains("Due because: interface changed: src/api/items.py::create_item"),
        "{api}"
    );
    assert!(api.contains("+        return 400, None"), "{api}");
    assert!(text.contains("Due because: no docs yet"));
}

#[test]
fn an_answer_to_an_older_review_brief_is_no_brief_and_writes_nothing() {
    let dir = changed_both();
    let d = dir.path();
    review_job(d);
    let before = written(d);
    let mut a = answer(json!([approve("api"), documented("store")]));
    a["snapshot"] = json!("0".repeat(40));
    let (code, v) = submit(d, &a);
    assert_eq!((code, &v["error"]["code"]), (2, &json!("no_brief")), "{v}");
    assert!(v["error"]["fix"].as_str().unwrap().contains("read"), "{v}");
    assert_eq!(written(d), before);
    assert!(d.join(BRIEF).is_file());
    // the brief prints the snapshot to copy
    let h = read_header(&d.join(BRIEF)).unwrap();
    let text = fs::read_to_string(d.join(BRIEF)).unwrap();
    assert!(text.contains(&format!(
        "Set `snapshot` in your answer to `{}`.",
        h.snapshot
    )));
}

#[test]
fn a_check_is_required_when_the_brief_shows_a_test_command() {
    let dir = changed_both();
    let d = dir.path();
    review_job(d);
    let text = fs::read_to_string(d.join(BRIEF)).unwrap();
    assert!(text.contains("## Test commands\n\n| command |\n|---|\n| pytest |"));
    let mut api = approve("api");
    api["check"] = Value::Null;
    let a = answer(json!([api, documented("store")]));
    rejected(d, &a, "answer_parse_failed", "units[0].check");
}

#[test]
fn with_no_test_command_a_check_is_absent_and_approve_needs_none() {
    let dir = fixture("single");
    let d = dir.path();
    edit(
        &d.join(".stewards/repo.toml"),
        "test = [\"pytest\"]",
        "test = []",
    );
    for unit in ["api", "store"] {
        let rel = format!(".stewards/units/{unit}.test.toml");
        edit(
            &d.join(&rel),
            &format!("test_command = \"pytest tests/{unit}\"\n"),
            "",
        );
    }
    stamp_units(d, &["api", "store"]);
    append(&d.join("src/api/items.py"), "\n# more\n");
    review_job(d);
    let text = fs::read_to_string(d.join(BRIEF)).unwrap();
    assert!(text.contains("## Test commands\n\n(none)"), "{text}");
    // a check reported anyway is refused
    let a = answer(json!([approve("api"), documented("store")]));
    rejected(d, &a, "answer_parse_failed", "units[0].check");
    let (mut api, mut store) = (approve("api"), documented("store"));
    api.as_object_mut().unwrap().remove("check");
    store["check"] = Value::Null;
    let (code, v) = submit(d, &answer(json!([api, store])));
    assert_eq!(code, 0, "{v}");
    assert!(card(d, "api").reviewed.unwrap().tree == card(d, "store").reviewed.unwrap().tree);
}

#[test]
fn a_unit_alone_over_the_budget_drops_its_doc_and_scenarios_to_fit() {
    let dir = fixture("single");
    let d = dir.path();
    let big = "word ".repeat(12_000);
    edit(
        &d.join(".stewards/units/api.toml"),
        "overview = \"\"\"The HTTP surface for items.\"\"\"",
        &format!("overview = \"\"\"{big}\"\"\""),
    );
    let mut scenarios = String::new();
    for i in 0..400 {
        scenarios += &format!(
            "\n[[scenario]]\nid = \"s{i}\"\ninterface = \"create-item\"\ngiven = \"{}\"\nexpect = \"ok\"\nlevel = \"unit\"\n",
            "g".repeat(150)
        );
    }
    append(&d.join(".stewards/units/api.test.toml"), &scenarios);
    stamp_units(d, &["api", "store"]);
    append(&d.join("src/api/items.py"), "\n# more\n");
    review_job(d);
    let h = read_header(&d.join(BRIEF)).unwrap();
    assert_eq!(h.units, Some(vec!["api".to_string()]));
    assert!(h.bytes <= REVIEW_BUDGET, "{}", h.bytes);
    let text = fs::read_to_string(d.join(BRIEF)).unwrap();
    assert!(text.contains("Left out to fit: "), "{text}");
    assert!(text.contains("doc; scenarios"));
    assert!(!text.contains("## Scenarios") && !text.contains("### Overview"));
    assert!(text.contains("not in this review: store"));
}

#[test]
fn a_unit_still_over_the_budget_has_its_fenced_bodies_cut_and_says_so_once() {
    let dir = fixture("single");
    let d = dir.path();
    let units = fs::read_to_string(d.join(".stewards/units.toml")).unwrap();
    let purpose = units
        .lines()
        .find(|l| l.starts_with("purpose = "))
        .unwrap()
        .to_string();
    edit(
        &d.join(".stewards/units.toml"),
        &purpose,
        &format!("purpose = \"{}\"", "word ".repeat(12_000)),
    );
    stamp_units(d, &["api", "store"]);
    append(&d.join("src/api/items.py"), "\n# more\n");
    review_job(d);
    let h = read_header(&d.join(BRIEF)).unwrap();
    assert!(h.bytes <= REVIEW_BUDGET, "{}", h.bytes);
    let text = fs::read_to_string(d.join(BRIEF)).unwrap();
    assert_eq!(text.matches("fenced bodies cut to ").count(), 1, "{text}");
    let left_out = text
        .lines()
        .find(|l| l.contains("Left out to fit: "))
        .unwrap();
    assert!(
        !left_out.contains("; ;") && !left_out.trim_end().ends_with(';'),
        "{left_out}"
    );
}

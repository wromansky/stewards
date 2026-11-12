#[allow(dead_code)] // each test crate uses a subset of the helpers
mod common;

use std::fs;
use std::path::Path;

use common::{append, edit, fixture, run, stamp_units};
use serde_json::{Value, json};
use stewards::brief::read_header;

const TASKS: &str = ".stewards/briefs/api-tasks.json";
const CARD: &str = ".stewards/units/api.toml";
const MAINTAINER_ANSWER: &str = ".stewards/briefs/api-maintainer.answer.json";

fn task(title: &str, files: &[&str]) -> Value {
    json!({"title": title, "change": format!("do {title}"), "files": files,
           "check": "pytest tests/api"})
}

fn submit_maintainer(dir: &Path, answer: &Value) -> (i32, Value) {
    fs::write(dir.join(MAINTAINER_ANSWER), answer.to_string()).unwrap();
    let args = [
        "submit",
        "api",
        "--role",
        "maintainer",
        "--file",
        MAINTAINER_ANSWER,
    ];
    run(dir, &args)
}

/// Briefs the api maintainer to explain and submits an answer with `tasks`.
fn plan(dir: &Path, tasks: Value) -> (i32, Value) {
    let args = [
        "brief",
        "api",
        "--role",
        "maintainer",
        "--task",
        "add a count endpoint",
    ];
    assert_eq!(run(dir, &args).0, 0);
    let answer = json!({"explanation": "plan", "tasks": tasks, "card_patch": null,
                        "pulled_anchors": [], "notes": ""});
    submit_maintainer(dir, &answer)
}

/// Writes the api maintainer's review brief with `next` and submits `findings`.
fn review(dir: &Path, verdict: &str, findings: Value) -> (i32, Value) {
    run(dir, &["next", "--task", "t", "--no-run"]);
    assert!(dir.join(".stewards/briefs/api-maintainer.md").is_file());
    let answer = json!({"verdict": verdict, "findings": findings, "card_patch": null,
                        "tester_needed": false, "pulled_anchors": [], "notes": ""});
    submit_maintainer(dir, &answer)
}

fn finding(file: &str, line: u32) -> Value {
    json!({"file": file, "line": line, "rung": "one_line", "cut": "the loop",
           "replace_with": "sum()"})
}

fn developers(dir: &Path) -> (i32, Value) {
    let args = [
        "brief",
        "api",
        "--role",
        "developer",
        "--task",
        "add a count endpoint",
    ];
    run(dir, &args)
}

/// Plans two tasks, lanes `src/api/items.py` and `tests/api/test_items.py`, and writes their
/// developer briefs.
fn two_developers(dir: &Path) {
    let tasks = json!([
        task("count", &["src/api/items.py"]),
        task("test count", &["tests/api/test_items.py"])
    ]);
    assert_eq!(plan(dir, tasks).0, 0);
    assert_eq!(developers(dir).0, 0);
}

fn developer_answer(blocked_on: Option<&str>) -> Value {
    json!({"check": {"command": "pytest tests/api", "pass": true, "output": "1 passed"},
           "summary": "done", "blocked_on": blocked_on, "pulled_anchors": [], "notes": ""})
}

fn submit_developer_as(dir: &Path, name: &str, answer: &Value) -> (i32, Value) {
    let file = format!(".stewards/briefs/{name}");
    fs::write(dir.join(&file), answer.to_string()).unwrap();
    run(
        dir,
        &["submit", "api", "--role", "developer", "--file", &file],
    )
}

fn submit_developer(dir: &Path, n: usize, answer: &Value) -> (i32, Value) {
    submit_developer_as(dir, &format!("api-developer-{n}.answer.json"), answer)
}

/// Asserts the plan is rejected with `code` naming `field` and no task list is written.
fn plan_rejected(tasks: Value, code: &str, field: &str) {
    let dir = fixture("single");
    let (exit, v) = plan(dir.path(), tasks);
    assert_eq!((exit, &v["error"]["code"]), (2, &json!(code)), "{v}");
    let fields: Vec<&Value> = v["error"]["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| &e["field"])
        .collect();
    assert!(fields.contains(&&json!(field)), "{v}");
    assert!(!dir.path().join(TASKS).exists());
}

#[test]
fn two_tasks_with_disjoint_lanes_give_two_briefs_with_their_task_and_lane() {
    let dir = fixture("single");
    let tasks = json!([
        task("count", &["src/api/items.py"]),
        task("test count", &["tests/api/test_items.py"])
    ]);
    let (code, v) = plan(dir.path(), tasks);
    assert_eq!(code, 0, "{v}");
    let root = dir.path().canonicalize().unwrap();
    assert_eq!(
        v["summary"],
        format!(
            "api maintainer: explained; card unchanged; 2 tasks; next: stewards -C '{}' brief api \
             --role developer",
            root.display()
        )
    );
    let (code, v) = developers(dir.path());
    assert_eq!(code, 0, "{v}");
    for (n, title, lane) in [
        (1, "count", "src/api/items.py"),
        (2, "test count", "tests/api/test_items.py"),
    ] {
        let path = root.join(format!(".stewards/briefs/api-developer-{n}.md"));
        assert_eq!(
            v["jobs"][n - 1],
            json!({"task": n, "title": title, "brief_path": path})
        );
        let h = read_header(&path).unwrap();
        assert_eq!((h.n, h.lane), (Some(n), Some(vec![lane.to_string()])));
    }
}

#[test]
fn overlapping_lanes_are_rejected_and_write_no_task_list() {
    let tasks = json!([
        task("a", &["src/api/items.py"]),
        task("b", &["tests/api/test_items.py", "src/api/items.py"])
    ]);
    plan_rejected(tasks, "answer_parse_failed", "tasks[1].files[1]");
}

#[test]
fn a_lane_file_of_another_unit_is_rejected() {
    let tasks = json!([task("a", &["src/store/db.py"])]);
    plan_rejected(tasks, "answer_parse_failed", "tasks[0].files[0]");
}

#[test]
fn a_lane_path_with_dot_dot_is_rejected() {
    let tasks = json!([task("a", &["src/api/../api/items.py"])]);
    plan_rejected(tasks, "answer_parse_failed", "tasks[0].files[0]");
}

#[test]
fn a_task_check_with_a_semicolon_is_rejected() {
    let mut t = task("a", &["src/api/items.py"]);
    t["check"] = json!("pytest tests/api; rm -rf src");
    plan_rejected(json!([t]), "command_not_allowed", "tasks[0].check");
}

#[test]
fn a_task_with_no_files_is_rejected() {
    plan_rejected(
        json!([task("a", &[])]),
        "answer_parse_failed",
        "tasks[0].files",
    );
}

#[test]
fn review_findings_on_two_files_give_one_fix_task() {
    let dir = fixture("single");
    let findings = json!([
        finding("src/api/items.py", 4),
        finding("tests/api/test_items.py", 5),
        finding("src/api/items.py", 9)
    ]);
    let (code, v) = review(dir.path(), "changes", findings);
    assert_eq!(code, 0, "{v}");
    let tasks: Value = serde_json::from_slice(&fs::read(dir.path().join(TASKS)).unwrap()).unwrap();
    assert_eq!(
        tasks,
        json!([{"title": "fix review findings",
                "change": "src/api/items.py:4 one_line: cut the loop; replace with sum()\n\
                           tests/api/test_items.py:5 one_line: cut the loop; replace with sum()\n\
                           src/api/items.py:9 one_line: cut the loop; replace with sum()",
                "files": ["src/api/items.py", "tests/api/test_items.py"],
                "check": null}])
    );
}

#[test]
fn a_finding_on_another_units_file_is_rejected() {
    let dir = fixture("single");
    let findings = json!([
        finding("src/api/items.py", 4),
        finding("src/store/db.py", 2)
    ]);
    let (code, v) = review(dir.path(), "changes", findings);
    assert_eq!(
        (code, &v["error"]["code"]),
        (2, &json!("answer_parse_failed"))
    );
    assert_eq!(v["error"]["errors"][0]["field"], "findings[1].file", "{v}");
    assert!(!dir.path().join(TASKS).exists());
}

#[test]
fn an_approving_review_removes_the_task_list() {
    let dir = fixture("single");
    assert_eq!(
        plan(dir.path(), json!([task("a", &["src/api/items.py"])])).0,
        0
    );
    assert!(dir.path().join(TASKS).is_file());
    let (code, v) = review(dir.path(), "approve", json!([]));
    assert_eq!(code, 0, "{v}");
    assert!(!dir.path().join(TASKS).exists());
}

#[test]
fn a_shorter_task_list_removes_the_extra_developer_brief() {
    let dir = fixture("single");
    two_developers(dir.path());
    let briefs = dir.path().join(".stewards/briefs");
    fs::write(briefs.join("api-developer-2.answer.json"), "{}").unwrap();
    assert_eq!(
        plan(dir.path(), json!([task("a", &["src/api/items.py"])])).0,
        0
    );
    let (code, v) = developers(dir.path());
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["jobs"].as_array().unwrap().len(), 1);
    assert!(briefs.join("api-developer-1.md").is_file());
    assert!(!briefs.join("api-developer-2.md").exists());
    assert!(!briefs.join("api-developer-2.answer.json").exists());
}

#[test]
fn developer_brief_with_no_task_list_is_no_tasks() {
    let dir = fixture("single");
    let (code, v) = developers(dir.path());
    assert_eq!((code, &v["error"]["code"]), (2, &json!("no_tasks")));
    let fix = v["error"]["fix"].as_str().unwrap();
    assert!(
        fix.contains("stewards brief api --role maintainer"),
        "{fix}"
    );
}

#[test]
fn a_task_list_edited_to_add_an_unowned_file_is_no_tasks_and_writes_no_brief() {
    let dir = fixture("single");
    assert_eq!(
        plan(dir.path(), json!([task("a", &["src/api/items.py"])])).0,
        0
    );
    edit(
        &dir.path().join(TASKS),
        "\"src/api/items.py\"",
        "\"src/api/items.py\", \"src/store/db.py\"",
    );
    let (code, v) = developers(dir.path());
    assert_eq!((code, &v["error"]["code"]), (2, &json!("no_tasks")), "{v}");
    assert!(
        !dir.path()
            .join(".stewards/briefs/api-developer-1.md")
            .exists()
    );
}

#[test]
fn the_developer_pack_shows_source_only_for_anchors_in_the_lane() {
    let dir = fixture("single");
    let anchor = "\n[[anchor]]\npath = \"tests/api/test_items.py\"\nname = \"test_create_ok\"\n";
    append(&dir.path().join(CARD), anchor);
    assert_eq!(
        plan(dir.path(), json!([task("a", &["tests/api/test_items.py"])])).0,
        0
    );
    assert_eq!(developers(dir.path()).0, 0);
    let brief = fs::read_to_string(dir.path().join(".stewards/briefs/api-developer-1.md")).unwrap();
    assert!(brief.contains("def test_create_ok"), "{brief}");
    assert!(!brief.contains("save_item(body)"), "{brief}");
    assert!(brief.contains("(source not inlined)"), "{brief}");
}

#[test]
fn a_developer_submit_leaves_the_card_stamp_and_task_list_unchanged() {
    let dir = fixture("single");
    stamp_units(dir.path(), &["api"]);
    two_developers(dir.path());
    let card = fs::read(dir.path().join(CARD)).unwrap();
    let tasks = fs::read(dir.path().join(TASKS)).unwrap();
    edit(
        &dir.path().join("src/api/items.py"),
        "router = []",
        "router = [1]",
    );
    let (code, v) = submit_developer(dir.path(), 1, &developer_answer(None));
    assert_eq!(code, 0, "{v}");
    assert_eq!(fs::read(dir.path().join(CARD)).unwrap(), card);
    assert_eq!(fs::read(dir.path().join(TASKS)).unwrap(), tasks);
    assert!(
        !dir.path()
            .join(".stewards/briefs/api-developer-1.md")
            .exists()
    );
}

#[test]
fn a_write_inside_the_lane_is_counted() {
    let dir = fixture("single");
    two_developers(dir.path());
    edit(
        &dir.path().join("src/api/items.py"),
        "router = []",
        "router = [1]",
    );
    let (code, v) = submit_developer(dir.path(), 1, &developer_answer(None));
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["summary"], "api developer 1: 1 file; check pass");
    assert_eq!(v["out_of_lane"], json!([]));
    assert_eq!(v["blocked_on"], Value::Null);
}

#[test]
fn a_write_to_an_owned_file_in_no_lane_is_reported_and_the_submit_succeeds() {
    let dir = fixture("single");
    two_developers(dir.path());
    fs::write(dir.path().join("src/api/extra.py"), "x = 1\n").unwrap();
    let (code, v) = submit_developer(dir.path(), 1, &developer_answer(None));
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        v["summary"],
        "api developer 1: 0 files; check pass; out of lane: src/api/extra.py"
    );
    assert_eq!(v["out_of_lane"], json!(["src/api/extra.py"]));
    let telemetry = dir.path().join(".stewards/telemetry");
    let file = fs::read_dir(telemetry).unwrap().next().unwrap().unwrap();
    let text = fs::read_to_string(file.path()).unwrap();
    let line: Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
    assert_eq!(
        (&line["role"], &line["out_of_lane"]),
        (&json!("developer"), &json!(1))
    );
}

#[test]
fn a_write_in_a_sibling_tasks_lane_is_not_out_of_lane() {
    let dir = fixture("single");
    two_developers(dir.path());
    append(&dir.path().join("tests/api/test_items.py"), "# more\n");
    let (code, v) = submit_developer(dir.path(), 1, &developer_answer(None));
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["summary"], "api developer 1: 0 files; check pass");
    assert_eq!(v["out_of_lane"], json!([]));
}

#[test]
fn blocked_on_is_printed_and_the_summary_says_blocked() {
    let dir = fixture("single");
    two_developers(dir.path());
    let answer = developer_answer(Some("need src/store/db.py\nfor the column"));
    let (code, v) = submit_developer(dir.path(), 1, &answer);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["blocked_on"], "need src/store/db.py\nfor the column");
    assert_eq!(
        v["summary"],
        "api developer 1: blocked: need src/store/db.py for the column"
    );
}

#[test]
fn an_answer_file_with_the_wrong_name_is_a_usage_error() {
    let dir = fixture("single");
    two_developers(dir.path());
    let name = "api-developer.answer.json";
    let (code, v) = submit_developer_as(dir.path(), name, &developer_answer(None));
    assert_eq!((code, &v["error"]["code"]), (2, &json!("usage")), "{v}");
}

#[test]
fn a_task_2_brief_under_task_1s_name_fails_the_header_check() {
    let dir = fixture("single");
    two_developers(dir.path());
    let briefs = dir.path().join(".stewards/briefs");
    fs::copy(
        briefs.join("api-developer-2.md"),
        briefs.join("api-developer-1.md"),
    )
    .unwrap();
    let (code, v) = submit_developer(dir.path(), 1, &developer_answer(None));
    assert_eq!((code, &v["error"]["code"]), (2, &json!("no_brief")), "{v}");
}

#[test]
fn after_a_developer_change_next_lists_the_maintainer_review() {
    let dir = fixture("single");
    stamp_units(dir.path(), &["api", "store"]);
    two_developers(dir.path());
    edit(
        &dir.path().join("src/api/items.py"),
        "router = []",
        "router = [1]",
    );
    assert_eq!(
        submit_developer(dir.path(), 1, &developer_answer(None)).0,
        0
    );
    let (code, v) = run(
        dir.path(),
        &["next", "--task", "t", "--no-run", "--no-brief"],
    );
    assert_eq!(code, 1, "{v}");
    let jobs: Vec<(&Value, &Value)> = v["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|j| (&j["unit"], &j["role"]))
        .collect();
    assert!(jobs.contains(&(&json!("api"), &json!("maintainer"))), "{v}");
}

#[test]
#[cfg(unix)]
fn a_symlink_at_the_task_list_is_unsafe_path_and_its_target_is_unchanged() {
    let dir = fixture("single");
    let target = dir.path().join("target.json");
    fs::write(&target, "keep").unwrap();
    let args = ["brief", "api", "--role", "maintainer", "--task", "t"];
    assert_eq!(run(dir.path(), &args).0, 0);
    std::os::unix::fs::symlink(&target, dir.path().join(TASKS)).unwrap();
    let answer = json!({"explanation": "plan", "tasks": [task("a", &["src/api/items.py"])],
                        "card_patch": null, "pulled_anchors": [], "notes": ""});
    let (code, v) = submit_maintainer(dir.path(), &answer);
    assert_eq!(
        (code, &v["error"]["code"]),
        (2, &json!("unsafe_path")),
        "{v}"
    );
    let (code, v) = developers(dir.path());
    assert_eq!(
        (code, &v["error"]["code"]),
        (2, &json!("unsafe_path")),
        "{v}"
    );
    assert_eq!(fs::read_to_string(&target).unwrap(), "keep");
}

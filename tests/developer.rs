#[allow(dead_code)] // each test crate uses a subset of the helpers
mod common;

use std::fs;
use std::path::Path;

use common::{append, edit, fixture, run, stamp_units};
use serde_json::{Value, json};
use stewards::brief::read_header;
use stewards::output::CliError;
use stewards::submit::{record, write_remove_and_retire};
use stewards::workspace::Repo;

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
    run(dir, &["next", "--roles", "--task", "t", "--no-run"]);
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
            json!({"task": n, "title": title, "brief_path": path,
                   "handoff": format!("Read `{}` and follow it.", path.display())})
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
fn the_developer_pack_lists_only_the_anchors_in_the_lane() {
    let dir = fixture("single");
    let anchor = "\n[[anchor]]\npath = \"tests/api/test_items.py\"\nname = \"test_create_ok\"\n";
    append(&dir.path().join(CARD), anchor);
    assert_eq!(
        plan(dir.path(), json!([task("a", &["tests/api/test_items.py"])])).0,
        0
    );
    assert_eq!(developers(dir.path()).0, 0);
    let brief = fs::read_to_string(dir.path().join(".stewards/briefs/api-developer-1.md")).unwrap();
    assert!(
        brief.contains("### `tests/api/test_items.py::test_create_ok`"),
        "{brief}"
    );
    assert!(brief.contains("def test_create_ok"), "{brief}");
    assert!(!brief.contains("### `src/api/items.py::router`"), "{brief}");
    assert!(!brief.contains("save_item(body)"), "{brief}");
    assert!(!brief.contains("(source not inlined)"), "{brief}");
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
    assert!(
        !dir.path()
            .join(".stewards/briefs/api-developer-1.answer.json")
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
    // what it printed, for `review` to report, blocked_on and out_of_lane included
    let mut printed = v.clone();
    printed.as_object_mut().unwrap().remove("stewards");
    let result = dir
        .path()
        .join(".stewards/briefs/api-developer-1.result.json");
    let written: Value = serde_json::from_slice(&std::fs::read(result).unwrap()).unwrap();
    assert_eq!(written, printed);
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
        &["next", "--roles", "--task", "t", "--no-run", "--no-brief"],
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

/// Makes `api` own every path, `paths = ["**"]`, as a single-root repo's draft does.
fn api_owns_everything(dir: &Path) {
    edit(
        &dir.join(".stewards/units.toml"),
        "paths = [\"src/api/**\", \"tests/api/**\"]",
        "paths = [\"**\"]",
    );
}

/// Each telemetry file with its bytes, to show a refused submit appended nothing.
fn telemetry(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let Ok(list) = fs::read_dir(dir.join(".stewards/telemetry")) else {
        return vec![];
    };
    let mut files: Vec<(String, Vec<u8>)> = list
        .map(|e| {
            let e = e.unwrap();
            (
                e.file_name().into_string().unwrap(),
                fs::read(e.path()).unwrap(),
            )
        })
        .collect();
    files.sort();
    files
}

/// Asserts a developer submit for task 1 is `no_tasks` and writes nothing: the brief stays and no
/// telemetry is appended.
fn developer_submit_is_no_tasks(dir: &Path) {
    let before = telemetry(dir);
    let (code, v) = submit_developer(dir, 1, &developer_answer(None));
    assert_eq!((code, &v["error"]["code"]), (2, &json!("no_tasks")), "{v}");
    assert!(dir.join(".stewards/briefs/api-developer-1.md").is_file());
    assert_eq!(telemetry(dir), before);
}

#[test]
fn a_git_hook_in_a_lane_is_rejected_at_maintainer_submit() {
    let dir = fixture("single");
    api_owns_everything(dir.path());
    let (exit, v) = plan(dir.path(), json!([task("a", &[".git/hooks/pre-commit"])]));
    assert_eq!(
        (exit, &v["error"]["code"]),
        (2, &json!("answer_parse_failed")),
        "{v}"
    );
    assert_eq!(v["error"]["errors"][0]["field"], "tasks[0].files[0]", "{v}");
    assert!(!dir.path().join(TASKS).exists());
}

#[test]
fn a_git_hook_edited_into_the_task_list_writes_no_brief_and_fails_developer_submit() {
    let dir = fixture("single");
    api_owns_everything(dir.path());
    assert_eq!(
        plan(dir.path(), json!([task("a", &["src/api/items.py"])])).0,
        0
    );
    let tasks = fs::read_to_string(dir.path().join(TASKS)).unwrap();
    fs::write(
        dir.path().join(TASKS),
        tasks.replace("\"src/api/items.py\"", "\".git/hooks/pre-commit\""),
    )
    .unwrap();
    let (code, v) = developers(dir.path());
    assert_eq!((code, &v["error"]["code"]), (2, &json!("no_tasks")), "{v}");
    assert!(
        !dir.path()
            .join(".stewards/briefs/api-developer-1.md")
            .exists()
    );
    // the brief written while the list was clean, then the hook edited in
    fs::write(dir.path().join(TASKS), &tasks).unwrap();
    assert_eq!(developers(dir.path()).0, 0);
    edit(
        &dir.path().join(TASKS),
        "\"src/api/items.py\"",
        "\"src/api/items.py\", \".git/hooks/pre-commit\"",
    );
    developer_submit_is_no_tasks(dir.path());
    assert!(!dir.path().join(".git/hooks/pre-commit").exists());
}

#[test]
fn the_checked_write_step_refuses_a_task_list_changed_since_it_was_recorded() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().to_path_buf();
    let briefs = root.join(".stewards/briefs");
    fs::create_dir_all(root.join(".stewards/units")).unwrap();
    fs::create_dir_all(&briefs).unwrap();
    let developer_brief = ".stewards/briefs/api-developer-1.md";
    let brief = ".stewards/briefs/api-maintainer.md";
    for (rel, text) in [
        (CARD, "card"),
        (TASKS, "first"),
        (developer_brief, "d"),
        (brief, "b"),
    ] {
        fs::write(root.join(rel), text).unwrap();
    }
    let rels = [CARD, TASKS].map(String::from);
    let before = record(&root, &rels).unwrap();
    // another explain answer writes its task list in between
    fs::write(root.join(TASKS), "another submit's").unwrap();
    let repo = Repo {
        name: "r".into(),
        root: root.clone(),
        loaded: None,
    };
    let changes = [
        (CARD.to_string(), Some("stale".to_string())),
        (TASKS.to_string(), None),
        (developer_brief.to_string(), None),
    ];
    let e = write_remove_and_retire(&repo, &before, &[CARD, TASKS], &changes, brief)
        .expect_err("card_changed");
    let e = e.downcast_ref::<CliError>().unwrap();
    assert_eq!(e.code, "card_changed");
    assert!(e.message.contains(TASKS), "{}", e.message);
    assert_eq!(fs::read(root.join(CARD)).unwrap(), b"card");
    assert_eq!(fs::read(root.join(TASKS)).unwrap(), b"another submit's");
    assert!(root.join(developer_brief).is_file());
    assert!(root.join(brief).is_file());
}

#[test]
fn the_checked_write_step_writes_and_removes_unchanged_files() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().to_path_buf();
    fs::create_dir_all(root.join(".stewards/units")).unwrap();
    fs::create_dir_all(root.join(".stewards/briefs")).unwrap();
    let brief = ".stewards/briefs/api-maintainer.md";
    fs::write(root.join(TASKS), "tasks").unwrap();
    fs::write(root.join(brief), "b").unwrap();
    let before = record(&root, &[CARD, TASKS].map(String::from)).unwrap();
    let repo = Repo {
        name: "r".into(),
        root: root.clone(),
        loaded: None,
    };
    let changes = [
        (CARD.to_string(), Some("new".to_string())),
        (TASKS.to_string(), None),
    ];
    write_remove_and_retire(&repo, &before, &[CARD, TASKS], &changes, brief).unwrap();
    assert_eq!(fs::read(root.join(CARD)).unwrap(), b"new");
    assert!(!root.join(TASKS).exists());
    assert!(!root.join(brief).exists());
}

/// Adds the unit `api-v2`, whose name starts with `api`, and one developer brief and answer of
/// its own, plus files named like api's job files that are not; returns them all.
fn neighbours(dir: &Path) -> Vec<std::path::PathBuf> {
    append(
        &dir.join(".stewards/units.toml"),
        "\n[[unit]]\nname = \"api-v2\"\npurpose = \"next api\"\npaths = [\"src/api_v2/**\"]\n",
    );
    let briefs = dir.join(".stewards/briefs");
    let files: Vec<std::path::PathBuf> = [
        "api-v2-developer-1.md",
        "api-v2-developer-3.md",
        "api-v2-developer-3.answer.json",
        "api-developer-01.md",
        "api-developer-03.md",
        "api-developer-x.md",
    ]
    .iter()
    .map(|n| briefs.join(n))
    .collect();
    for f in &files {
        fs::write(f, "keep").unwrap();
    }
    files
}

#[test]
fn an_approving_review_removes_every_developer_brief_and_answer_of_the_unit_only() {
    let dir = fixture("single");
    two_developers(dir.path());
    let kept = neighbours(dir.path());
    let briefs = dir.path().join(".stewards/briefs");
    fs::write(briefs.join("api-developer-1.answer.json"), "{}").unwrap();
    let (code, v) = review(dir.path(), "approve", json!([]));
    assert_eq!(code, 0, "{v}");
    assert!(!dir.path().join(TASKS).exists());
    for name in [
        "api-developer-1.md",
        "api-developer-2.md",
        "api-developer-1.answer.json",
    ] {
        assert!(!briefs.join(name).exists(), "{name}");
    }
    for f in &kept {
        assert_eq!(fs::read(f).unwrap(), b"keep", "{}", f.display());
    }
}

#[test]
fn an_explanation_with_no_tasks_removes_the_developer_briefs() {
    let dir = fixture("single");
    two_developers(dir.path());
    let kept = neighbours(dir.path());
    let (code, v) = plan(dir.path(), json!([]));
    assert_eq!(code, 0, "{v}");
    assert!(!dir.path().join(TASKS).exists());
    let briefs = dir.path().join(".stewards/briefs");
    assert!(!briefs.join("api-developer-1.md").exists());
    assert!(!briefs.join("api-developer-2.md").exists());
    for f in &kept {
        assert!(f.is_file(), "{}", f.display());
    }
}

#[test]
fn a_shorter_task_list_removes_brief_3_of_3_only_and_leaves_other_units_alone() {
    let dir = fixture("single");
    let tasks = json!([
        task("count", &["src/api/items.py"]),
        task("test count", &["tests/api/test_items.py"]),
        task("new", &["src/api/new.py"])
    ]);
    assert_eq!(plan(dir.path(), tasks).0, 0);
    assert_eq!(developers(dir.path()).0, 0);
    let kept = neighbours(dir.path());
    let briefs = dir.path().join(".stewards/briefs");
    fs::write(briefs.join("api-developer-3.answer.json"), "{}").unwrap();
    let two = json!([
        task("count", &["src/api/items.py"]),
        task("test count", &["tests/api/test_items.py"])
    ]);
    assert_eq!(plan(dir.path(), two).0, 0);
    let (code, v) = developers(dir.path());
    assert_eq!(code, 0, "{v}");
    assert!(briefs.join("api-developer-1.md").is_file());
    assert!(briefs.join("api-developer-2.md").is_file());
    assert!(!briefs.join("api-developer-3.md").exists());
    assert!(!briefs.join("api-developer-3.answer.json").exists());
    for f in &kept {
        assert_eq!(fs::read(f).unwrap(), b"keep", "{}", f.display());
    }
}

#[test]
fn a_task_list_edited_after_the_brief_to_add_an_unowned_file_fails_developer_submit() {
    let dir = fixture("single");
    two_developers(dir.path());
    edit(
        &dir.path().join(TASKS),
        "\"src/api/items.py\"",
        "\"src/api/items.py\", \"src/store/db.py\"",
    );
    developer_submit_is_no_tasks(dir.path());
}

#[test]
fn a_change_in_another_unit_is_neither_counted_nor_out_of_lane() {
    let dir = fixture("single");
    two_developers(dir.path());
    append(&dir.path().join("src/store/db.py"), "# store\n");
    let (code, v) = submit_developer(dir.path(), 1, &developer_answer(None));
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["summary"], "api developer 1: 0 files; check pass");
    assert_eq!(v["out_of_lane"], json!([]));
}

#[test]
fn a_developer_submit_for_task_1_leaves_task_2s_brief() {
    let dir = fixture("single");
    two_developers(dir.path());
    let briefs = dir.path().join(".stewards/briefs");
    let brief_2 = fs::read(briefs.join("api-developer-2.md")).unwrap();
    let (code, v) = submit_developer(dir.path(), 1, &developer_answer(None));
    assert_eq!(code, 0, "{v}");
    assert!(!briefs.join("api-developer-1.md").exists());
    assert_eq!(
        fs::read(briefs.join("api-developer-2.md")).unwrap(),
        brief_2
    );
}

#[test]
fn a_changes_review_with_no_findings_is_rejected() {
    let dir = fixture("single");
    let (code, v) = review(dir.path(), "changes", json!([]));
    assert_eq!(
        (code, &v["error"]["code"]),
        (2, &json!("answer_parse_failed")),
        "{v}"
    );
    assert_eq!(v["error"]["errors"][0]["field"], "verdict", "{v}");
    assert!(!dir.path().join(TASKS).exists());
    assert!(
        dir.path()
            .join(".stewards/briefs/api-maintainer.md")
            .is_file()
    );
}

#[test]
fn answer_files_not_named_for_a_task_of_the_unit_are_usage_errors() {
    let dir = fixture("single");
    two_developers(dir.path());
    for name in [
        "api-developer-01.answer.json",
        "api-developer-0.answer.json",
        "store-developer-1.answer.json",
        "api-developer-1.answer.json.bak",
    ] {
        let (code, v) = submit_developer_as(dir.path(), name, &developer_answer(None));
        assert_eq!(
            (code, &v["error"]["code"]),
            (2, &json!("usage")),
            "{name}: {v}"
        );
    }
    assert!(
        dir.path()
            .join(".stewards/briefs/api-developer-1.md")
            .is_file()
    );
}

#[test]
fn a_task_list_edited_to_an_absolute_a_stewards_or_a_shared_lane_file_is_no_tasks() {
    let dir = fixture("single");
    let absolute = dir.path().join("src/api/items.py");
    let absolute = format!("\"{}\"", absolute.display());
    for (from, to) in [
        ("\"src/api/items.py\"", absolute.as_str()),
        ("\"src/api/items.py\"", "\".stewards/units/api.toml\""),
        (
            "\"tests/api/test_items.py\"",
            "\"tests/api/test_items.py\", \"src/api/items.py\"",
        ),
    ] {
        let tasks = json!([
            task("count", &["src/api/items.py"]),
            task("test count", &["tests/api/test_items.py"])
        ]);
        assert_eq!(plan(dir.path(), tasks).0, 0);
        edit(&dir.path().join(TASKS), from, to);
        let (code, v) = developers(dir.path());
        assert_eq!(
            (code, &v["error"]["code"]),
            (2, &json!("no_tasks")),
            "{to}: {v}"
        );
        assert!(
            !dir.path()
                .join(".stewards/briefs/api-developer-1.md")
                .exists()
        );
    }
}

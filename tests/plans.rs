// Architect plans and flow checks in `next` and `review`: step checks run, results stored,
// plans closed; flow checks run while a unit of the flow is stale.
#[allow(dead_code)] // each test crate uses a subset of the helpers
mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{append, edit, run, stamp_units};
use serde_json::{Value, json};
use tempfile::TempDir;

const PLAN: &str = ".stewards/plans/2026-11-15-add-count.toml";

/// The `single` fixture with `sh` allowlisted, docs on store's card (so no documenter job),
/// and both units stamped; and a directory outside the repo for check scripts.
fn setup() -> (TempDir, TempDir) {
    let dir = common::fixture("single");
    let d = dir.path();
    edit(
        &d.join(".stewards/repo.toml"),
        "test = [\"pytest\"]",
        "test = [\"pytest\", \"sh\"]",
    );
    append(
        &d.join(".stewards/units/store.toml"),
        "\n[doc]\noverview = \"Keeps the items.\"\n",
    );
    stamp_units(d, &["api", "store"]);
    (dir, TempDir::new().unwrap())
}

/// Writes `scripts/<name>.sh` exiting with `code` after printing a line; its check command.
fn script(scripts: &Path, name: &str, code: i32) -> String {
    let path: PathBuf = scripts.join(format!("{name}.sh"));
    fs::write(&path, format!("echo {name} ran\nexit {code}\n")).unwrap();
    format!("sh {}", path.display())
}

fn write_plan(d: &Path, steps: &[(&str, &str)]) {
    let mut text = "title = \"add count\"\ncreated = \"2026-11-15\"\n".to_string();
    for (unit, check) in steps {
        text += &format!(
            "[[step]]\nunit = \"{unit}\"\nchange = \"add count to {unit}\"\ninterfaces = []\ncheck = \"{check}\"\n"
        );
    }
    fs::create_dir_all(d.join(".stewards/plans")).unwrap();
    fs::write(d.join(PLAN), text).unwrap();
}

fn touch_api(d: &Path) {
    edit(&d.join("src/api/items.py"), "router = []", "router = [1]");
}

fn touch_store(d: &Path) {
    fs::write(d.join("src/store/README.md"), "changed\n").unwrap();
}

fn next(d: &Path, extra: &[&str]) -> (i32, Value) {
    let mut args = vec!["next", "--no-brief", "--task", "t"];
    args.extend(extra);
    run(d, &args)
}

fn stored(d: &Path) -> String {
    fs::read_to_string(d.join(PLAN)).unwrap()
}

#[test]
fn a_stale_units_step_check_runs_and_stores_pass() {
    let (dir, scripts) = setup();
    let d = dir.path();
    write_plan(d, &[("api", &script(scripts.path(), "ok", 0))]);
    touch_api(d);
    let (_, v) = next(d, &[]);
    let plan = &v["plans"][0];
    assert_eq!(plan["id"], "2026-11-15-add-count", "{v}");
    assert_eq!(plan["closed"], false);
    let step = &plan["steps"][0];
    assert_eq!(
        (&step["unit"], &step["result"], &step["fresh"]),
        (&json!("api"), &json!("pass"), &json!(false)),
        "{v}"
    );
    assert_eq!(step["output"], "ok ran\n");
    // a passed step has nothing left to do
    assert!(step.get("todo").is_none(), "{v}");
    assert!(stored(d).contains("result = \"pass\""), "{}", stored(d));
}

#[test]
fn a_failing_step_stores_fail_exits_1_and_runs_again_after_the_unit_is_stamped() {
    let (dir, scripts) = setup();
    let d = dir.path();
    let check = script(scripts.path(), "step", 1);
    write_plan(d, &[("api", &check)]);
    touch_api(d);
    let (code, v) = next(d, &[]);
    assert_eq!(code, 1);
    let step = &v["plans"][0]["steps"][0];
    assert_eq!(step["result"], "fail", "{v}");
    assert_eq!(
        step["todo"],
        "make the change yourself, or run stewards brief api --role maintainer --task \
         \"<change>\" to split it into developer tasks"
    );
    assert!(stored(d).contains("result = \"fail\""));

    // stamped, the unit is fresh, but a failed step runs again; now it passes and the plan closes
    stamp_units(d, &["api"]);
    script(scripts.path(), "step", 0);
    let (code, v) = next(d, &[]);
    let plan = &v["plans"][0];
    assert_eq!(plan["steps"][0]["output"], "step ran\n", "{v}");
    assert_eq!(plan["steps"][0]["result"], "pass");
    assert_eq!(plan["closed"], true);
    assert!(!d.join(PLAN).exists());
    assert_eq!(code, 0, "{v}");
}

#[test]
fn a_plan_whose_steps_all_pass_on_fresh_units_is_deleted_and_reported_closed() {
    let (dir, scripts) = setup();
    let d = dir.path();
    let ok = script(scripts.path(), "ok", 0);
    write_plan(d, &[("api", &ok), ("store", &ok)]);
    touch_api(d);
    touch_store(d);
    let (_, v) = next(d, &[]);
    assert_eq!(v["plans"][0]["closed"], false, "{v}");
    stamp_units(d, &["api", "store"]);
    let (_, v) = next(d, &[]);
    let plan = &v["plans"][0];
    assert_eq!(plan["closed"], true, "{v}");
    // fresh and passed: not run again
    assert!(plan["steps"][0].get("output").is_none(), "{v}");
    assert!(!d.join(PLAN).exists());
    let (_, v) = next(d, &[]);
    assert_eq!(v["plans"], json!([]));
}

#[test]
fn no_run_runs_nothing_and_writes_nothing() {
    let (dir, scripts) = setup();
    let d = dir.path();
    let fail = script(scripts.path(), "fail", 1);
    write_plan(d, &[("api", &fail)]);
    edit(
        &d.join(".stewards/repo.toml"),
        "entry = \"src/api/items.py::create_item\"",
        &format!("entry = \"src/api/items.py::create_item\"\ncheck = \"{fail}\""),
    );
    touch_api(d);
    let before = stored(d);
    let (_, v) = next(d, &["--no-run"]);
    let step = &v["plans"][0]["steps"][0];
    assert_eq!(step["result"], Value::Null, "{v}");
    assert!(step.get("output").is_none());
    assert_eq!(v["flow_checks"], json!([]));
    assert_eq!(stored(d), before);
}

#[test]
fn a_flow_check_runs_only_while_a_unit_of_the_flow_is_stale_and_a_failing_one_exits_1() {
    let (dir, scripts) = setup();
    let d = dir.path();
    let fail = script(scripts.path(), "flow", 1);
    edit(
        &d.join(".stewards/repo.toml"),
        "entry = \"src/api/items.py::create_item\"",
        &format!("entry = \"src/api/items.py::create_item\"\ncheck = \"{fail}\""),
    );
    let (code, v) = next(d, &[]);
    assert_eq!(v["flow_checks"], json!([]), "{v}");
    assert_eq!(code, 0, "{v}");
    touch_api(d);
    let (code, v) = next(d, &[]);
    assert_eq!(code, 1);
    assert_eq!(
        v["flow_checks"],
        json!([{"flow": "create-item", "command": fail, "pass": false, "output": "flow ran\n"}])
    );
}

#[test]
fn the_architect_job_is_left_out_only_when_an_open_plan_names_every_moved_unit() {
    let (dir, scripts) = setup();
    let d = dir.path();
    touch_api(d);
    touch_store(d);
    let architect = |d: &Path| {
        let (_, v) = run(d, &["next", "--roles", "--no-brief", "--no-run"]);
        v["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|j| j["role"] == "architect")
    };
    assert!(architect(d));
    let ok = script(scripts.path(), "ok", 0);
    write_plan(d, &[("api", &ok)]);
    assert!(
        architect(d),
        "a plan naming fewer units does not cover them"
    );
    write_plan(d, &[("api", &ok), ("store", &ok)]);
    assert!(!architect(d));
}

#[test]
fn an_invalid_plan_file_is_invalid_toml_naming_its_path() {
    let (dir, _) = setup();
    let d = dir.path();
    fs::create_dir_all(d.join(".stewards/plans")).unwrap();
    fs::write(d.join(PLAN), "title = \"x\"\n[[step]]\nunit = 3\n").unwrap();
    let (code, v) = next(d, &[]);
    assert_eq!(code, 2);
    assert_eq!(v["error"]["code"], "invalid_toml", "{v}");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .starts_with(".stewards/plans/2026-11-15-add-count.toml:"),
        "{v}"
    );
    write_plan(d, &[("nope", "sh x")]);
    let (_, v) = next(d, &[]);
    assert_eq!(v["error"]["code"], "invalid_toml", "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("`nope`"));
}

#[test]
fn a_plan_file_that_is_a_symlink_is_unsafe_path() {
    let (dir, scripts) = setup();
    let d = dir.path();
    let target = scripts.path().join("plan.toml");
    fs::write(
        &target,
        "title = \"x\"\ncreated = \"2026-11-15\"\nstep = []\n",
    )
    .unwrap();
    fs::create_dir_all(d.join(".stewards/plans")).unwrap();
    std::os::unix::fs::symlink(&target, d.join(PLAN)).unwrap();
    let (code, v) = next(d, &[]);
    assert_eq!(
        (code, &v["error"]["code"]),
        (2, &json!("unsafe_path")),
        "{v}"
    );
}

#[test]
fn review_says_to_fix_failing_checks_after_the_jobs_line() {
    let (dir, scripts) = setup();
    let d = dir.path();
    write_plan(d, &[("api", &script(scripts.path(), "fail", 1))]);
    touch_api(d);
    let (code, v) = run(d, &["review", "--task", "t"]);
    assert_eq!(code, 1);
    let then = v["then"].as_str().unwrap();
    assert!(
        then.starts_with("Start one subagent per handoff below"),
        "{v}"
    );
    assert!(
        then.ends_with(
            " Fix the failing checks listed under next.flow_checks and next.plans, then run \
             stewards review again."
        ),
        "{v}"
    );
}

#[test]
fn review_with_only_a_failed_step_left_says_so_alone_and_exits_1() {
    let (dir, scripts) = setup();
    let d = dir.path();
    write_plan(d, &[("api", &script(scripts.path(), "fail", 1))]);
    touch_api(d);
    next(d, &[]);
    stamp_units(d, &["api"]);
    let (code, v) = run(d, &["review", "--task", "t"]);
    assert_eq!(
        (code, &v["then"]),
        (
            1,
            &json!(
                "Fix the failing checks listed under next.flow_checks and next.plans, then run \
                 stewards review again."
            )
        ),
        "{v}"
    );
}

#[test]
fn review_is_done_with_an_open_plan_and_names_it() {
    let (dir, scripts) = setup();
    let d = dir.path();
    write_plan(d, &[("api", &script(scripts.path(), "ok", 0))]);
    let (code, v) = run(d, &["review", "--task", "t"]);
    assert_eq!(
        (code, &v["then"]),
        (
            0,
            &json!("done. Plan 2026-11-15-add-count has steps left; see next.plans.")
        ),
        "{v}"
    );
    let step = &v["next"]["plans"][0]["steps"][0];
    assert_eq!(
        (&step["fresh"], &step["result"]),
        (&json!(true), &Value::Null)
    );
}

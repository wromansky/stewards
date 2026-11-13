// `stewards review` and `--run`: each brief run in a harness session, with a fake `pi` on PATH
// that reads the brief path from its prompt and runs a canned `stewards submit`, or does not.
#[allow(dead_code)] // each test crate uses a subset of the helpers
mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};

use common::{append, edit, fixture, json, run, stamp_units};
use serde_json::{Value, json};
use stewards::harness::{HARNESSES, Setting};
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_stewards");

/// The fake harness. For the brief `<name>.md` it records its argv, environment, and the order
/// of launches under `$FAKE_DIR`, then by `$FAKE_DIR/<name>.mode`: `silent` says something and
/// exits; `sleep` sleeps with a background child; `nested` runs `stewards review` itself; anything
/// else copies `$FAKE_DIR/<name>.json` (`SNAPSHOT` replaced by the brief's) to the answer path
/// and runs `stewards submit` for the brief's unit and role. Every mode then prints one Pi
/// `message_end` line with usage.
const FAKE_PI: &str = r#"#!/bin/sh
for a in "$@"; do prompt=$a; done
brief=${prompt#Read }
brief=${brief% and follow it.}
name=$(basename "$brief" .md)
printf '%s\n' "$@" > "$FAKE_DIR/$name.args"
env > "$FAKE_DIR/$name.env"
pwd > "$FAKE_DIR/$name.pwd"
echo "$name" >> "$FAKE_DIR/order"
said="submitted"
case "$(cat "$FAKE_DIR/$name.mode" 2>/dev/null)" in
  silent) said="I looked at it and decided not to submit." ;;
  sleep) sleep 30 & echo $! > "$FAKE_DIR/$name.bgpid"; sleep 30 ;;
  nested) "$STEWARDS" review > "$FAKE_DIR/$name.nested" ;;
  *)
    if [ -f "$FAKE_DIR/$name.json" ]; then
      head=$(head -n 1 "$brief")
      snap=$(printf '%s' "$head" | sed -n 's/.*"snapshot":"\([0-9a-f]*\)".*/\1/p')
      unit=$(printf '%s' "$head" | sed -n 's/.*"unit":"\([a-z0-9_-]*\)".*/\1/p')
      role=$(printf '%s' "$head" | sed -n 's/.*"role":"\([a-z]*\)".*/\1/p')
      answer="${brief%.md}.answer.json"
      sed "s/SNAPSHOT/$snap/" "$FAKE_DIR/$name.json" > "$answer"
      "$STEWARDS" submit $unit --role "$role" --file "$answer" > "$FAKE_DIR/$name.out"
    fi ;;
esac
echo "{\"type\":\"message_end\",\"message\":{\"role\":\"assistant\",\"usage\":{\"input\":1,\"output\":2,\"cacheRead\":3,\"cacheWrite\":4},\"content\":[{\"type\":\"text\",\"text\":\"$said\"}]}}"
"#;

struct Fake {
    bin: TempDir,
    dir: TempDir,
    home: TempDir,
}

impl Fake {
    /// The fake installed as `pi`.
    fn new() -> Fake {
        Fake::named("pi")
    }

    /// The fake installed as `program`.
    fn named(program: &str) -> Fake {
        let bin = TempDir::new().unwrap();
        let fake = bin.path().join(program);
        fs::write(&fake, FAKE_PI).unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        Fake {
            bin,
            dir: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
        }
    }

    fn answer(&self, name: &str, answer: Value) {
        fs::write(
            self.dir.path().join(format!("{name}.json")),
            answer.to_string(),
        )
        .unwrap();
    }

    fn mode(&self, name: &str, mode: &str) {
        fs::write(self.dir.path().join(format!("{name}.mode")), mode).unwrap();
    }

    fn file(&self, name: &str) -> String {
        fs::read_to_string(self.dir.path().join(name)).unwrap_or_default()
    }

    fn order(&self) -> Vec<String> {
        self.file("order").lines().map(String::from).collect()
    }

    /// `stewards <args>` in `d` with the fake first on PATH, no harness variables from this
    /// process, and `env` on top.
    fn command(&self, d: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
        let mut cmd = Command::new(BIN);
        cmd.args(args).current_dir(d);
        for (k, _) in std::env::vars_os() {
            let k = k.to_string_lossy().into_owned();
            let harness = ["PI_", "CLAUDE", "CODEX_", "HERMES_", "STEWARDS_"]
                .iter()
                .any(|p| k.starts_with(p));
            if harness {
                cmd.env_remove(&k);
            }
        }
        let path = format!(
            "{}:{}",
            self.bin.path().display(),
            std::env::var("PATH").unwrap()
        );
        cmd.env("PATH", path)
            .env("STEWARDS_HOME", self.home.path())
            .env("FAKE_DIR", self.dir.path())
            .env("STEWARDS", BIN);
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.output().unwrap()
    }

    fn run(&self, d: &Path, args: &[&str]) -> (i32, Value) {
        json(self.command(d, args, &[]))
    }
}

/// The single fixture with both units stamped and documented: `next` lists nothing.
fn clean() -> TempDir {
    let dir = fixture("single");
    let d = dir.path();
    append(
        &d.join(".stewards/units/store.toml"),
        "[doc]\noverview = \"o\"\nhow_it_works = \"h\"\nhow_to_change = \"c\"\n",
    );
    stamp_units(d, &["api", "store"]);
    dir
}

/// `clean` with `create_item` edited: one reviewer job, covering api.
fn api_changed() -> TempDir {
    let dir = clean();
    edit(
        &dir.path().join("src/api/items.py"),
        "return 422, None",
        "return 400, None",
    );
    dir
}

fn check() -> Value {
    json!({"command": "pytest tests", "pass": true, "output": "2 passed"})
}

fn review_answer(units: Value) -> Value {
    json!({"snapshot": "SNAPSHOT", "units": units, "pulled_anchors": [], "notes": ""})
}

fn approve(unit: &str) -> Value {
    json!({"unit": unit, "verdict": "approve", "findings": [], "card_patch": null,
           "check": check()})
}

fn changes(unit: &str, file: &str) -> Value {
    json!({"unit": unit, "verdict": "changes", "card_patch": null, "check": check(),
           "findings": [{"file": file, "line": 3, "rung": "one_line", "cut": "the loop",
                         "replace_with": "sum()"}]})
}

const PI: [&str; 2] = ["--harness", "pi"];

fn review(fake: &Fake, d: &Path, extra: &[&str]) -> (i32, Value) {
    let mut args = vec!["review", "--task", "t"];
    args.extend(PI);
    args.extend(extra);
    fake.run(d, &args)
}

#[test]
fn review_with_no_jobs_says_so_and_exits_0() {
    let dir = clean();
    let fake = Fake::new();
    let (code, v) = review(&fake, dir.path(), &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["jobs"], json!([]));
    assert_eq!(v["remaining"]["jobs"], json!([]));
    assert_eq!(v["then"], "done");
    assert!(fake.order().is_empty());
}

#[test]
fn an_approving_review_stamps_and_exits_0() {
    let dir = api_changed();
    let d = dir.path();
    let fake = Fake::new();
    fake.answer("review", review_answer(json!([approve("api")])));
    let (code, v) = review(&fake, d, &[]);
    assert_eq!(code, 0, "{v}");
    let job = &v["jobs"][0];
    assert_eq!(job["role"], "reviewer");
    assert_eq!(job["units"], json!(["api"]));
    assert_eq!(job["status"], "approved");
    assert_eq!(
        job["result"]["units"],
        json!([{"unit": "api", "verdict": "approve", "stamped": true, "check_reported": true}])
    );
    assert!(job["result"].get("findings").is_none(), "{job}");
    assert_eq!(v["then"], "done");
    assert_eq!(v["remaining"]["jobs"], json!([]));
    assert_eq!(
        job["usage"],
        json!({"input": 1, "output": 2, "cache_read": 3, "cache_write": 4})
    );
    // the brief was retired by the submit, and review removed the result file
    assert!(!d.join(".stewards/briefs/review.md").exists());
    assert!(!d.join(".stewards/briefs/review.result.json").exists());
    assert!(d.join(".stewards/briefs/review.log").is_file());
}

#[test]
fn a_review_with_findings_prints_them_and_says_fix_and_rerun() {
    let dir = api_changed();
    let fake = Fake::new();
    fake.answer(
        "review",
        review_answer(json!([changes("api", "src/api/items.py")])),
    );
    let (code, v) = review(&fake, dir.path(), &[]);
    assert_eq!(code, 1, "{v}");
    let job = &v["jobs"][0];
    assert_eq!(job["status"], "changes");
    assert_eq!(
        job["result"]["findings"],
        json!([{"unit": "api", "file": "src/api/items.py", "line": 3, "cut": "the loop",
                "replace_with": "sum()"}])
    );
    assert_eq!(v["then"], "fix the findings and run stewards review again");
    assert_eq!(v["remaining"]["jobs"][0]["role"], "reviewer");
}

#[test]
fn a_session_that_never_submits_is_not_submitted_with_its_last_words() {
    let dir = api_changed();
    let d = dir.path();
    let fake = Fake::new();
    fake.mode("review", "silent");
    let (code, v) = review(&fake, d, &[]);
    assert_eq!(code, 1, "{v}");
    let job = &v["jobs"][0];
    assert_eq!(job["status"], "not_submitted");
    assert_eq!(job["tail"], "I looked at it and decided not to submit.");
    assert!(job["result"].is_null());
    let log = fs::read_to_string(d.join(".stewards/briefs/review.log")).unwrap();
    assert!(log.contains("decided not to submit"), "{log}");
}

#[test]
fn a_session_over_its_timeout_is_killed_with_its_group() {
    let dir = api_changed();
    let fake = Fake::new();
    fake.mode("review", "sleep");
    let (code, v) = review(&fake, dir.path(), &["--timeout", "1"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["jobs"][0]["status"], "timed_out");
    let pid = fake.file("review.bgpid");
    let alive = Command::new("kill")
        .args(["-0", pid.trim()])
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success();
    assert!(!alive, "background child {pid} survived");
}

#[test]
fn a_unit_deferred_past_the_budget_is_reviewed_in_a_second_round() {
    let dir = clean();
    let d = dir.path();
    for f in ["src/api/items.py", "src/store/db.py"] {
        append(
            &d.join(f),
            &"# padding line for the review budget\n".repeat(900),
        );
    }
    let fake = Fake::new();
    fake.answer("review", review_answer(json!([approve("api")])));
    let (code, v) = review(&fake, d, &[]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["jobs"][0]["status"], "approved");
    assert_eq!(v["then"], "run stewards review again for the next round");
    let reasons = v["remaining"]["jobs"][0]["reasons"].to_string();
    assert!(reasons.contains("store"), "{reasons}");
    fake.answer("review", review_answer(json!([approve("store")])));
    let (code, v) = review(&fake, d, &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["jobs"][0]["units"], json!(["store"]));
}

#[test]
fn review_roles_runs_each_per_role_job_in_order() {
    let dir = api_changed();
    let d = dir.path();
    let (_, listed) = run(d, &["next", "--roles", "--no-brief", "--task", "t"]);
    let listed: Vec<String> = listed["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|j| {
            format!(
                "{}-{}",
                j["unit"].as_str().unwrap(),
                j["role"].as_str().unwrap()
            )
        })
        .collect();
    assert_eq!(listed, ["api-maintainer", "api-tester", "api-documenter"]);
    let fake = Fake::new();
    fake.answer(
        "api-maintainer",
        json!({"verdict": "approve", "findings": [], "card_patch": null,
               "tester_needed": false, "pulled_anchors": [], "notes": ""}),
    );
    fake.answer(
        "api-tester",
        json!({"scenario_patch": [], "results": [], "bugs": [], "tests_written": [],
               "pulled_anchors": [], "notes": ""}),
    );
    fake.answer(
        "api-documenter",
        json!({"doc": {"overview": "o", "how_it_works": "h", "how_to_change": "c"},
               "pulled_anchors": [], "notes": ""}),
    );
    let (code, v) = review(&fake, d, &["--roles"]);
    assert_eq!(fake.order(), listed);
    let statuses: Vec<&str> = v["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|j| j["status"].as_str().unwrap())
        .collect();
    assert_eq!(statuses, ["approved", "done", "done"], "{v}");
    assert_eq!(code, 0, "{v}");
}

#[test]
fn a_never_reviewed_units_first_review_runs_through_review() {
    let dir = fixture("single");
    let d = dir.path();
    let fake = Fake::new();
    for unit in ["api", "store"] {
        fake.answer(
            &format!("{unit}-maintainer"),
            json!({"verdict": "approve", "findings": [], "card_patch": null,
                   "tester_needed": false, "pulled_anchors": [], "notes": ""}),
        );
    }
    let (_, v) = review(&fake, d, &[]);
    assert_eq!(fake.order(), ["api-maintainer", "store-maintainer"]);
    for job in v["jobs"].as_array().unwrap() {
        assert_eq!(job["status"], "approved", "{v}");
    }
    let card = fs::read_to_string(d.join(".stewards/units/api.toml")).unwrap();
    assert!(card.contains("[reviewed]"), "{card}");
}

#[test]
fn inside_a_launched_session_review_and_run_do_nothing() {
    let dir = api_changed();
    let d = dir.path();
    let fake = Fake::new();
    for args in [
        &["review", "--harness", "pi"][..],
        &[
            "brief",
            "api",
            "--role",
            "maintainer",
            "--run",
            "--harness",
            "pi",
        ],
    ] {
        let (code, v) = json(fake.command(d, args, &[("STEWARDS_CHILD", "1")]));
        assert_eq!(code, 0, "{v}");
        assert!(
            v["skipped"].as_str().unwrap().contains("STEWARDS_CHILD"),
            "{v}"
        );
    }
    assert!(!d.join(".stewards/briefs").exists());
    assert!(fake.order().is_empty());
    // a session stewards launched that runs `stewards review` gets the same answer
    fake.mode("review", "nested");
    review(&fake, d, &[]);
    assert!(fake.file("review.nested").contains("\"skipped\""));
}

#[test]
fn no_harness_named_or_detected_is_no_launcher_with_the_manual_path() {
    let dir = api_changed();
    let fake = Fake::new();
    let (code, v) = fake.run(dir.path(), &["review"]);
    assert_eq!(code, 2, "{v}");
    assert_eq!(v["error"]["code"], "no_launcher");
    assert!(
        v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("with exactly: Read <absolute brief path> and follow it."),
        "{v}"
    );
}

/// Every row of the harness table through the same launcher: a row whose line is known starts
/// the fake under its program name with the fixed prompt last, its no-prompts and
/// no-instructions arguments, the repo root as its directory, and the guard; a row with a part
/// no source gives is `failed` with one message naming the part and the manual path.
#[test]
fn every_harness_row_launches_the_same_way() {
    let dir = api_changed();
    let d = dir.path();
    let root = d.canonicalize().unwrap();
    let brief = root.join(".stewards/briefs/review.md");
    for h in &HARNESSES {
        let l = &h.launch;
        let fake = Fake::named(l.program.unwrap_or("unknown-harness"));
        fake.mode("review", "silent");
        let (_, v) = fake.run(d, &["review", "--harness", h.name, "--model", "m1"]);
        let job = &v["jobs"][0];
        match l.argv(&root, Some("m1"), "p") {
            Err(part) => {
                assert_eq!(job["status"], "failed", "{}: {v}", h.name);
                assert_eq!(
                    job["tail"],
                    format!(
                        "the `{}` launch line has no known `{part}`, so stewards cannot start it",
                        h.name
                    )
                );
                assert_eq!(job["fix"], stewards::launch::MANUAL_PATH);
                assert!(fake.order().is_empty(), "{}", h.name);
            }
            Ok(_) => {
                assert_eq!(job["status"], "not_submitted", "{}: {v}", h.name);
                let args: Vec<String> =
                    fake.file("review.args").lines().map(String::from).collect();
                assert_eq!(
                    args.last().unwrap(),
                    &format!("Read {} and follow it.", brief.display()),
                    "{}",
                    h.name
                );
                let env = fake.file("review.env");
                for setting in [l.unprompted, l.no_instructions, l.model] {
                    match setting {
                        Setting::Args(want) => {
                            let want: Vec<String> =
                                want.iter().map(|a| a.replace("{id}", "m1")).collect();
                            assert!(
                                args.windows(want.len()).any(|w| w == want),
                                "{}: {args:?}",
                                h.name
                            );
                        }
                        Setting::Env(vars) => {
                            for (k, v) in vars {
                                let line = format!("{k}={v}");
                                assert!(env.lines().any(|l| l == line), "{}: {line}", h.name);
                            }
                        }
                        Setting::NotNeeded | Setting::Unknown => {}
                    }
                }
                if l.argv.contains(&"{dir}") {
                    assert!(args.contains(&root.display().to_string()), "{}", h.name);
                }
                assert_eq!(fake.file("review.pwd").trim(), root.display().to_string());
                assert!(
                    fake.file("review.env")
                        .lines()
                        .any(|l| l == "STEWARDS_CHILD=1")
                );
            }
        }
    }
}

#[test]
fn the_session_gets_the_fixed_prompt_the_guard_and_no_outer_session() {
    let dir = api_changed();
    let d = dir.path();
    let fake = Fake::new();
    let out = fake.command(
        d,
        &["review", "--harness", "pi"],
        &[
            ("PI_SESSION_FILE", "/outer/session.jsonl"),
            ("CLAUDE_CODE_ENTRYPOINT", "cli"),
            ("PI_CODING_AGENT_DIR", "/agent/dir"),
        ],
    );
    assert_eq!(json(out).0, 1);
    let args: Vec<String> = fake.file("review.args").lines().map(String::from).collect();
    let brief = d.canonicalize().unwrap().join(".stewards/briefs/review.md");
    assert_eq!(
        args.last().unwrap(),
        &format!("Read {} and follow it.", brief.display())
    );
    assert_eq!(args[args.len() - 2], "--");
    for flag in ["--no-context-files", "--no-extensions", "--no-skills"] {
        assert!(args.iter().any(|a| a == flag), "{args:?}");
    }
    let env = fake.file("review.env");
    assert!(env.lines().any(|l| l == "STEWARDS_CHILD=1"), "{env}");
    assert!(env.lines().any(|l| l == "PI_CODING_AGENT_DIR=/agent/dir"));
    for gone in ["PI_SESSION_FILE=", "CLAUDE_CODE_ENTRYPOINT="] {
        assert!(!env.lines().any(|l| l.starts_with(gone)), "{gone}");
    }
}

#[test]
fn submit_writes_its_result_file_and_review_removes_it() {
    let dir = api_changed();
    let d = dir.path();
    let (code, _) = run(d, &["next", "--task", "t"]);
    assert_eq!(code, 1);
    let snapshot = stewards::brief::read_header(&d.join(".stewards/briefs/review.md"))
        .unwrap()
        .snapshot;
    let answer = review_answer(json!([approve("api")]))
        .to_string()
        .replace("SNAPSHOT", &snapshot);
    fs::write(d.join(".stewards/briefs/review.answer.json"), answer).unwrap();
    let file = ".stewards/briefs/review.answer.json";
    let (code, printed) = run(d, &["submit", "--role", "reviewer", "--file", file]);
    assert_eq!(code, 0, "{printed}");
    let result: Value = serde_json::from_str(
        &fs::read_to_string(d.join(".stewards/briefs/review.result.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(result["summary"], printed["summary"]);
    assert_eq!(result["units"], printed["units"]);
    // the next launch of the job clears it first, and removes the one its submit writes
    let fake = Fake::new();
    edit(
        &d.join("src/api/items.py"),
        "return 400, None",
        "return 401, None",
    );
    fake.mode("review", "silent");
    review(&fake, d, &[]);
    assert!(!d.join(".stewards/briefs/review.result.json").exists());
}

#[test]
fn each_launched_job_appends_a_telemetry_record() {
    let dir = api_changed();
    let d = dir.path();
    let fake = Fake::new();
    fake.answer(
        "review",
        review_answer(json!([changes("api", "src/api/items.py")])),
    );
    review(&fake, d, &[]);
    let telemetry: Vec<Value> = fs::read_dir(d.join(".stewards/telemetry"))
        .unwrap()
        .flat_map(|e| {
            let text = fs::read_to_string(e.unwrap().path()).unwrap();
            text.lines()
                .map(|l| serde_json::from_str::<Value>(l).unwrap())
                .collect::<Vec<_>>()
        })
        .filter(|v| v["event"] == "launch")
        .collect();
    assert_eq!(telemetry.len(), 1);
    let r = &telemetry[0];
    assert_eq!(
        (&r["command"], &r["role"], &r["status"], &r["harness"]),
        (
            &json!("review"),
            &json!("reviewer"),
            &json!("changes"),
            &json!("pi")
        )
    );
    assert_eq!(r["units"], json!(["api"]));
    assert_eq!(r["usage"]["cache_write"], 4);
    assert_eq!(
        r["reviewed"],
        json!([{"unit": "api", "verdict": "changes", "findings": 1, "check_reported": true}])
    );
    assert!(r["seconds"].as_f64().is_some());
    // a launch is not counted as a submit
    let (_, status) = run(d, &["status", "--telemetry"]);
    assert_eq!(status["telemetry"]["submits"]["reviewer"], 1, "{status}");
}

#[test]
fn the_harness_and_model_come_from_the_user_config_when_not_given() {
    let dir = api_changed();
    let d = dir.path();
    let fake = Fake::new();
    fs::write(
        fake.home.path().join("config.toml"),
        "harness = \"pi\"\n[models.pi]\nreviewer = \"anthropic/some-model\"\n",
    )
    .unwrap();
    fake.mode("review", "silent");
    let (code, v) = fake.run(d, &["review"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["harness"], "pi");
    assert_eq!(
        v["task"].as_str().unwrap().split(';').next(),
        Some("(none given")
    );
    assert!(v["hint"].as_str().unwrap().contains("--task"));
    let args = fake.file("review.args");
    assert!(args.contains("--model\nanthropic/some-model\n"), "{args}");
}

fn brief_run(fake: &Fake, d: &Path, args: &[&str]) -> (i32, Value) {
    let mut all = vec!["brief"];
    all.extend(args);
    all.extend(["--run", "--harness", "pi"]);
    fake.run(d, &all)
}

#[test]
fn brief_run_runs_a_maintainer_explain_and_prints_its_result() {
    let dir = clean();
    let d = dir.path();
    let fake = Fake::new();
    fake.answer(
        "api-maintainer",
        json!({"explanation": "create_item validates, then saves.", "tasks": [],
               "card_patch": null, "pulled_anchors": [], "notes": ""}),
    );
    let (code, v) = brief_run(&fake, d, &["api", "--role", "maintainer", "--task", "how?"]);
    assert_eq!(code, 0, "{v}");
    let job = &v["jobs"][0];
    assert_eq!(job["status"], "done");
    assert_eq!(
        job["result"]["explanation"],
        "create_item validates, then saves."
    );
    assert!(!d.join(".stewards/briefs/api-maintainer.md").exists());
}

#[test]
fn brief_run_runs_the_architect() {
    let dir = clean();
    let fake = Fake::new();
    fake.answer(
        "architect",
        json!({"plan": null, "flows": [], "decisions": [], "unit_proposals": [],
               "explanation": "api then store.", "pulled_anchors": [], "notes": ""}),
    );
    let (code, v) = brief_run(
        &fake,
        dir.path(),
        &["--role", "architect", "--task", "plan"],
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["jobs"][0]["role"], "architect");
    assert_eq!(v["jobs"][0]["result"]["explanation"], "api then store.");
}

#[test]
fn brief_run_runs_every_developer_task_in_order() {
    let dir = clean();
    let d = dir.path();
    let task = |title: &str, file: &str| {
        json!({"title": title, "change": format!("do {title}"), "files": [file],
               "check": "pytest tests/api"})
    };
    let (code, _) = run(
        d,
        &["brief", "api", "--role", "maintainer", "--task", "count"],
    );
    assert_eq!(code, 0);
    let plan = json!({"explanation": "plan", "card_patch": null, "pulled_anchors": [],
                      "notes": "", "tasks": [task("count", "src/api/items.py"),
                                             task("test", "tests/api/test_items.py")]});
    let file = ".stewards/briefs/api-maintainer.answer.json";
    fs::write(d.join(file), plan.to_string()).unwrap();
    let submitted = run(
        d,
        &["submit", "api", "--role", "maintainer", "--file", file],
    );
    assert_eq!(submitted.0, 0, "{}", submitted.1);
    let fake = Fake::new();
    for n in [1, 2] {
        fake.answer(
            &format!("api-developer-{n}"),
            json!({"check": {"command": "pytest tests/api", "pass": true, "output": "ok"},
                   "summary": "done", "blocked_on": null, "pulled_anchors": [], "notes": ""}),
        );
    }
    let (code, v) = brief_run(&fake, d, &["api", "--role", "developer"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(fake.order(), ["api-developer-1", "api-developer-2"]);
    let summaries: Vec<&str> = v["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|j| j["result"]["summary"].as_str().unwrap())
        .collect();
    assert_eq!(
        summaries,
        [
            "api developer 1: 0 files; check pass",
            "api developer 2: 0 files; check pass"
        ]
    );
    assert_eq!(v["jobs"][0]["result"]["out_of_lane"], json!([]));
}

#[test]
fn brief_run_reports_a_session_that_never_submits_and_exits_1() {
    let dir = clean();
    let fake = Fake::new();
    fake.mode("api-maintainer", "silent");
    let (code, v) = brief_run(&fake, dir.path(), &["api", "--role", "maintainer"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["jobs"][0]["status"], "not_submitted");
}

#[test]
fn init_draft_run_runs_the_naming_job_and_the_first_reviews() {
    let dir = fixture("single");
    let d = dir.path();
    let fake = Fake::new();
    fake.answer(
        "architect",
        json!({"plan": null, "flows": [], "decisions": [], "unit_proposals": [],
               "explanation": "named.", "pulled_anchors": [], "notes": ""}),
    );
    for unit in ["api", "store"] {
        fake.answer(
            &format!("{unit}-maintainer"),
            json!({"verdict": "approve", "findings": [], "card_patch": null,
                   "tester_needed": false, "pulled_anchors": [], "notes": ""}),
        );
    }
    let (code, v) = fake.run(d, &["init", "--draft", "--run", "--harness", "pi"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        fake.order(),
        ["architect", "api-maintainer", "store-maintainer"]
    );
    assert_eq!(v["then"], "run `stewards review --roles` until it exits 0");
    let statuses: Vec<&Value> = v["launched"]
        .as_array()
        .unwrap()
        .iter()
        .map(|j| &j["status"])
        .collect();
    assert_eq!(
        statuses,
        [&json!("done"), &json!("approved"), &json!("approved")]
    );
}

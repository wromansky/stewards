#[allow(dead_code)] // each test crate uses a subset of the helpers
mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::{git, pairs, run};
use serde_json::{Value, json};
use tempfile::TempDir;

const UNITS: &str = ".stewards/units.toml";

/// A repo `proj` with `src/a` and `src/b`, three files each, changed apart so step 1 proposes
/// two units, `a` and `b`; file `f<i>.py` defines `thing<i>`.
fn repo() -> (TempDir, PathBuf) {
    repo_at("proj")
}

/// `repo` at `<temp>/<name>`.
fn repo_at(name: &str) -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join(name);
    fs::create_dir(&dir).unwrap();
    git(&dir, &["init", "-q"]);
    git(&dir, &["config", "user.name", "stewards-test"]);
    git(&dir, &["config", "user.email", "test@stewards.invalid"]);
    git(&dir, &["config", "commit.gpgsign", "false"]);
    for unit in ["a", "b"] {
        fs::create_dir_all(dir.join("src").join(unit)).unwrap();
        for i in 0..3 {
            let file = dir.join(format!("src/{unit}/f{i}.py"));
            fs::write(file, format!("def thing{i}():\n    pass\n")).unwrap();
        }
    }
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "initial"]);
    for round in 0..2 {
        for unit in ["a", "b"] {
            common::append(&dir.join(format!("src/{unit}/f0.py")), "# again\n");
            git(&dir, &["commit", "-q", "-am", &format!("{unit} {round}")]);
        }
    }
    (tmp, dir)
}

/// `(unit, role)` of each job.
fn jobs(v: &Value) -> Vec<(String, String)> {
    v["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|j| {
            (
                j["unit"].as_str().unwrap().into(),
                j["role"].as_str().unwrap().into(),
            )
        })
        .collect()
}

/// The files in the briefs dir, each brief's `-<snapshot>` cut from its name.
fn briefs(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir.join(".stewards/briefs"))
        .map(|d| {
            d.map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect()
        })
        .unwrap_or_default();
    for n in &mut names {
        if let Some((stem, snap)) = n.strip_suffix(".md").and_then(|s| s.rsplit_once('-'))
            && snap.len() == 8
        {
            *n = format!("{stem}.md");
        }
    }
    names.sort();
    names
}

/// Writes `answer` to the job's answer path, submits it, and returns submit's JSON.
fn submit(dir: &Path, unit: Option<&str>, role: &str, answer: &Value) -> Value {
    let stem = unit.map_or(role.to_string(), |u| format!("{u}-{role}"));
    let file = dir.join(common::answer(dir, &stem));
    fs::write(&file, answer.to_string()).unwrap();
    let file = file.to_str().unwrap();
    let mut args = vec!["submit"];
    args.extend(unit);
    args.extend(["--role", role, "--file", file]);
    let (code, v) = run(dir, &args);
    assert_eq!(code, 0, "{v}");
    v
}

fn architect(proposals: Value) -> Value {
    json!({"plan": null, "flows": [], "decisions": [], "unit_proposals": proposals,
           "explanation": "two units.", "pulled_anchors": [], "notes": ""})
}

fn proposal(name: &str, purpose: &str, paths: &[&str]) -> Value {
    json!({"name": name, "purpose": purpose, "paths": paths})
}

/// An approving first review; with `interface`, the drafted card has one entry anchored there.
fn approve(interface: Option<&str>) -> Value {
    let patch = interface.map(|anchor| {
        json!({"interface": [{"id": "thing", "anchor": anchor, "contract": "does the thing"}]})
    });
    json!({"verdict": "approve", "findings": [], "card_patch": patch, "tester_needed": false,
           "pulled_anchors": [], "notes": ""})
}

fn documenter() -> Value {
    json!({"doc": {"overview": "Does things.", "how_it_works": "Through thing0.",
                   "how_to_change": "Start at thing0."},
           "pulled_anchors": [], "notes": ""})
}

/// The documenter jobs `next` lists, as `(unit, reasons)`.
fn documenter_jobs(dir: &Path) -> Vec<(String, Value)> {
    let (_, v) = run(dir, &["next", "--roles"]);
    v["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|j| j["role"] == "documenter")
        .map(|j| (j["unit"].as_str().unwrap().into(), j["reasons"].clone()))
        .collect()
}

/// `init --draft` then an approving first review of `a` drafting one interface.
fn reviewed_with_interface() -> (TempDir, PathBuf) {
    let (tmp, dir) = repo();
    assert_eq!(run(&dir, &["init", "--draft"]).0, 0);
    submit(
        &dir,
        Some("a"),
        "maintainer",
        &approve(Some("src/a/f0.py::thing0")),
    );
    (tmp, dir)
}

/// Runs `stewards init --draft` plus `extra` with a STEWARDS_HOME holding `config`, no provider
/// key, no BAML library or download, and any HTTP sent to a closed port.
fn init_direct(dir: &Path, config: &str, extra: &[&str], envs: &[(&str, &str)]) -> (i32, Value) {
    let home = TempDir::new().unwrap();
    fs::write(home.path().join("config.toml"), config).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_stewards"));
    cmd.args(["init", "--draft"]).args(extra).current_dir(dir);
    for var in [
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "GOOGLE_API_KEY",
        "BAML_LIBRARY_PATH",
        "STEWARDS_BAML_DOWNLOAD",
        "STEWARDS_DIRECT",
    ] {
        cmd.env_remove(var);
    }
    let cache = TempDir::new().unwrap();
    cmd.env("STEWARDS_HOME", home.path())
        .env("BAML_CACHE_DIR", cache.path())
        .env("HTTPS_PROXY", "http://127.0.0.1:9")
        .env("HTTP_PROXY", "http://127.0.0.1:9")
        .env("ALL_PROXY", "http://127.0.0.1:9")
        .envs(envs.iter().copied());
    let out = cmd.output().unwrap();
    let v = serde_json::from_slice(&out.stdout).unwrap();
    (out.status.code().unwrap(), v)
}

#[test]
fn draft_on_a_fresh_repo_writes_step_1_and_the_architect_and_first_review_briefs() {
    let (_tmp, dir) = repo();
    let (code, v) = run(&dir, &["init", "--draft"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["units"].as_array().unwrap().len(), 2, "{v}");
    for f in [UNITS, ".stewards/repo.toml", "AGENTS.md"] {
        assert!(dir.join(f).is_file(), "{f}");
    }
    assert_eq!(
        jobs(&v),
        pairs(&[("", "architect"), ("a", "maintainer"), ("b", "maintainer")])
    );
    // adoption finishes in the per-role flow, and changes no existing code
    assert!(
        v["then"]
            .as_str()
            .unwrap()
            .contains("never asks for existing code to change"),
        "{v}"
    );
    assert!(
        v["then"]
            .as_str()
            .unwrap()
            .starts_with("start one subagent per job's handoff")
            && v["then"]
                .as_str()
                .unwrap()
                .ends_with("run `stewards review --roles` until it exits 0"),
        "{v}"
    );
    assert_eq!(
        v["jobs"][0]["task"],
        "name and describe each proposed unit; propose merges or splits only where a boundary is wrong"
    );
    assert_eq!(
        briefs(&dir),
        ["a-maintainer.md", "architect.md", "b-maintainer.md"]
    );
    for j in v["jobs"].as_array().unwrap() {
        let path = j["brief_path"].as_str().unwrap();
        assert!(Path::new(path).is_file(), "{j}");
        assert_eq!(j["handoff"], format!("Read `{path}` and follow it."));
    }
}

#[test]
fn every_command_that_writes_a_brief_hands_it_off_with_the_same_sentence() {
    let (_tmp, dir) = repo_at("my proj");
    let of = |v: &Value| {
        v["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|j| j["unit"] == "a")
            .unwrap_or_else(|| panic!("{v}"))["handoff"]
            .clone()
    };
    let (code, v) = run(&dir, &["init", "--draft"]);
    assert_eq!(code, 0, "{v}");
    let brief = dir
        .canonicalize()
        .unwrap()
        .join(common::brief(&dir, "a-maintainer"));
    // the path byte for byte, its space unquoted
    let want = format!("Read `{}` and follow it.", brief.display());
    assert!(want.contains("/my proj/.stewards/briefs/"), "{want}");
    assert_eq!(of(&v), want);
    let (_, v) = run(&dir, &["next", "--roles"]);
    assert_eq!(of(&v), want);
    let (_, v) = run(&dir, &["review", "--roles"]);
    assert_eq!(of(&v["next"]), want);
    let (code, v) = run(&dir, &["brief", "a", "--role", "maintainer"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["handoff"], want);
}

/// The text between the first brief's DATA markers.
fn data_section(brief: &str) -> &str {
    let start = brief.find("\n<<<DATA\n").unwrap() + "\n<<<DATA\n".len();
    let len = brief[start..].find("\nDATA>>>\n").unwrap();
    &brief[start..start + len]
}

#[test]
fn draft_first_review_briefs_carry_the_first_review_task_and_name_the_units_files() {
    let (_tmp, dir) = repo();
    let (code, v) = run(&dir, &["init", "--draft"]);
    assert_eq!(code, 0, "{v}");
    let task = "first review: read the unit's files, draft its card, record each problem as a gap, and approve; existing code is not changed";
    for (i, unit) in [(1, "a"), (2, "b")] {
        assert_eq!(v["jobs"][i]["task"], task, "{v}");
        let brief = common::brief(&dir, &format!("{unit}-maintainer"));
        let brief = fs::read_to_string(dir.join(brief)).unwrap();
        assert!(brief.contains(&format!("\nTask: {task}\n")), "{brief}");
        let data = data_section(&brief);
        assert!(
            data.contains(&format!("\n## Paths\n\n- src/{unit}/**\n")),
            "{data}"
        );
        let files: String = (0..3).map(|i| format!("- src/{unit}/f{i}.py\n")).collect();
        assert!(
            data.ends_with(&format!(
                "\n## Owned files\n\nThis unit has not been reviewed yet: these are all its files.\n\n{}",
                files.trim_end()
            )),
            "{data}"
        );
    }
}

#[test]
fn an_architect_answer_built_only_from_its_brief_sets_the_purposes() {
    let (_tmp, dir) = repo();
    assert_eq!(run(&dir, &["init", "--draft"]).0, 0);
    let brief = fs::read_to_string(dir.join(common::brief(&dir, "architect"))).unwrap();
    let units = data_section(&brief).split("\n## Units\n\n").nth(1).unwrap();
    let mut lines = units.lines();
    assert_eq!(
        lines.next(),
        Some("| name | purpose | interface ids | paths | files |")
    );
    let proposals: Vec<Value> = lines
        .skip(1)
        .take_while(|l| l.starts_with('|'))
        .map(|row| {
            let cells: Vec<&str> = row.split('|').map(str::trim).collect();
            assert_eq!(cells[5], "3", "{row}");
            let paths: Vec<&str> = cells[4].split(", ").collect();
            proposal(cells[1], &format!("Does {}.", cells[1]), &paths)
        })
        .collect();
    assert_eq!(proposals.len(), 2, "{units}");
    let v = submit(&dir, None, "architect", &architect(json!(proposals)));
    assert!(
        v["summary"]
            .as_str()
            .unwrap()
            .starts_with("architect: 2 purposes set; 0 unit proposals"),
        "{v}"
    );
    let units = fs::read_to_string(dir.join(UNITS)).unwrap();
    for unit in ["a", "b"] {
        let set = format!("name = \"{unit}\"\npurpose = \"Does {unit}.\"");
        assert!(units.contains(&set), "{units}");
    }
}

#[test]
fn draft_on_an_initialized_repo_writes_the_briefs_and_does_not_fail() {
    let (_tmp, dir) = repo();
    assert_eq!(run(&dir, &["init"]).0, 0);
    let units = fs::read(dir.join(UNITS)).unwrap();
    let (code, v) = run(&dir, &["init", "--draft"]);
    assert_eq!(code, 0, "{v}");
    assert!(v.get("units").is_none(), "{v}");
    assert_eq!(
        jobs(&v),
        pairs(&[("", "architect"), ("a", "maintainer"), ("b", "maintainer")])
    );
    assert_eq!(
        briefs(&dir),
        ["a-maintainer.md", "architect.md", "b-maintainer.md"]
    );
    assert_eq!(fs::read(dir.join(UNITS)).unwrap(), units);
}

#[test]
fn architect_proposals_matching_unnamed_units_set_their_purposes_and_nothing_else() {
    let (_tmp, dir) = repo();
    assert_eq!(run(&dir, &["init", "--draft"]).0, 0);
    // a hand comment and spacing that a re-serialization would drop
    let before = format!(
        "# my units\n\n{}",
        fs::read_to_string(dir.join(UNITS)).unwrap()
    );
    let before = before.replace("paths = [\"src/b/**\"]", "paths = [ \"src/b/**\" ]  # b");
    fs::write(dir.join(UNITS), &before).unwrap();
    let a = architect(json!([
        proposal("a", "Does A.", &["src/a/**"]),
        proposal("b", "Does B.", &["src/b/**"])
    ]));
    let v = submit(&dir, None, "architect", &a);
    assert_eq!(
        v["summary"],
        "architect: 2 purposes set; 0 unit proposals for you to review; 0 flows upserted; 0 decisions"
    );
    assert!(v.get("unit_proposals").is_none(), "{v}");
    let expected = before
        .replace(
            "name = \"a\"\npurpose = \"\"",
            "name = \"a\"\npurpose = \"Does A.\"",
        )
        .replace(
            "name = \"b\"\npurpose = \"\"",
            "name = \"b\"\npurpose = \"Does B.\"",
        );
    assert_ne!(expected, before);
    assert_eq!(fs::read_to_string(dir.join(UNITS)).unwrap(), expected);
}

#[test]
fn architect_proposals_that_rename_move_add_or_overwrite_change_nothing_and_are_printed() {
    let (_tmp, dir) = repo();
    assert_eq!(run(&dir, &["init", "--draft"]).0, 0);
    common::edit(
        &dir.join(UNITS),
        "name = \"b\"\npurpose = \"\"",
        "name = \"b\"\npurpose = \"Kept.\"",
    );
    let before = fs::read(dir.join(UNITS)).unwrap();
    let proposals = json!([
        proposal("a", "Does A.", &["src/a/**", "src/b/**"]),
        proposal("c", "Does C.", &["src/c/**"]),
        proposal("b", "Does B.", &["src/b/**"])
    ]);
    let v = submit(&dir, None, "architect", &architect(proposals.clone()));
    assert_eq!(
        v["summary"],
        "architect: 0 purposes set; 3 unit proposals for you to review; 0 flows upserted; 0 decisions"
    );
    assert_eq!(v["unit_proposals"], proposals);
    assert_eq!(fs::read(dir.join(UNITS)).unwrap(), before);
}

#[test]
fn a_first_review_drafting_an_interface_brings_a_documenter_job_with_no_docs_yet() {
    let (_tmp, dir) = reviewed_with_interface();
    assert_eq!(
        documenter_jobs(&dir),
        [("a".to_string(), json!(["no docs yet"]))]
    );
}

#[test]
fn a_documenter_submit_clears_the_no_docs_yet_job() {
    let (_tmp, dir) = reviewed_with_interface();
    assert_eq!(documenter_jobs(&dir).len(), 1);
    submit(&dir, Some("a"), "documenter", &documenter());
    assert_eq!(documenter_jobs(&dir), []);
}

#[test]
fn a_unit_with_no_interface_gets_no_documenter_job() {
    let (_tmp, dir) = repo();
    assert_eq!(run(&dir, &["init", "--draft"]).0, 0);
    submit(&dir, Some("a"), "maintainer", &approve(None));
    assert_eq!(documenter_jobs(&dir), []);
}

#[test]
fn a_scripted_draft_run_ends_with_a_green_check_and_no_jobs() {
    let (_tmp, dir) = repo();
    assert_eq!(run(&dir, &["init", "--draft"]).0, 0);
    let a = architect(json!([
        proposal("a", "Does A.", &["src/a/**"]),
        proposal("b", "Does B.", &["src/b/**"])
    ]));
    submit(&dir, None, "architect", &a);
    for unit in ["a", "b"] {
        let anchor = format!("src/{unit}/f0.py::thing0");
        submit(&dir, Some(unit), "maintainer", &approve(Some(&anchor)));
    }
    // adoption runs the per-role flow
    let (code, v) = run(&dir, &["next", "--roles"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(jobs(&v), pairs(&[("a", "documenter"), ("b", "documenter")]));
    for unit in ["a", "b"] {
        submit(&dir, Some(unit), "documenter", &documenter());
    }
    let (code, v) = run(&dir, &["check"]);
    assert_eq!(code, 0, "{v}");
    let (code, v) = run(&dir, &["next", "--roles"]);
    assert_eq!((code, jobs(&v)), (0, vec![]), "{v}");
    // after adoption, the plain per-change review has nothing to do either
    let (code, v) = run(&dir, &["next"]);
    assert_eq!((code, jobs(&v)), (0, vec![]), "{v}");
}

/// Adopting never asks for existing code to change: a first review's finding is refused, the
/// same problem recorded as a gap is accepted, and `review --roles` reaches exit 0 with the
/// source exactly as it was.
#[test]
fn adoption_records_problems_as_gaps_and_review_roles_exits_0_with_no_source_change() {
    let (_tmp, dir) = repo();
    let src_before = git(&dir, &["rev-parse", "HEAD:src"]);
    let (code, v) = run(&dir, &["init", "--draft"]);
    assert_eq!(code, 0, "{v}");
    let brief = fs::read_to_string(dir.join(common::brief(&dir, "a-maintainer"))).unwrap();
    assert!(
        brief.contains(&format!(
            "\n\n{}\n\nYou maintain unit `a`.",
            stewards::brief::FIRST_REVIEW
        )),
        "{brief}"
    );
    let a = architect(json!([
        proposal("a", "Does A.", &["src/a/**"]),
        proposal("b", "Does B.", &["src/b/**"])
    ]));
    submit(&dir, None, "architect", &a);

    let card = dir.join(".stewards/units/a.toml");
    let card_before = fs::read(&card).ok();
    let mut changes = approve(Some("src/a/f0.py::thing0"));
    changes["verdict"] = json!("changes");
    changes["findings"] = json!([{"file": "src/a/f1.py", "line": 2, "rung": "not_needed",
                                  "cut": "pass", "replace_with": "nothing"}]);
    let file = dir.join(common::answer(&dir, "a-maintainer"));
    fs::write(&file, changes.to_string()).unwrap();
    let args = [
        "submit",
        "a",
        "--role",
        "maintainer",
        "--file",
        file.to_str().unwrap(),
    ];
    let (code, v) = run(&dir, &args);
    assert_eq!(
        (code, &v["error"]["code"]),
        (2, &json!("answer_parse_failed")),
        "{v}"
    );
    let fields: Vec<&str> = v["error"]["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["field"].as_str().unwrap())
        .collect();
    assert_eq!(fields, ["verdict", "findings[0]"], "{v}");
    assert!(
        v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("card_patch.gaps"),
        "{v}"
    );
    assert_eq!(fs::read(&card).ok(), card_before, "nothing written");

    // the same problem as a gap: accepted, and the first review stamps
    let mut recorded = approve(Some("src/a/f0.py::thing0"));
    recorded["card_patch"]["gaps"] = json!([{"kind": "gap", "title": "thing1 does nothing",
        "anchor": "src/a/f1.py::thing1", "note": "its body is only pass"}]);
    submit(&dir, Some("a"), "maintainer", &recorded);
    let text = fs::read_to_string(&card).unwrap();
    assert!(
        text.contains("thing1 does nothing") && text.contains("[reviewed]"),
        "{text}"
    );
    submit(
        &dir,
        Some("b"),
        "maintainer",
        &approve(Some("src/b/f0.py::thing0")),
    );
    assert_eq!(run(&dir, &["next", "--roles"]).0, 1);
    for unit in ["a", "b"] {
        submit(&dir, Some(unit), "documenter", &documenter());
    }
    let (code, v) = run(&dir, &["review", "--roles", "--task", "adopt"]);
    assert_eq!((code, &v["then"]), (0, &json!("done")), "{v}");
    assert_eq!(git(&dir, &["status", "--porcelain", "--", "src"]), "");
    assert_eq!(git(&dir, &["rev-parse", "HEAD:src"]), src_before);

    // a second review of the now stamped unit may ask for changes as before
    common::append(&dir.join("src/a/f1.py"), "x = 1\n");
    let (code, v) = run(&dir, &["next", "--roles", "--task", "t"]);
    assert_eq!(code, 1, "{v}");
    let brief = fs::read_to_string(dir.join(common::brief(&dir, "a-maintainer"))).unwrap();
    assert!(!brief.contains(stewards::brief::FIRST_REVIEW), "{brief}");
    let mut second = approve(None);
    second["verdict"] = json!("changes");
    second["findings"] = json!([{"file": "src/a/f1.py", "line": 3, "rung": "untraced",
                                 "cut": "x = 1", "replace_with": "nothing"}]);
    let v = submit(&dir, Some("a"), "maintainer", &second);
    assert!(
        v["summary"]
            .as_str()
            .unwrap()
            .contains("a maintainer: changes"),
        "{v}"
    );
}

#[test]
fn draft_direct_with_no_direct_model_is_no_direct_model_and_writes_no_brief() {
    let (_tmp, dir) = repo();
    let (code, v) = init_direct(&dir, "", &["--direct"], &[]);
    assert_eq!(
        (code, &v["error"]["code"]),
        (2, &json!("no_direct_model")),
        "{v}"
    );
    assert!(dir.join(UNITS).is_file());
    assert_eq!(briefs(&dir), Vec::<String>::new());
}

#[test]
fn draft_direct_with_a_model_but_no_key_is_no_api_key_and_writes_no_brief() {
    let (_tmp, dir) = repo();
    let config = "[models.direct]\narchitect = \"anthropic/m\"\nmaintainer = \"anthropic/m\"\n";
    let (code, v) = init_direct(&dir, config, &[], &[("STEWARDS_DIRECT", "1")]);
    assert_eq!(
        (code, &v["error"]["code"]),
        (2, &json!("no_api_key")),
        "{v}"
    );
    assert_eq!(briefs(&dir), Vec::<String>::new());
}

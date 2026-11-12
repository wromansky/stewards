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

fn stamp(dir: &Path, units: &[&str]) {
    for unit in units {
        assert_eq!(run(dir, &["stamp", unit]).0, 0);
    }
}

fn edit(path: &Path, from: &str, to: &str) {
    let text = fs::read_to_string(path).unwrap();
    assert!(text.contains(from), "{from} not in {}", path.display());
    fs::write(path, text.replace(from, to)).unwrap();
}

/// `(unit, role)` of each job.
fn jobs(v: &Value) -> Vec<(String, String)> {
    v["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|j| {
            (
                j["unit"].as_str().unwrap().to_string(),
                j["role"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(u, r)| (u.to_string(), r.to_string()))
        .collect()
}

fn strings(v: &Value) -> Vec<&str> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect()
}

/// Replaces the `tree = ...` line of `unit`'s `[reviewed]` with `lines`.
fn set_tree_line(dir: &Path, unit: &str, lines: impl Fn(&str) -> String) {
    let path = dir.join(format!(".stewards/units/{unit}.toml"));
    let text = fs::read_to_string(&path).unwrap();
    let out: Vec<String> = text
        .lines()
        .map(|l| {
            if l.starts_with("tree = ") {
                lines(l)
            } else {
                l.to_string()
            }
        })
        .collect();
    fs::write(&path, out.join("\n") + "\n").unwrap();
}

fn set_tester_mark(dir: &Path, unit: &str, tree: &str) {
    set_tree_line(dir, unit, |l| format!("{l}\ntester = \"{tree}\""));
}

fn snapshot(dir: &Path) -> String {
    run(dir, &["next"]).1["jobs"][0]["snapshot"]
        .as_str()
        .unwrap()
        .to_string()
}

fn add_gaps(dir: &Path, unit: &str, gaps: &str) {
    let path = dir.join(format!(".stewards/units/{unit}.toml"));
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, format!("{text}\n{gaps}")).unwrap();
}

#[test]
fn never_stamped_unit_gets_a_maintainer_job_listing_all_its_files() {
    let repo = fixture("single");
    stamp(repo.path(), &["api"]);
    let (code, v) = run(repo.path(), &["next"]);
    assert_eq!(code, 1);
    let job = &v["jobs"][0];
    assert_eq!(
        (&job["unit"], &job["role"]),
        (&"store".into(), &"maintainer".into())
    );
    assert_eq!(
        strings(&job["changed"]),
        [
            "src/store/README.md",
            "src/store/db.py",
            "tests/store/test_db.py"
        ]
    );
    assert_eq!(job["command"], "stewards brief store --role maintainer");
    assert!(job["brief_path"].is_null());
}

#[test]
fn everything_stamped_and_unchanged_has_no_jobs() {
    let repo = fixture("single");
    stamp(repo.path(), &["api", "store"]);
    let (code, v) = run(repo.path(), &["next"]);
    assert_eq!(code, 0, "{v}");
    assert!(jobs(&v).is_empty());
    assert_eq!(v["architect"], false);
}

#[test]
fn non_interface_non_test_edit_gives_only_a_maintainer_job() {
    let repo = fixture("single");
    stamp(repo.path(), &["api", "store"]);
    fs::write(repo.path().join("src/store/README.md"), "changed\n").unwrap();
    let (code, v) = run(repo.path(), &["next"]);
    assert_eq!(code, 1);
    assert_eq!(jobs(&v), pairs(&[("store", "maintainer")]));
    assert_eq!(strings(&v["jobs"][0]["reasons"]), ["stale"]);
    assert_eq!(strings(&v["jobs"][0]["changed"]), ["src/store/README.md"]);
}

#[test]
fn interface_file_edit_gives_maintainer_tester_documenter() {
    let repo = fixture("single");
    stamp(repo.path(), &["api", "store"]);
    edit(
        &repo.path().join("src/store/db.py"),
        "_items = []",
        "_items = [0]",
    );
    let (_, v) = run(repo.path(), &["next"]);
    assert_eq!(
        jobs(&v),
        pairs(&[
            ("store", "maintainer"),
            ("store", "tester"),
            ("store", "documenter")
        ])
    );
    assert_eq!(
        strings(&v["jobs"][1]["reasons"]),
        ["interface file changed: src/store/db.py"]
    );
}

#[test]
fn test_root_edit_gives_maintainer_and_tester() {
    let repo = fixture("single");
    stamp(repo.path(), &["api", "store"]);
    edit(
        &repo.path().join("tests/store/test_db.py"),
        "== 1",
        "== 1  # first",
    );
    let (_, v) = run(repo.path(), &["next"]);
    assert_eq!(
        jobs(&v),
        pairs(&[("store", "maintainer"), ("store", "tester")])
    );
    assert_eq!(
        strings(&v["jobs"][1]["reasons"]),
        ["test file changed: tests/store/test_db.py"]
    );
}

#[test]
fn tester_mark_after_the_change_suppresses_the_tester_job() {
    let repo = fixture("single");
    stamp(repo.path(), &["api", "store"]);
    edit(
        &repo.path().join("src/store/db.py"),
        "_items = []",
        "_items = [0]",
    );
    let tree = snapshot(repo.path());
    set_tester_mark(repo.path(), "store", &tree);
    let (_, v) = run(repo.path(), &["next"]);
    assert_eq!(
        jobs(&v),
        pairs(&[("store", "maintainer"), ("store", "documenter")])
    );
}

#[test]
fn fresh_unit_with_an_older_tester_mark_still_gets_a_tester_job() {
    let repo = fixture("single");
    stamp(repo.path(), &["api", "store"]);
    let card = fs::read_to_string(repo.path().join(".stewards/units/store.toml")).unwrap();
    let stamped = card
        .lines()
        .find_map(|l| l.strip_prefix("tree = "))
        .unwrap()
        .trim_matches('"')
        .to_string();
    set_tester_mark(repo.path(), "store", &stamped);
    edit(
        &repo.path().join("src/store/db.py"),
        "_items = []",
        "_items = [0]",
    );
    stamp(repo.path(), &["store"]);
    let (code, v) = run(repo.path(), &["next"]);
    assert_eq!(code, 1);
    assert_eq!(jobs(&v), pairs(&[("store", "tester")]));
}

#[test]
fn two_stamped_changed_units_set_architect() {
    let repo = fixture("single");
    stamp(repo.path(), &["api", "store"]);
    fs::write(repo.path().join("src/store/README.md"), "changed\n").unwrap();
    edit(
        &repo.path().join("src/api/items.py"),
        "router = []",
        "router = [1]",
    );
    let (_, v) = run(repo.path(), &["next"]);
    assert_eq!(v["architect"], true);
    let job = &v["jobs"][0];
    assert_eq!(
        (&job["unit"], &job["role"]),
        (&"".into(), &"architect".into())
    );
    assert_eq!(job["command"], "stewards brief --role architect");
}

#[test]
fn never_stamped_unit_does_not_count_toward_architect() {
    let repo = fixture("single");
    stamp(repo.path(), &["api"]);
    edit(
        &repo.path().join("src/api/items.py"),
        "router = []",
        "router = [1]",
    );
    let (_, v) = run(repo.path(), &["next"]);
    assert_eq!(v["architect"], false);
    assert!(!jobs(&v).iter().any(|(_, r)| r == "architect"));
}

#[test]
fn unjudged_bugs_give_one_validator_job_for_the_first() {
    let repo = fixture("single");
    add_gaps(
        repo.path(),
        "store",
        "[[gap]]\nkind = \"bug\"\ntitle = \"save loses items\"\n\n[[gap]]\nkind = \"bug\"\ntitle = \"count is off\"\n",
    );
    stamp(repo.path(), &["api", "store"]);
    let (code, v) = run(repo.path(), &["next"]);
    assert_eq!(code, 1);
    assert_eq!(jobs(&v), pairs(&[("store", "validator")]));
    assert_eq!(
        strings(&v["jobs"][0]["reasons"]),
        ["bug not judged: save loses items"]
    );
}

#[test]
fn bug_with_a_severity_gives_no_validator_job() {
    let repo = fixture("single");
    add_gaps(
        repo.path(),
        "store",
        "[[gap]]\nkind = \"bug\"\ntitle = \"save loses items\"\nseverity = \"high\"\n",
    );
    stamp(repo.path(), &["api", "store"]);
    let (code, v) = run(repo.path(), &["next"]);
    assert_eq!(code, 0, "{v}");
}

#[test]
fn added_simplified_comment_appears_in_debt_added() {
    let repo = fixture("single");
    stamp(repo.path(), &["api", "store"]);
    edit(
        &repo.path().join("src/store/db.py"),
        "_items = []",
        "_items = []\n# simplified: no locking",
    );
    let (_, v) = run(repo.path(), &["next"]);
    assert_eq!(v["jobs"][0]["role"], "maintainer");
    let debt = &v["jobs"][0]["debt_added"];
    assert_eq!(debt.as_array().unwrap().len(), 1, "{debt}");
    assert_eq!(debt[0]["path"], "src/store/db.py");
    assert_eq!(debt[0]["line"], 2);
    assert_eq!(debt[0]["text"], "# simplified: no locking");
}

#[test]
fn task_flag_overrides_the_task() {
    let repo = fixture("single");
    let (_, v) = run(repo.path(), &["next", "--task", "add a count endpoint"]);
    assert_eq!(v["jobs"][0]["task"], "add a count endpoint");
}

#[test]
fn pr_flag_is_not_implemented() {
    let repo = fixture("single");
    let (code, v) = run(repo.path(), &["next", "--pr", "1"]);
    assert_eq!(code, 2);
    assert_eq!(v["error"]["code"], "not_implemented");
}

#[test]
fn pruned_stamped_tree_falls_back_to_all_owned_files() {
    let repo = fixture("single");
    stamp(repo.path(), &["api", "store"]);
    fs::write(repo.path().join("src/store/README.md"), "changed\n").unwrap();
    set_tree_line(repo.path(), "store", |_| {
        format!("tree = \"{}\"", "ab".repeat(20))
    });
    let (_, v) = run(repo.path(), &["next"]);
    assert_eq!(jobs(&v)[0], ("store".to_string(), "maintainer".to_string()));
    assert_eq!(
        strings(&v["jobs"][0]["changed"]),
        [
            "src/store/README.md",
            "src/store/db.py",
            "tests/store/test_db.py"
        ]
    );
}

#[test]
fn workspace_unit_names_carry_the_repo_prefix() {
    let tmp = TempDir::new().unwrap();
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/workspace");
    fs::copy(src.join("stewards.toml"), tmp.path().join("stewards.toml")).unwrap();
    for repo in ["backend", "frontend"] {
        let dest = tmp.path().join(repo);
        let status = std::process::Command::new("cp")
            .arg("-R")
            .arg(src.join(repo))
            .arg(&dest)
            .status()
            .unwrap();
        assert!(status.success());
        git(&dest, &["init", "-q"]);
    }
    fs::write(
        tmp.path().join("backend/.stewards/units.toml"),
        "[[unit]]\nname = \"core\"\npaths = [\"src/**\"]\n",
    )
    .unwrap();
    let (code, v) = run(tmp.path(), &["next"]);
    assert_eq!(code, 1);
    assert_eq!(jobs(&v), pairs(&[("api-server/core", "maintainer")]));
    assert_eq!(strings(&v["jobs"][0]["changed"]), ["src/main.py"]);
}

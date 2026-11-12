#[allow(dead_code)] // each test crate uses a subset of the helpers
mod common;

use std::fs;
use std::path::Path;

use common::{edit, fixture, git, pairs, run};
use serde_json::Value;
use stewards::check::{check, fingerprint};
use stewards::model::{UnitDef, load_repo};
use stewards::route::Router;
use stewards::workspace::discover;

fn stamp_all(dir: &Path) {
    for unit in ["api", "store"] {
        assert_eq!(run(dir, &["stamp", unit]).0, 0);
    }
}

/// `(kind, unit)` of each finding in `list`.
fn kinds(v: &Value, list: &str) -> Vec<(String, String)> {
    v[list]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["kind"].as_str().unwrap().to_string(),
                f["unit"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect()
}

#[test]
fn stamped_fixture_passes_with_its_warnings() {
    let repo = fixture("single");
    stamp_all(repo.path());
    let (code, v) = run(repo.path(), &["check"]);
    assert_eq!(code, 0);
    assert!(kinds(&v, "failures").is_empty());
    assert_eq!(
        kinds(&v, "warnings"),
        pairs(&[("uncovered_interface", "store"), ("unowned", "")])
    );
}

#[test]
fn unstaged_edit_to_an_owned_file_makes_the_unit_stale() {
    let repo = fixture("single");
    stamp_all(repo.path());
    edit(
        &repo.path().join("src/api/items.py"),
        "router = []",
        "router = [1]",
    );
    let (code, v) = run(repo.path(), &["check"]);
    assert_eq!(code, 1);
    assert_eq!(kinds(&v, "failures"), pairs(&[("stale", "api")]));
}

#[test]
fn new_untracked_owned_file_makes_the_unit_stale() {
    let repo = fixture("single");
    stamp_all(repo.path());
    fs::write(repo.path().join("src/store/cache.py"), "x = 1\n").unwrap();
    let (_, v) = run(repo.path(), &["check"]);
    assert_eq!(kinds(&v, "failures"), pairs(&[("stale", "store")]));
}

#[test]
fn ignored_file_does_not_make_the_unit_stale() {
    let repo = fixture("single");
    stamp_all(repo.path());
    fs::write(repo.path().join(".gitignore"), "*.log\n").unwrap();
    fs::write(repo.path().join("src/api/debug.log"), "noise\n").unwrap();
    let (code, v) = run(repo.path(), &["check"]);
    assert_eq!(code, 0, "{v}");
}

#[test]
fn never_stamped_unit_is_unreviewed() {
    let repo = fixture("single");
    run(repo.path(), &["stamp", "api"]);
    let (code, v) = run(repo.path(), &["check"]);
    assert_eq!(code, 1);
    assert_eq!(kinds(&v, "failures"), pairs(&[("unreviewed", "store")]));
}

#[test]
fn committing_after_a_stamp_keeps_the_unit_fresh() {
    let repo = fixture("single");
    edit(
        &repo.path().join("src/api/items.py"),
        "router = []",
        "router = [1]",
    );
    stamp_all(repo.path());
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-q", "-m", "change and stamp"]);
    assert_eq!(run(repo.path(), &["check"]).0, 0);
}

#[test]
fn anchor_that_does_not_resolve_is_dead() {
    let repo = fixture("single");
    stamp_all(repo.path());
    edit(
        &repo.path().join(".stewards/units/api.toml"),
        "name = \"router\"",
        "name = \"gone\"",
    );
    let (code, v) = run(repo.path(), &["check"]);
    assert_eq!(code, 1);
    assert_eq!(kinds(&v, "failures"), pairs(&[("dead_anchor", "api")]));
    assert_eq!(v["failures"][0]["anchor"], "src/api/items.py::gone");
}

#[test]
fn scenario_with_an_unknown_interface_id_fails() {
    let repo = fixture("single");
    stamp_all(repo.path());
    edit(
        &repo.path().join(".stewards/units/api.test.toml"),
        "interface = \"create-item\"",
        "interface = \"delete-item\"",
    );
    let (code, v) = run(repo.path(), &["check"]);
    assert_eq!(code, 1);
    assert_eq!(
        kinds(&v, "failures"),
        pairs(&[("unknown_interface", "api")])
    );
}

#[test]
fn interface_no_scenario_references_warns_uncovered() {
    let repo = fixture("single");
    let (_, v) = run(repo.path(), &["check"]);
    let w = &v["warnings"][0];
    assert_eq!(w["kind"], "uncovered_interface");
    assert_eq!(w["unit"], "store");
    assert!(w["message"].as_str().unwrap().contains("count-items"));
}

#[test]
fn tracked_file_no_unit_owns_warns_unowned_but_tool_owned_does_not() {
    let repo = fixture("single");
    let (_, v) = run(repo.path(), &["check"]);
    let unowned: Vec<_> = v["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|w| w["kind"] == "unowned")
        .map(|w| w["path"].as_str().unwrap())
        .collect();
    assert_eq!(unowned, ["Makefile"]);
}

#[test]
fn files_two_units_match_equally_warn_overlap() {
    let repo = fixture("single");
    let units = repo.path().join(".stewards/units.toml");
    let text = fs::read_to_string(&units).unwrap();
    fs::write(
        &units,
        format!("{text}\n[[unit]]\nname = \"dup\"\npaths = [\"src/api/**\"]\n"),
    )
    .unwrap();
    let (_, v) = run(repo.path(), &["check"]);
    let overlap: Vec<_> = v["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|w| w["kind"] == "overlap")
        .map(|w| w["path"].as_str().unwrap())
        .collect();
    assert_eq!(overlap, ["src/api/items.py"]);
}

#[test]
fn stamping_keeps_role_marks_and_the_rest_of_the_card() {
    let repo = fixture("single");
    let card = repo.path().join(".stewards/units/api.toml");
    edit(
        &card,
        "unit = \"api\"\n",
        "unit = \"api\"\n[reviewed]\ncommit = \"old\"\nfingerprint = \"sha256:old\"\ntree = \"old\"\ntester = \"t1\"\ndocumenter = \"d1\"\n",
    );
    let (code, v) = run(repo.path(), &["stamp", "api"]);
    assert_eq!(code, 0);
    assert_eq!(v["reviewed"]["tester"], "t1");
    let loaded = load_repo(repo.path()).unwrap();
    let card = &loaded.cards["api"];
    let r = card.reviewed.as_ref().unwrap();
    assert_eq!(r.commit, git(repo.path(), &["rev-parse", "HEAD"]).trim());
    assert_ne!(r.fingerprint, "sha256:old");
    assert_eq!(r.tree, v["reviewed"]["tree"].as_str().unwrap());
    assert_eq!(
        (r.tester.as_deref(), r.documenter.as_deref()),
        (Some("t1"), Some("d1"))
    );
    assert_eq!(card.interface[0].id, "create-item");
    assert_eq!(card.anchors[0].name, "router");
    assert_eq!(card.gaps[0].title, "no pagination on list");
    assert_eq!(
        card.doc.as_ref().unwrap().overview,
        "The HTTP surface for items."
    );
}

#[test]
fn stamping_a_unit_without_a_card_creates_one() {
    let repo = fixture("single");
    fs::remove_file(repo.path().join(".stewards/units/store.toml")).unwrap();
    assert_eq!(run(repo.path(), &["stamp", "store"]).0, 0);
    let loaded = load_repo(repo.path()).unwrap();
    assert!(loaded.cards["store"].reviewed.is_some());
}

#[test]
fn a_gap_appended_by_hand_to_a_stamped_card_still_loads() {
    let repo = fixture("single");
    assert_eq!(run(repo.path(), &["stamp", "store"]).0, 0);
    let card = repo.path().join(".stewards/units/store.toml");
    let text = fs::read_to_string(&card).unwrap();
    fs::write(&card, text + "\n[[gap]]\nkind = \"gap\"\ntitle = \"t\"\n").unwrap();
    let loaded = load_repo(repo.path()).unwrap();
    assert_eq!(loaded.cards["store"].gaps[0].title, "t");
    assert!(loaded.cards["store"].reviewed.is_some());
}

#[test]
fn stamping_with_no_commits_records_an_empty_commit() {
    let repo = fixture("single");
    git(repo.path(), &["update-ref", "-d", "HEAD"]);
    let (code, v) = run(repo.path(), &["stamp", "api"]);
    assert_eq!(code, 0);
    assert_eq!(v["reviewed"]["commit"], "");
}

#[test]
fn stamping_an_unknown_unit_is_unknown_unit() {
    let repo = fixture("single");
    let (code, v) = run(repo.path(), &["stamp", "nope"]);
    assert_eq!(code, 2);
    assert_eq!(v["error"]["code"], "unknown_unit");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("api, store")
    );
}

#[test]
fn fingerprint_covers_owned_files_not_merely_matched_ones() {
    let unit = |name: &str| UnitDef {
        name: name.into(),
        purpose: String::new(),
        paths: vec!["src/*.py".into()],
    };
    let router = Router::new(&[unit("first"), unit("second")], "docs/units").unwrap();
    let tree = [("src/a.py".to_string(), "abc".to_string())];
    assert_eq!(
        fingerprint(&tree, "second", &router),
        fingerprint(&[], "second", &router)
    );
    assert_ne!(
        fingerprint(&tree, "first", &router),
        fingerprint(&[], "first", &router)
    );
}

#[test]
fn library_check_with_a_unit_filter_keeps_only_that_units_findings() {
    let repo = fixture("single");
    let ws = discover(repo.path(), None).unwrap();
    let report = check(&ws, Some("store")).unwrap();
    let found: Vec<_> = report
        .failures
        .iter()
        .chain(&report.warnings)
        .map(|f| (f.kind, f.unit.as_deref()))
        .collect();
    assert_eq!(
        found,
        [
            ("unreviewed", Some("store")),
            ("uncovered_interface", Some("store"))
        ]
    );
}

#[test]
fn stamping_keeps_bug_gaps_multiline_doc_prose_and_role_marks_exactly() {
    let repo = fixture("single");
    let card = repo.path().join(".stewards/units/api.toml");
    let text = fs::read_to_string(&card).unwrap();
    let doc_at = text.find("[doc]").unwrap();
    let rest = r#"[reviewed]
commit = "old"
fingerprint = "sha256:old"
tree = "old"
tester = "tester-tree"
documenter = "documenter-tree"
[[gap]]
kind = "bug"
title = "list crashes on empty store"
anchor = "src/api/items.py::list_items"
note = "seen in prod"
repro = """
GET /items
with an empty store"""
severity = "high"
issue = "https://example.invalid/issues/7"
[doc]
overview = """First line.
Second line with 'quotes' and \"double\" ones."""
how_it_works = """
Leading newline is trimmed by TOML.

Blank line above, trailing newline below.
"""
how_to_change = "tab\there\nback\\slash \"\"\" triple\n"
"#;
    fs::write(&card, format!("{}{rest}", &text[..doc_at])).unwrap();
    let before = load_repo(repo.path()).unwrap().cards["api"].clone();
    let bug = &before.gaps[1];
    assert!(bug.repro.is_some() && bug.severity.is_some() && bug.issue.is_some());
    let doc = before.doc.as_ref().unwrap();
    assert!(doc.overview.contains('\n') && doc.how_it_works.contains("\n\n"));
    assert!(doc.how_to_change.contains("\\slash \"\"\""));

    assert_eq!(run(repo.path(), &["stamp", "api"]).0, 0);
    let after = load_repo(repo.path()).unwrap().cards["api"].clone();
    assert_eq!(
        serde_json::to_value(&after.gaps).unwrap(),
        serde_json::to_value(&before.gaps).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&after.doc).unwrap(),
        serde_json::to_value(&before.doc).unwrap()
    );
    let (b, a) = (before.reviewed.unwrap(), after.reviewed.unwrap());
    assert_eq!((a.tester, a.documenter), (b.tester, b.documenter));
}

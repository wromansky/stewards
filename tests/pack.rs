#[allow(dead_code)] // each test crate uses a subset of the helpers
mod common;

use std::fs;
use std::path::Path;

use common::{append, edit, fixture, git, run, stewards};
use serde_json::Value;
use stewards::git::snapshot;
use stewards::model::{Doc, Role};
use stewards::pack::pack;
use stewards::workspace::{Workspace, discover};

/// A snapshot of the lone repo's working tree, taken now.
fn snap(ws: &Workspace) -> String {
    snapshot(&ws.repos[0].root).unwrap()
}

fn pack_json(dir: &Path, args: &[&str]) -> Value {
    let (code, v) = run(dir, args);
    assert_eq!(code, 0, "{v}");
    v
}

/// Asserts that exactly the `present` sections of the pack `v` are non-null.
fn assert_sections(v: &Value, present: &[&str]) {
    const ALL: [&str; 19] = [
        "purpose",
        "interface",
        "anchors",
        "scenarios",
        "test_command",
        "primary_paths",
        "test_commands",
        "gaps",
        "flows",
        "decisions",
        "units",
        "doc",
        "changed_files",
        "hunks",
        "other_changed_files",
        "memory",
        "bug",
        "dev_task",
        "repro_result",
    ];
    for s in ALL {
        assert_eq!(
            !v[s].is_null(),
            present.contains(&s),
            "section `{s}` in {v}"
        );
    }
}

#[test]
fn maintainer_pack_has_purpose_interface_anchors_gaps_flows() {
    let repo = fixture("single");
    let v = pack_json(repo.path(), &["pack", "api", "--role", "maintainer"]);
    assert_sections(&v, &["purpose", "interface", "anchors", "gaps", "flows"]);
    assert_eq!(v["anchors"][0]["name"], "router");
    assert_eq!(v["anchors"][0]["line"], 3);
    assert!(
        v["anchors"][0]["source"]
            .as_str()
            .unwrap()
            .contains("router = []")
    );
    assert_eq!(v["flows"][0]["name"], "create-item");
}

#[test]
fn tester_pack_has_interface_source_scenarios_and_test_command() {
    let repo = fixture("single");
    let v = pack_json(repo.path(), &["pack", "api", "--role", "tester"]);
    assert_sections(
        &v,
        &[
            "interface",
            "anchors",
            "scenarios",
            "test_command",
            "primary_paths",
        ],
    );
    assert_eq!(v["anchors"][0]["name"], "create_item");
    assert!(
        v["anchors"][0]["source"]
            .as_str()
            .unwrap()
            .contains("def create_item")
    );
    assert_eq!(v["scenarios"][0]["id"], "create-ok");
    assert_eq!(v["test_command"], "pytest tests/api");
}

#[test]
fn documenter_pack_has_purpose_interface_doc_flows() {
    let repo = fixture("single");
    let v = pack_json(repo.path(), &["pack", "api", "--role", "documenter"]);
    assert_sections(&v, &["purpose", "interface", "doc", "flows"]);
    assert_eq!(v["doc"]["overview"], "The HTTP surface for items.");
}

#[test]
fn validator_pack_has_purpose_interface_paths_and_test_commands() {
    let repo = fixture("single");
    let v = pack_json(repo.path(), &["pack", "api", "--role", "validator"]);
    assert_sections(
        &v,
        &[
            "purpose",
            "interface",
            "primary_paths",
            "test_command",
            "test_commands",
        ],
    );
    assert_eq!(v["test_commands"], serde_json::json!(["pytest"]));
}

#[test]
fn validator_pack_carries_the_first_unjudged_bug_and_renders_it() {
    let repo = fixture("single");
    let dir = repo.path();
    append(
        &dir.join(".stewards/units/api.toml"),
        "[[gap]]\nkind = \"bug\"\ntitle = \"judged\"\nseverity = \"low\"\n\
         [[gap]]\nkind = \"bug\"\ntitle = \"create drops tags\"\nrepro = \"pytest -k tags\"\n\
         note = \"tags ```vanish```\"\nanchor = \"src/api/items.py::create_item\"\n\
         [[gap]]\nkind = \"bug\"\ntitle = \"second\"\n",
    );
    let v = pack_json(dir, &["pack", "api", "--role", "validator"]);
    assert_eq!(v["bug"]["title"], "create drops tags");
    assert_eq!(v["bug"]["repro"], "pytest -k tags");
    assert_eq!(v["bug"]["note"], "tags ```vanish```");
    assert_eq!(v["bug"]["anchor"], "src/api/items.py::create_item");
    let out = stewards(dir, &["pack", "api", "--role", "validator", "--human"]);
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(
        text.contains("\n## Bug\n\n### Title\n\n```\ncreate drops tags\n```\n"),
        "{text}"
    );
    assert!(text.contains("````\ntags ```vanish```\n````\n"), "{text}");
}

#[test]
fn validator_pack_bug_is_null_when_every_bug_has_a_severity() {
    let repo = fixture("single");
    let dir = repo.path();
    append(
        &dir.join(".stewards/units/api.toml"),
        "[[gap]]\nkind = \"bug\"\ntitle = \"judged\"\nseverity = \"low\"\n",
    );
    let v = pack_json(dir, &["pack", "api", "--role", "validator"]);
    assert!(v["bug"].is_null(), "{v}");
}

#[test]
fn diff_runs_to_the_given_snapshot_not_the_working_tree() {
    let repo = fixture("single");
    let dir = repo.path();
    assert_eq!(run(dir, &["stamp", "api"]).0, 0);
    let ws = discover(dir, None).unwrap();
    edit(&dir.join("src/api/items.py"), "router = []", "router = [1]");
    let older = snap(&ws);
    edit(
        &dir.join("src/api/items.py"),
        "router = [1]",
        "router = [2]",
    );
    let p = pack(
        &ws.repos[0],
        Some("api"),
        Role::Maintainer,
        "t",
        true,
        &older,
    )
    .unwrap();
    let diff = p.hunks.unwrap()[0].diff.clone().unwrap();
    assert!(diff.contains("+router = [1]"), "{diff}");
    assert!(!diff.contains("router = [2]"), "{diff}");
}

#[test]
fn architect_pack_has_every_unit_flows_decisions_and_no_source() {
    let repo = fixture("single");
    let v = pack_json(repo.path(), &["pack", "--role", "architect"]);
    assert_sections(&v, &["units", "flows", "decisions"]);
    assert_eq!(
        v["units"],
        serde_json::json!([
            {"name": "api", "purpose": "HTTP surface", "interface_ids": ["create-item"]},
            {"name": "store", "purpose": "item persistence", "interface_ids": ["save-item", "count-items"]},
        ])
    );
    assert_eq!(v["decisions"][0]["title"], "store has no HTTP knowledge");
    assert!(!v.to_string().contains("save_item("));
}

#[test]
fn developer_pack_is_not_implemented() {
    let repo = fixture("single");
    let (code, v) = run(repo.path(), &["pack", "api", "--role", "developer"]);
    assert_eq!(code, 2);
    assert_eq!(v["error"]["code"], "not_implemented");
}

#[test]
fn architect_with_a_unit_is_a_usage_error() {
    let repo = fixture("single");
    let (code, v) = run(repo.path(), &["pack", "api", "--role", "architect"]);
    assert_eq!(code, 2);
    assert_eq!(v["error"]["code"], "usage");
}

#[test]
fn maintainer_without_a_unit_is_a_usage_error() {
    let repo = fixture("single");
    let (code, v) = run(repo.path(), &["pack", "--role", "maintainer"]);
    assert_eq!(code, 2);
    assert_eq!(v["error"]["code"], "usage");
}

#[test]
fn unknown_unit_is_unknown_unit() {
    let repo = fixture("single");
    let (code, v) = run(repo.path(), &["pack", "nope", "--role", "maintainer"]);
    assert_eq!(code, 2);
    assert_eq!(v["error"]["code"], "unknown_unit");
}

#[test]
fn dead_anchor_is_listed_with_null_line_and_source() {
    let repo = fixture("single");
    edit(
        &repo.path().join(".stewards/units/api.toml"),
        "name = \"router\"",
        "name = \"gone\"",
    );
    let v = pack_json(repo.path(), &["pack", "api", "--role", "maintainer"]);
    let a = &v["anchors"][0];
    assert_eq!(a["name"], "gone");
    assert!(a["line"].is_null() && a["source"].is_null(), "{a}");
}

#[test]
fn excluded_file_is_named_but_not_inlined() {
    let repo = fixture("single");
    edit(
        &repo.path().join(".stewards/repo.toml"),
        "exclude = [",
        "exclude = [\"items.py\", ",
    );
    let out = stewards(repo.path(), &["pack", "api", "--role", "maintainer"]);
    let text = String::from_utf8(out.stdout).unwrap();
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["anchors"][0]["path"], "src/api/items.py");
    assert!(v["anchors"][0]["source"].is_null());
    assert!(!text.contains("router = []"), "{text}");
    assert!(
        v["dropped"][0]
            .as_str()
            .unwrap()
            .contains("src/api/items.py::router")
    );
}

#[test]
fn diff_after_a_stamp_shows_hunks_restricts_anchors_and_lists_other_units_files() {
    let repo = fixture("single");
    let dir = repo.path();
    append(
        &dir.join(".stewards/units/api.toml"),
        "[[anchor]]\npath = \"tests/api/test_items.py\"\nname = \"test_create_ok\"\n",
    );
    for unit in ["api", "store"] {
        assert_eq!(run(dir, &["stamp", unit]).0, 0);
    }
    edit(&dir.join("src/api/items.py"), "router = []", "router = [1]");
    edit(&dir.join("src/store/db.py"), "_items = []", "_items = [0]");
    let v = pack_json(dir, &["pack", "api", "--role", "maintainer", "--diff"]);
    assert_eq!(v["changed_files"], serde_json::json!(["src/api/items.py"]));
    let paths: Vec<_> = v["anchors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, ["src/api/items.py"]);
    assert_eq!(v["hunks"][0]["path"], "src/api/items.py");
    assert!(
        v["hunks"][0]["diff"]
            .as_str()
            .unwrap()
            .contains("+router = [1]")
    );
    assert_eq!(
        v["other_changed_files"],
        serde_json::json!(["src/store/db.py"])
    );
}

#[test]
fn diff_with_no_stamp_equals_the_plain_pack() {
    let repo = fixture("single");
    let args = ["pack", "api", "--role", "maintainer", "--task", "t"];
    let plain = pack_json(repo.path(), &args);
    let diff = pack_json(repo.path(), &[&args[..], &["--diff"]].concat());
    assert_eq!(plain, diff);
}

#[test]
fn fit_shrinks_in_the_fixed_order() {
    let repo = fixture("single");
    let dir = repo.path();
    append(
        &dir.join(".stewards/units/api.toml"),
        "[[anchor]]\npath = \"src/api/items.py\"\nname = \"list_items\"\n",
    );
    assert_eq!(run(dir, &["stamp", "api"]).0, 0);
    let extra: String = (0..60)
        .map(|i| format!("EXTRA_{i} = \"a padded line to make the window and hunk long\"\n"))
        .collect();
    append(&dir.join("src/api/items.py"), &extra);

    let ws = discover(dir, None).unwrap();
    let mut p = pack(
        &ws.repos[0],
        Some("api"),
        Role::Maintainer,
        "t",
        true,
        &snap(&ws),
    )
    .unwrap();
    assert!(p.dropped.is_empty() && !p.over_budget);
    let steps = [
        "source windows halved",
        "source src/api/items.py::list_items",
        "source src/api/items.py::router",
        "gaps",
        "flows",
        "hunks truncated to 40 lines",
    ];
    for (i, _) in steps.iter().enumerate() {
        p.fit(p.bytes - 1);
        assert_eq!(p.dropped, steps[..=i], "after step {i}");
        assert!(!p.over_budget);
        assert_eq!(p.bytes, p.render().len());
    }
}

#[test]
fn never_dropped_sections_survive_a_tiny_budget() {
    let repo = fixture("single");
    let dir = repo.path();
    for unit in ["api", "store"] {
        assert_eq!(run(dir, &["stamp", unit]).0, 0);
    }
    edit(&dir.join("src/store/db.py"), "_items = []", "_items = [0]");
    let ws = discover(dir, None).unwrap();

    let mut p = pack(
        &ws.repos[0],
        Some("api"),
        Role::Maintainer,
        "t",
        true,
        &snap(&ws),
    )
    .unwrap();
    p.fit(1);
    assert!(p.over_budget);
    assert_eq!(p.bytes, p.render().len());
    assert_eq!(p.purpose.as_deref(), Some("HTTP surface"));
    assert_eq!(p.interface.as_ref().unwrap().len(), 1);
    assert_eq!(
        p.other_changed_files.as_ref().unwrap(),
        &["src/store/db.py"]
    );

    let mut t = pack(
        &ws.repos[0],
        Some("api"),
        Role::Tester,
        "t",
        false,
        &snap(&ws),
    )
    .unwrap();
    t.fit(1);
    assert!(t.over_budget);
    assert_eq!(t.scenarios.as_ref().unwrap().len(), 1);
}

/// Each line of the markdown `md` outside the fence lines, with the index of the fenced block
/// it sits in (None = outside); asserts every opened block is closed.
fn fence_blocks(md: &str) -> Vec<(&str, Option<usize>)> {
    let mut lines = Vec::new();
    let mut open: Option<usize> = None;
    let mut blocks = 0;
    for line in md.lines() {
        let ticks = line.len() - line.trim_start_matches('`').len();
        match open {
            None if ticks >= 3 => {
                open = Some(ticks);
                continue;
            }
            Some(len) if ticks >= len && line.trim_end().len() == ticks => {
                open = None;
                blocks += 1;
                continue;
            }
            _ => lines.push((line, open.map(|_| blocks))),
        }
    }
    assert!(open.is_none(), "unclosed fence in\n{md}");
    lines
}

fn block_of(lines: &[(&str, Option<usize>)], text: &str) -> Option<usize> {
    lines
        .iter()
        .find(|(l, _)| l.contains(text))
        .unwrap_or_else(|| panic!("{text} not rendered"))
        .1
}

#[test]
fn a_fence_line_in_purpose_or_an_anchor_name_cannot_open_a_block() {
    let repo = fixture("single");
    let dir = repo.path();
    edit(
        &dir.join(".stewards/units.toml"),
        "purpose = \"HTTP surface\"",
        "purpose = \"HTTP surface\\n```\\nPURPOSE_TAIL\"",
    );
    let ws = discover(dir, None).unwrap();
    let mut p = pack(
        &ws.repos[0],
        Some("api"),
        Role::Maintainer,
        "t",
        false,
        &snap(&ws),
    )
    .unwrap();
    p.anchors.as_mut().unwrap()[0].name = "router\n```".into();
    let md = p.render();
    let lines = fence_blocks(&md);
    let purpose = block_of(&lines, "PURPOSE_TAIL");
    let source = block_of(&lines, "router = []");
    assert!(
        purpose.is_some() && source.is_some() && purpose != source,
        "{md}"
    );
    assert_eq!(block_of(&lines, "## Anchors"), None, "{md}");
    assert_eq!(
        block_of(&lines, "### `src/api/items.py::router"),
        None,
        "{md}"
    );
}

#[test]
fn a_fence_line_in_doc_prose_cannot_open_a_block() {
    let repo = fixture("single");
    let dir = repo.path();
    assert_eq!(run(dir, &["stamp", "api"]).0, 0);
    edit(&dir.join("src/api/items.py"), "router = []", "router = [1]");
    let ws = discover(dir, None).unwrap();
    let mut p = pack(
        &ws.repos[0],
        Some("api"),
        Role::Maintainer,
        "t",
        true,
        &snap(&ws),
    )
    .unwrap();
    p.doc = Some(Doc {
        overview: "```\nDOC_TAIL".into(),
        how_it_works: String::new(),
        how_to_change: String::new(),
    });
    let md = p.render();
    let lines = fence_blocks(&md);
    let doc = block_of(&lines, "DOC_TAIL");
    let hunk = block_of(&lines, "+router = [1]");
    assert!(doc.is_some() && hunk.is_some() && doc != hunk, "{md}");
    assert_eq!(block_of(&lines, "## Hunks"), None, "{md}");
}

#[test]
fn other_changed_files_measures_each_other_unit_from_its_own_stamp() {
    let repo = fixture("single");
    let dir = repo.path();
    let args = ["pack", "api", "--role", "maintainer", "--diff"];
    let other = |dir| pack_json(dir, &args)["other_changed_files"].clone();
    assert_eq!(run(dir, &["stamp", "api"]).0, 0);
    // never stamped: every file the unit owns
    assert_eq!(
        other(dir),
        serde_json::json!([
            "src/store/README.md",
            "src/store/db.py",
            "tests/store/test_db.py"
        ])
    );
    edit(&dir.join("src/store/db.py"), "_items = []", "_items = [0]");
    assert_eq!(run(dir, &["stamp", "store"]).0, 0);
    // stamped after the change: fresh, so nothing
    assert_eq!(other(dir), serde_json::json!([]));
    edit(&dir.join("src/store/db.py"), "_items = [0]", "_items = [1]");
    assert_eq!(other(dir), serde_json::json!(["src/store/db.py"]));
}

#[test]
fn changed_excluded_file_is_listed_but_its_hunk_is_not_inlined() {
    let repo = fixture("single");
    let dir = repo.path();
    fs::write(dir.join("src/api/.env"), "TOKEN=old\n").unwrap();
    assert_eq!(run(dir, &["stamp", "api"]).0, 0);
    fs::write(dir.join("src/api/.env"), "TOKEN=leaked\n").unwrap();
    let out = stewards(dir, &["pack", "api", "--role", "maintainer", "--diff"]);
    let text = String::from_utf8(out.stdout).unwrap();
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["changed_files"], serde_json::json!(["src/api/.env"]));
    assert_eq!(v["hunks"][0]["path"], "src/api/.env");
    assert!(v["hunks"][0]["diff"].is_null(), "{v}");
    assert!(!text.contains("TOKEN"), "{text}");
    let human = stewards(
        dir,
        &["pack", "api", "--role", "maintainer", "--diff", "--human"],
    );
    assert!(!String::from_utf8(human.stdout).unwrap().contains("TOKEN"));
}

#[test]
fn excluded_file_at_a_tester_interface_anchor_is_not_inlined() {
    let repo = fixture("single");
    edit(
        &repo.path().join(".stewards/repo.toml"),
        "exclude = [",
        "exclude = [\"items.py\", ",
    );
    let out = stewards(repo.path(), &["pack", "api", "--role", "tester"]);
    let text = String::from_utf8(out.stdout).unwrap();
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["anchors"][0]["path"], "src/api/items.py");
    assert!(v["anchors"][0]["source"].is_null(), "{v}");
    assert!(!text.contains("save_item(body)"), "{text}");
}

#[test]
fn excluded_content_stays_absent_while_fit_shrinks() {
    let repo = fixture("single");
    let dir = repo.path();
    edit(
        &dir.join(".stewards/repo.toml"),
        "exclude = [",
        "exclude = [\"items.py\", ",
    );
    append(
        &dir.join(".stewards/units/api.toml"),
        "[[anchor]]\npath = \"tests/api/test_items.py\"\nname = \"test_create_ok\"\n",
    );
    assert_eq!(run(dir, &["stamp", "api"]).0, 0);
    edit(&dir.join("src/api/items.py"), "router = []", "router = [1]");
    append(&dir.join("tests/api/test_items.py"), "\n# more\n");
    let ws = discover(dir, None).unwrap();
    let mut p = pack(
        &ws.repos[0],
        Some("api"),
        Role::Maintainer,
        "t",
        true,
        &snap(&ws),
    )
    .unwrap();
    let assert_absent = |p: &stewards::pack::Pack| {
        let json = serde_json::to_string(p).unwrap();
        assert!(!p.render().contains("router = [") && !json.contains("router = ["));
        assert!(!json.contains("save_item(body)"));
    };
    assert_absent(&p);
    for _ in 0..8 {
        p.fit(p.bytes - 1);
        assert_absent(&p);
    }
    assert!(p.dropped.iter().any(|d| d == "source windows halved"));
}

#[test]
fn halving_shortens_every_source_window() {
    let repo = fixture("single");
    let dir = repo.path();
    append(
        &dir.join(".stewards/units/api.toml"),
        "[[anchor]]\npath = \"src/api/items.py\"\nname = \"list_items\"\n",
    );
    let extra: String = (0..60).map(|i| format!("EXTRA_{i} = {i}\n")).collect();
    append(&dir.join("src/api/items.py"), &extra);
    let ws = discover(dir, None).unwrap();
    let mut p = pack(
        &ws.repos[0],
        Some("api"),
        Role::Maintainer,
        "t",
        false,
        &snap(&ws),
    )
    .unwrap();
    let lines = |p: &stewards::pack::Pack| -> Vec<usize> {
        p.anchors
            .iter()
            .flatten()
            .map(|a| a.source.as_ref().unwrap().lines().count())
            .collect()
    };
    let before = lines(&p);
    p.fit(p.bytes - 1);
    assert_eq!(p.dropped, ["source windows halved"]);
    let after = lines(&p);
    assert_eq!(before.len(), 2);
    for (b, a) in before.iter().zip(&after) {
        assert!(a < b, "{before:?} -> {after:?}");
    }
    for a in p.anchors.iter().flatten() {
        assert!(a.source.as_ref().unwrap().contains(&a.name), "{a:?}");
    }
}

#[test]
fn diff_with_a_pruned_stamped_tree_equals_the_plain_pack() {
    let repo = fixture("single");
    let dir = repo.path();
    assert_eq!(run(dir, &["stamp", "api"]).0, 0);
    let card = dir.join(".stewards/units/api.toml");
    let text = fs::read_to_string(&card).unwrap();
    let tree = text
        .lines()
        .find_map(|l| l.strip_prefix("tree = "))
        .unwrap()
        .to_string();
    edit(&card, &tree, &format!("\"{}\"", "deadbeef".repeat(5)));
    edit(&dir.join("src/api/items.py"), "router = []", "router = [1]");
    let args = ["pack", "api", "--role", "maintainer", "--task", "t"];
    let plain = pack_json(dir, &args);
    let diff = pack_json(dir, &[&args[..], &["--diff"]].concat());
    assert!(diff["hunks"].is_null(), "{diff}");
    assert_eq!(plain, diff);
}

#[test]
fn human_task_line_cannot_open_a_code_fence_or_add_lines() {
    let repo = fixture("single");
    let args = ["pack", "api", "--role", "maintainer", "--human", "--task"];
    let out = stewards(repo.path(), &[&args[..], &["a\n```\nb"]].concat());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("\nTask: a ``` b\n"), "{text}");
}

/// Packs `api` for `role` as JSON and as `--human`; returns the parsed JSON and both raw texts.
fn pack_both(dir: &Path, role: &str) -> (Value, String) {
    let raw = |extra: &[&str]| {
        let mut args = vec!["pack", "api", "--role", role];
        args.extend(extra);
        let out = stewards(dir, &args);
        assert!(out.status.success());
        String::from_utf8(out.stdout).unwrap()
    };
    let (json, human) = (raw(&[]), raw(&["--human"]));
    (serde_json::from_str(&json).unwrap(), json + &human)
}

/// Points `api`'s maintainer anchor and tester interface anchor at `path::name`, then packs both.
fn assert_anchor_is_not_inlined(dir: &Path, path: &str, name: &str, secret: &str) {
    let card = dir.join(".stewards/units/api.toml");
    edit(
        &card,
        "path = \"src/api/items.py\"\nname = \"router\"",
        &format!("path = \"{path}\"\nname = \"{name}\""),
    );
    edit(
        &card,
        "anchor = \"src/api/items.py::create_item\"\ncontract",
        &format!("anchor = \"{path}::{name}\"\ncontract"),
    );
    for role in ["maintainer", "tester"] {
        let (v, text) = pack_both(dir, role);
        let a = &v["anchors"][0];
        assert_eq!(a["path"], path, "{role}");
        assert!(a["line"].is_null() && a["source"].is_null(), "{role}: {a}");
        assert!(!text.contains(secret), "{role}: {text}");
    }
}

#[test]
fn anchor_into_dot_git_config_is_not_inlined() {
    let repo = fixture("single");
    git(
        repo.path(),
        &[
            "remote",
            "add",
            "origin",
            "https://tok3n-9f2c@example.invalid/r.git",
        ],
    );
    assert_anchor_is_not_inlined(repo.path(), ".git/config", "url", "tok3n-9f2c");
}

#[test]
fn anchor_into_an_ignored_file_is_not_inlined() {
    let repo = fixture("single");
    fs::write(repo.path().join(".gitignore"), "local.cfg\n").unwrap();
    fs::write(repo.path().join("local.cfg"), "api_key = hunter2-9f2c\n").unwrap();
    assert_anchor_is_not_inlined(repo.path(), "local.cfg", "api_key", "hunter2-9f2c");
}

#[test]
fn anchor_into_an_untracked_file_that_is_not_ignored_is_inlined() {
    let repo = fixture("single");
    fs::write(repo.path().join("notes.cfg"), "api_key = visible-9f2c\n").unwrap();
    let card = repo.path().join(".stewards/units/api.toml");
    edit(
        &card,
        "path = \"src/api/items.py\"\nname = \"router\"",
        "path = \"notes.cfg\"\nname = \"api_key\"",
    );
    let (v, _) = pack_both(repo.path(), "maintainer");
    assert_eq!(v["anchors"][0]["line"], 1);
    assert!(
        v["anchors"][0]["source"]
            .as_str()
            .unwrap()
            .contains("visible-9f2c")
    );
}

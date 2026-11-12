#[allow(dead_code)] // each test crate uses a subset of the helpers
mod common;

use std::path::Path;

use common::{fixture, git, stewards};
use serde_json::Value;

fn run(dir: &Path, args: &[&str]) -> Value {
    let out = stewards(dir, args);
    serde_json::from_slice(&out.stdout).unwrap()
}

fn golden(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/docs")
            .join(name),
    )
    .unwrap()
}

fn read(dir: &Path, rel: &str) -> String {
    std::fs::read_to_string(dir.join(rel)).unwrap()
}

#[test]
fn unit_page_matches_golden() {
    let tmp = fixture("single");
    run(tmp.path(), &["docs"]);
    assert_eq!(read(tmp.path(), "docs/units/api.md"), golden("api.md"));
}

#[test]
fn index_matches_golden() {
    let tmp = fixture("single");
    run(tmp.path(), &["docs"]);
    assert_eq!(
        read(tmp.path(), "docs/units/README.md"),
        golden("README.md")
    );
}

#[test]
fn dead_anchor_renders_without_a_link_and_succeeds() {
    let tmp = fixture("single");
    let card = tmp.path().join(".stewards/units/store.toml");
    let text = std::fs::read_to_string(&card).unwrap();
    std::fs::write(
        &card,
        text.replace("db.py::save_item", "gone.py::save_item"),
    )
    .unwrap();
    let out = stewards(tmp.path(), &["docs", "store"]);
    assert!(out.status.success());
    let page = read(tmp.path(), "docs/units/store.md");
    assert!(page.contains(r"| src/store/gone.py\:\:save\_item |"));
    assert!(page.contains(r"[src/store/db.py\:\:count\_items](../../src/store/db.py#L"));
}

#[test]
fn missing_prose_prints_the_not_written_line() {
    let tmp = fixture("single");
    run(tmp.path(), &["docs", "api"]);
    let page = read(tmp.path(), "docs/units/api.md");
    assert!(page.contains("## How it works\n\n_Not written yet._\n"));
    assert!(page.contains("## How to change it\n\n_Not written yet._\n"));
    assert!(!page.contains("## Overview\n\n_Not written yet._"));
}

#[test]
fn second_render_writes_nothing() {
    let tmp = fixture("single");
    let first = run(tmp.path(), &["docs"]);
    assert_eq!(first["wrote"].as_array().unwrap().len(), 3);
    let second = run(tmp.path(), &["docs"]);
    assert_eq!(second["wrote"], serde_json::json!([]));
}

#[test]
fn one_unit_writes_its_page_and_the_index_only() {
    let tmp = fixture("single");
    let report = run(tmp.path(), &["docs", "api"]);
    assert_eq!(
        report["wrote"],
        serde_json::json!(["docs/units/api.md", "docs/units/README.md"])
    );
    assert!(!tmp.path().join("docs/units/store.md").exists());
}

#[test]
fn unknown_unit_is_an_error() {
    let tmp = fixture("single");
    let out = stewards(tmp.path(), &["docs", "nope"]);
    assert_eq!(out.status.code(), Some(2));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["error"]["code"], "unknown_unit");
}

#[test]
fn nothing_outside_docs_path_changes() {
    let tmp = fixture("single");
    run(tmp.path(), &["docs"]);
    let status = git(tmp.path(), &["status", "--porcelain"]);
    assert_eq!(status.trim(), "?? docs/");
}

fn edit(dir: &Path, rel: &str, from: &str, to: &str) {
    let path = dir.join(rel);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains(from), "{rel} has no `{from}`");
    std::fs::write(path, text.replace(from, to)).unwrap();
}

fn rejects_docs_path(value: &str) {
    let tmp = fixture("single");
    edit(
        tmp.path(),
        ".stewards/repo.toml",
        r#"docs_path = "docs/units""#,
        &format!("docs_path = {value:?}"),
    );
    git(tmp.path(), &["commit", "-qam", "docs_path"]);
    let out = stewards(tmp.path(), &["docs"]);
    assert_eq!(out.status.code(), Some(2));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["error"]["code"], "invalid_toml");
    assert_eq!(git(tmp.path(), &["status", "--porcelain"]), "");
}

#[test]
fn docs_path_with_parent_dir_is_rejected() {
    rejects_docs_path("../x");
}

#[test]
fn absolute_docs_path_is_rejected() {
    rejects_docs_path("/stewards-docs-test");
}

#[test]
fn dot_docs_path_is_rejected() {
    rejects_docs_path(".");
}

#[test]
fn empty_docs_path_is_rejected() {
    rejects_docs_path("");
}

/// Unescaped `|` characters on a line; an escaped one follows a backslash.
fn separators(line: &str) -> usize {
    let mut chars = line.chars();
    let mut n = 0;
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            }
            '|' => n += 1,
            _ => {}
        }
    }
    n
}

#[test]
fn hostile_card_text_renders_as_plain_text_in_table_cells() {
    let tmp = fixture("single");
    edit(
        tmp.path(),
        ".stewards/units/store.toml",
        "contract = \"stores an item and returns its id\"",
        r#"contract = 'a|b \| `x` <b>hi</b> [l](javascript:alert(1))'"#,
    );
    edit(
        tmp.path(),
        ".stewards/units/store.toml",
        "src/store/db.py::count_items",
        "src/store/d`b.py::count_items",
    );
    run(tmp.path(), &["docs", "store"]);
    let page = read(tmp.path(), "docs/units/store.md");
    let row = page.lines().find(|l| l.starts_with("| save-item")).unwrap();
    assert!(row.contains(r"a\|b \\\| \`x\` \<b\>hi\</b\> \[l\](javascript\:alert(1))"));
    assert!(!page.contains("<b>") && !page.contains("[l](javascript"));
    let dead = page
        .lines()
        .find(|l| l.starts_with("| count-items"))
        .unwrap();
    assert!(dead.contains(r"| src/store/d\`b.py\:\:count\_items |"));
    for line in page
        .lines()
        .filter(|l| l.starts_with("| save-item") || l.starts_with("| count-items"))
    {
        assert_eq!(separators(line), 4, "{line}");
    }
}

#[test]
fn hostile_flow_name_is_escaped() {
    let tmp = fixture("single");
    edit(
        tmp.path(),
        ".stewards/repo.toml",
        "name = \"create-item\"",
        "name = \"x*y_[z]<i>\"",
    );
    run(tmp.path(), &["docs"]);
    let index = read(tmp.path(), "docs/units/README.md");
    assert!(index.contains(r"- x\*y\_\[z\]\<i\>: "));
    let page = read(tmp.path(), "docs/units/api.md");
    assert!(page.contains(r"- x\*y\_\[z\]\<i\>: "));
}

#[test]
fn hostile_html_in_prose_is_neutralised() {
    let tmp = fixture("single");
    edit(
        tmp.path(),
        ".stewards/units/api.toml",
        r#"overview = """The HTTP surface for items.""""#,
        r#"overview = """Use **this** <script>x</script> &amp; that.""""#,
    );
    run(tmp.path(), &["docs", "api"]);
    let page = read(tmp.path(), "docs/units/api.md");
    assert!(page.contains("Use **this** &lt;script>x&lt;/script> &amp;amp; that."));
}

fn anchor_link_with_docs_path(docs_path: &str) -> String {
    let tmp = fixture("single");
    edit(
        tmp.path(),
        ".stewards/repo.toml",
        r#"docs_path = "docs/units""#,
        &format!("docs_path = {docs_path:?}"),
    );
    run(tmp.path(), &["docs", "api"]);
    read(tmp.path(), &format!("{docs_path}/api.md"))
}

#[test]
fn anchor_link_at_depth_one_goes_up_one_directory() {
    let page = anchor_link_with_docs_path("docs");
    assert!(page.contains("(../src/api/items.py#L6)"));
}

#[test]
fn anchor_link_at_depth_three_goes_up_three_directories() {
    let page = anchor_link_with_docs_path("a/b/c");
    assert!(page.contains("(../../../src/api/items.py#L6)"));
}

/// Writes `contract` into the `create-item` interface and returns its row in api.md.
fn contract_row(contract: &str) -> String {
    let tmp = fixture("single");
    edit(
        tmp.path(),
        ".stewards/units/api.toml",
        "contract = \"POST /items; 201 with body; 422 on invalid\"",
        &format!("contract = '{contract}'"),
    );
    run(tmp.path(), &["docs", "api"]);
    let page = read(tmp.path(), "docs/units/api.md");
    page.lines()
        .find(|l| l.starts_with("| create-item |"))
        .unwrap()
        .to_string()
}

#[test]
fn dangerous_contract_text_is_escaped_and_keeps_the_table_shape() {
    let cases = [
        ("a|b", r"a\|b"),
        (r"a\|b", r"a\\\|b"),
        ("`x`", r"\`x\`"),
        ("<b>", r"\<b\>"),
        ("[x](javascript:alert(1))", r"\[x\](javascript\:alert(1))"),
        ("![i](http://e/x.png)", r"\!\[i\](http\://e/x.png)"),
        ("*bold*", r"\*bold\*"),
        ("_em_", r"\_em\_"),
        ("~~s~~", r"\~\~s\~\~"),
        ("&amp;", r"\&amp;"),
        ("https://evil.example/x", r"https\://evil.example/x"),
        ("www.evil.example", r"www\.evil.example"),
        ("WWW.evil.example", r"WWW\.evil.example"),
        ("a@evil.example", r"a\@evil.example"),
        ("# heading", r"\# heading"),
    ];
    for (input, escaped) in cases {
        let row = contract_row(input);
        assert!(row.contains(&format!("| {escaped} |")), "{input}: {row}");
        assert_eq!(separators(&row), 4, "{input}: {row}");
    }
}

#[test]
fn ordinary_prose_has_no_backslash() {
    let row = contract_row(
        "Tree sha of the working tree (tracked, modified, untracked-not-ignored); works with no commits.",
    );
    assert!(row.contains("| Tree sha of the working tree (tracked, modified, untracked-not-ignored); works with no commits. |"));
    let contract_cell = row.split(" | ").nth(1).unwrap();
    assert!(!contract_cell.contains('\\'), "{row}");
}

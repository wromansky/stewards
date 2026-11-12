use std::path::Path;

use stewards::output::CliError;
use stewards::user::{Entry, load_config_from, load_memory_from, memory_for};
use tempfile::TempDir;

fn write(dir: &Path, name: &str, text: &str) {
    std::fs::write(dir.join(name), text).unwrap();
}

fn entry(text: &str, languages: Option<&[&str]>, roles: Option<&[&str]>) -> Entry {
    let own = |v: &[&str]| v.iter().map(|s| s.to_string()).collect();
    Entry {
        text: text.into(),
        languages: languages.map(own),
        roles: roles.map(own),
    }
}

fn texts(entries: Vec<&Entry>) -> Vec<&str> {
    entries.into_iter().map(|e| e.text.as_str()).collect()
}

fn langs(l: &[&str]) -> Vec<String> {
    l.iter().map(|s| s.to_string()).collect()
}

#[test]
fn missing_files_load_empty() {
    let tmp = TempDir::new().unwrap();
    assert!(
        load_config_from(tmp.path())
            .unwrap()
            .model("a", "b")
            .is_none()
    );
    assert!(load_memory_from(tmp.path()).unwrap().is_empty());
}

#[test]
fn role_pin_is_read() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "config.toml",
        "[models.claude]\nmaintainer = \"opus\"\n",
    );
    let c = load_config_from(tmp.path()).unwrap();
    assert_eq!(c.model("claude", "maintainer"), Some("opus"));
}

#[test]
fn unknown_harness_or_role_is_none() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "config.toml",
        "[models.claude]\nmaintainer = \"opus\"\n",
    );
    let c = load_config_from(tmp.path()).unwrap();
    assert_eq!(c.model("codex", "maintainer"), None);
    assert_eq!(c.model("claude", "tester"), None);
}

#[test]
fn memory_entries_are_read() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "memory.toml",
        "[[entry]]\ntext = \"a\"\nroles = [\"tester\"]\n[[entry]]\ntext = \"b\"\nlanguages = [\"rust\"]\n",
    );
    let m = load_memory_from(tmp.path()).unwrap();
    assert_eq!(m.len(), 2);
    assert_eq!(m[0].roles.as_deref(), Some(&["tester".to_string()][..]));
    assert_eq!(m[1].languages.as_deref(), Some(&["rust".to_string()][..]));
}

#[test]
fn role_filter_keeps_matching_and_unfiltered_entries() {
    let es = [
        entry("t", None, Some(&["tester"])),
        entry("m", None, Some(&["maintainer"])),
        entry("all", None, None),
    ];
    assert_eq!(texts(memory_for(&es, "tester", &[], 100)), ["t", "all"]);
}

#[test]
fn language_filter_needs_a_shared_language() {
    let es = [
        entry("rs", Some(&["rust"]), None),
        entry("py", Some(&["python", "go"]), None),
    ];
    assert_eq!(
        texts(memory_for(&es, "tester", &langs(&["go", "c"]), 100)),
        ["py"]
    );
    assert!(memory_for(&es, "tester", &[], 100).is_empty());
}

#[test]
fn entry_without_filters_always_matches() {
    let es = [entry("x", None, None)];
    assert_eq!(texts(memory_for(&es, "architect", &[], 100)), ["x"]);
}

#[test]
fn budget_cuts_whole_entries_and_stops_at_the_first_overflow() {
    let es = [
        entry("aaaa", None, None),
        entry("bbbb", None, None),
        entry("c", None, None),
    ];
    assert_eq!(texts(memory_for(&es, "tester", &[], 7)), ["aaaa"]);
    assert_eq!(texts(memory_for(&es, "tester", &[], 8)), ["aaaa", "bbbb"]);
}

#[test]
fn malformed_file_is_invalid_toml_with_file_and_line() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "memory.toml",
        "[[entry]]\ntext = \"ok\"\n[[entry]]\ntext = \n",
    );
    let e = load_memory_from(tmp.path()).unwrap_err();
    let e = e.downcast_ref::<CliError>().expect("a CliError");
    assert_eq!(e.code, "invalid_toml");
    assert!(e.message.contains("memory.toml:4"), "{}", e.message);
}

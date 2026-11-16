use std::path::Path;

use stewards::anchors::{Resolution, interface_lines, resolve, source_window};
use stewards::model::Anchor;

fn anchor(path: &str, name: &str) -> Anchor {
    Anchor {
        path: path.into(),
        name: name.into(),
    }
}

fn repo_with(file: &str, content: &[u8]) -> tempfile::TempDir {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(tmp.path().join(file), content).unwrap();
    tmp
}

fn window(root: &Path, name: &str, before: usize, after: usize) -> Option<String> {
    source_window(root, &anchor("f.txt", name), before, after).map(|(_, text)| text)
}

#[test]
fn found_reports_the_one_based_line() {
    let t = repo_with("f.txt", b"one\ntwo\nfn target() {}\n");
    assert_eq!(
        resolve(t.path(), &anchor("f.txt", "target")),
        Resolution::Found { line: 3 }
    );
}

#[test]
fn name_only_inside_a_longer_identifier_is_dead_name() {
    let t = repo_with("f.txt", b"fn target_two() {}\nlet my_target = 1;\n");
    assert_eq!(
        resolve(t.path(), &anchor("f.txt", "target")),
        Resolution::DeadName
    );
}

#[test]
fn matching_is_case_sensitive() {
    let t = repo_with("f.txt", b"fn Target() {}\n");
    assert_eq!(
        resolve(t.path(), &anchor("f.txt", "target")),
        Resolution::DeadName
    );
}

#[test]
fn a_whole_word_after_a_partial_one_is_found() {
    let t = repo_with("f.txt", b"target_two\ntarget\n");
    assert_eq!(
        resolve(t.path(), &anchor("f.txt", "target")),
        Resolution::Found { line: 2 }
    );
}

#[test]
fn interface_lines_cover_a_multi_line_signature_body_and_doc_comment() {
    let text = "use m::f;\n\n/// Doc.\n#[inline]\npub fn f(\n    a: u8,\n) -> u8 {\n    a\n}\n\nfn g() {}\n";
    // line 1 is a mention; the definition with its doc and attribute is lines 3-9
    assert_eq!(interface_lines("f.rs", text, "f"), [3, 4, 5, 6, 7, 8, 9]);
}

#[test]
fn interface_lines_of_a_python_def_stop_at_the_next_def() {
    let text = "@cache\ndef f(x):\n    y = x\n\n    return y\n\n\ndef g():\n    return 0\n";
    assert_eq!(interface_lines("f.py", text, "f"), [1, 2, 3, 4, 5]);
}

#[test]
fn interface_lines_of_a_mention_are_empty() {
    let text = "fn caller() {\n    f();\n}\n\nfn other() {}\n";
    assert!(interface_lines("f.rs", text, "f").is_empty());
}

#[test]
fn interface_lines_of_a_qualified_name_are_the_scoped_definition() {
    let text = "pub enum EnvRule {\n    /// Doc.\n    Unverified,\n}\n\nfn r() {\n    EnvRule::Unverified\n}\n";
    assert_eq!(interface_lines("f.rs", text, "EnvRule::Unverified"), [2, 3]);
    assert!(interface_lines("f.rs", text, "Other::Unverified").is_empty());
}

#[test]
fn interface_lines_do_not_cross_a_blank_line_up() {
    let text = "/// a\n\n/// b\n\n\n#[x]\nfn f() {}\n";
    assert_eq!(interface_lines("f.rs", text, "f"), [6, 7]);
}

#[test]
fn interface_lines_of_a_python_docstring_with_a_blank_then_column_zero_line_are_one_span() {
    let text =
        "def f():\n    \"\"\"Doc.\n\ncol0\n    \"\"\"\n    return 1\n\n\ndef g():\n    pass\n";
    assert_eq!(interface_lines("f.py", text, "f"), [1, 2, 3, 4, 5, 6]);
}

#[test]
fn interface_lines_of_a_rust_raw_string_with_a_lone_brace_are_one_span() {
    let text = "/// Doc.\npub fn f() {\n    let s = r#\"\n}\n\n\"#;\n}\n\nfn g() {}\n";
    assert_eq!(interface_lines("f.rs", text, "f"), [1, 2, 3, 4, 5, 6, 7]);
}

#[test]
fn interface_lines_keep_the_text_block_rule_for_java_and_nothing_for_other_files() {
    let java = "class A {\n    void f() {}\n}\n";
    assert!(!interface_lines("A.java", java, "f").is_empty());
    assert!(interface_lines("A.md", "# f\n", "f").is_empty());
    assert!(interface_lines("A.baml", "function f() {}\n", "f").is_empty());
}

#[test]
fn missing_file_is_dead_path() {
    let t = repo_with("f.txt", b"x");
    assert_eq!(
        resolve(t.path(), &anchor("nope.txt", "x")),
        Resolution::DeadPath
    );
}

#[test]
fn rust_name_with_double_colon_resolves() {
    let t = repo_with(
        "f.txt",
        b"impl Router {\n    fn x() {}\n}\n// Router::owner here\n",
    );
    assert_eq!(
        resolve(t.path(), &anchor("f.txt", "Router::owner")),
        Resolution::Found { line: 4 }
    );
}

#[test]
fn name_at_file_start_and_end() {
    let t = repo_with("f.txt", b"start\nmiddle\nend");
    assert_eq!(
        resolve(t.path(), &anchor("f.txt", "start")),
        Resolution::Found { line: 1 }
    );
    assert_eq!(
        resolve(t.path(), &anchor("f.txt", "end")),
        Resolution::Found { line: 3 }
    );
}

#[test]
fn window_clamps_at_file_start() {
    let t = repo_with("f.txt", b"a\nb\nc\nd\n");
    assert_eq!(window(t.path(), "a", 5, 1).unwrap(), "a\nb");
    assert_eq!(
        source_window(t.path(), &anchor("f.txt", "a"), 5, 1)
            .unwrap()
            .0,
        1
    );
}

#[test]
fn window_clamps_at_file_end() {
    let t = repo_with("f.txt", b"a\nb\nc\nd\n");
    assert_eq!(window(t.path(), "d", 1, 5).unwrap(), "c\nd");
}

#[test]
fn window_takes_lines_around_the_match() {
    let t = repo_with("f.txt", b"a\nb\nc\nd\ne\n");
    assert_eq!(window(t.path(), "c", 1, 1).unwrap(), "b\nc\nd");
}

#[test]
fn window_of_a_dead_anchor_is_none() {
    let t = repo_with("f.txt", b"a\n");
    assert_eq!(window(t.path(), "zzz", 1, 1), None);
    assert_eq!(source_window(t.path(), &anchor("no.txt", "a"), 1, 1), None);
}

#[test]
fn invalid_utf8_does_not_panic() {
    let t = repo_with("f.txt", b"\xff\xfe\nname\n\xc0");
    assert_eq!(
        resolve(t.path(), &anchor("f.txt", "name")),
        Resolution::Found { line: 2 }
    );
    assert_eq!(window(t.path(), "name", 0, 0).unwrap(), "name");
}

#[test]
fn absolute_path_is_dead_path_and_has_no_window() {
    let outside = repo_with("secret.txt", b"BEGIN\n");
    let abs = outside
        .path()
        .join("secret.txt")
        .to_string_lossy()
        .into_owned();
    let root = repo_with("f.txt", b"x");
    let a = anchor(&abs, "BEGIN");
    assert_eq!(resolve(root.path(), &a), Resolution::DeadPath);
    assert_eq!(source_window(root.path(), &a, 1, 1), None);
}

#[test]
fn parent_dir_path_is_dead_path_and_has_no_window() {
    let outer = repo_with("secret.txt", b"BEGIN\n");
    let inner = outer.path().join("repo");
    std::fs::create_dir(&inner).unwrap();
    let a = anchor("../secret.txt", "BEGIN");
    assert_eq!(resolve(&inner, &a), Resolution::DeadPath);
    assert_eq!(source_window(&inner, &a, 1, 1), None);
}

#[test]
fn empty_name_is_dead_name() {
    let t = repo_with("f.txt", b"abc\n");
    let a = anchor("f.txt", "");
    assert_eq!(resolve(t.path(), &a), Resolution::DeadName);
    assert_eq!(source_window(t.path(), &a, 1, 1), None);
}

#[test]
fn empty_file_is_dead_name_without_panic() {
    let t = repo_with("f.txt", b"");
    assert_eq!(
        resolve(t.path(), &anchor("f.txt", "x")),
        Resolution::DeadName
    );
    assert_eq!(window(t.path(), "x", 1, 1), None);
    assert_eq!(window(t.path(), "", 1, 1), None);
}

#[test]
fn match_overlapping_a_rejected_match_is_found() {
    let t = repo_with("f.txt", b"ax-x-x\n");
    assert_eq!(
        resolve(t.path(), &anchor("f.txt", "x-x")),
        Resolution::Found { line: 1 }
    );
}

#[test]
fn multibyte_neighbours_are_not_word_characters() {
    let t = repo_with("f.txt", "\u{e9}name\nname\u{e9}\n".as_bytes());
    assert_eq!(
        resolve(t.path(), &anchor("f.txt", "name")),
        Resolution::Found { line: 1 }
    );
    let t = repo_with("f.txt", "name\u{e9}\n".as_bytes());
    assert_eq!(
        resolve(t.path(), &anchor("f.txt", "name")),
        Resolution::Found { line: 1 }
    );
}

#[cfg(unix)]
mod symlinks {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn symlinked_file_pointing_outside_the_repo_is_dead_path() {
        let outside = repo_with("secret.txt", b"BEGIN\n");
        let root = repo_with("f.txt", b"x");
        symlink(
            outside.path().join("secret.txt"),
            root.path().join("link.txt"),
        )
        .unwrap();
        let a = anchor("link.txt", "BEGIN");
        assert_eq!(resolve(root.path(), &a), Resolution::DeadPath);
        assert_eq!(source_window(root.path(), &a, 1, 1), None);
    }

    #[test]
    fn symlinked_file_pointing_inside_the_repo_is_dead_path() {
        let root = repo_with(".env", b"TOKEN=1\n");
        symlink(".env", root.path().join("link.txt")).unwrap();
        let a = anchor("link.txt", "TOKEN");
        assert_eq!(resolve(root.path(), &a), Resolution::DeadPath);
        assert_eq!(source_window(root.path(), &a, 1, 1), None);
    }

    #[test]
    fn real_file_under_a_symlinked_directory_is_dead_path() {
        let outside = repo_with("secret.txt", b"BEGIN\n");
        let root = repo_with("f.txt", b"x");
        symlink(outside.path(), root.path().join("dir")).unwrap();
        let a = anchor("dir/secret.txt", "BEGIN");
        assert_eq!(resolve(root.path(), &a), Resolution::DeadPath);
        assert_eq!(source_window(root.path(), &a, 1, 1), None);
    }
}

fn found(file: &str, source: &str, name: &str) -> Resolution {
    resolve(
        repo_with(file, source.as_bytes()).path(),
        &anchor(file, name),
    )
}

/// A source file with `name` defined on `line` and mentioned elsewhere (`mention`, a comment or
/// a call); the definition is `Found`, the mention alone is `DeadName`.
fn assert_defined(file: &str, source: &str, mention: &str, name: &str, line: u32) {
    assert_eq!(
        found(file, source, name),
        Resolution::Found { line },
        "{file}"
    );
    assert_eq!(found(file, mention, name), Resolution::DeadName, "{file}");
}

#[test]
fn every_static_grammar_resolves_a_definition_and_not_a_mention() {
    let cases = [
        (
            "f.rs",
            "// x\nfn target() {}\n",
            "// target\nfn other() { target(); }\n",
        ),
        (
            "f.py",
            "# x\ndef target():\n    pass\n",
            "# target\ndef other():\n    target()\n",
        ),
        (
            "f.ts",
            "// x\nfunction target() {}\n",
            "// target\nfunction other() { target(); }\n",
        ),
        (
            "f.ts",
            "// x\ninterface target {}\n",
            "// target\nfunction other() { target(); }\n",
        ),
        (
            "f.tsx",
            "// x\nclass target {}\n",
            "// target\nfunction other() { target(); }\n",
        ),
        (
            "f.js",
            "// x\nfunction target() {}\n",
            "// target\nfunction other() { target(); }\n",
        ),
        (
            "f.jsx",
            "// x\nconst target = () => 1;\n",
            "// target\nfunction other() { target(); }\n",
        ),
        (
            "f.go",
            "// x\nfunc target() {}\n",
            "// target\nfunc other() { target() }\n",
        ),
        (
            "f.sh",
            "# x\ntarget() {\n  :\n}\n",
            "# target\nother() {\n  target\n}\n",
        ),
        (
            "f.c",
            "/* x */\nint target(void) { return 0; }\n",
            "/* target */\nint other(void) { return target(); }\n",
        ),
        (
            "f.h",
            "/* x */\nint target(void);\n",
            "/* target */\nint other(void);\n",
        ),
        (
            "f.cpp",
            "// x\nclass target {};\n",
            "// target\nint other() { target t; return 0; }\n",
        ),
        (
            "f.hpp",
            "// x\nint target() { return 0; }\n",
            "// target\nint other() { return target(); }\n",
        ),
        (
            "f.cs",
            "// x\nclass target {}\n",
            "// target\nclass Other { void M() { target(); } }\n",
        ),
        (
            "f.tf",
            "# x\nvariable \"target\" {}\n",
            "# target\nvariable \"other\" {\n  default = \"target\"\n}\n",
        ),
        ("f.yaml", "# x\ntarget: 1\n", "# target\nother: target\n"),
        ("f.yml", "# x\ntarget: 1\n", "# target\nother: target\n"),
    ];
    for (file, source, mention) in cases {
        assert_defined(file, source, mention, "target", 2);
    }
}

#[test]
fn the_remaining_extensions_of_each_grammar_resolve() {
    for (file, source) in [
        ("f.mts", "function target() {}\n"),
        ("f.cts", "function target() {}\n"),
        ("f.mjs", "function target() {}\n"),
        ("f.cjs", "function target() {}\n"),
        ("f.bash", "target() {\n  :\n}\n"),
        ("f.cc", "int target() { return 0; }\n"),
        ("f.cxx", "int target() { return 0; }\n"),
        ("f.hh", "int target() { return 0; }\n"),
        ("f.hcl", "variable \"target\" {}\n"),
    ] {
        assert_eq!(
            found(file, source, "target"),
            Resolution::Found { line: 1 },
            "{file}"
        );
    }
}

#[test]
fn a_rust_method_is_alive_under_its_impl_type_only() {
    let src = "struct Router;\nimpl Router {\n    fn owner(&self) {}\n}\nimpl<T> Gen<T> {\n    fn made() {}\n}\n";
    assert_eq!(
        found("f.rs", src, "Router::owner"),
        Resolution::Found { line: 3 }
    );
    assert_eq!(found("f.rs", src, "Other::owner"), Resolution::DeadName);
    assert_eq!(
        found("f.rs", src, "Gen::made"),
        Resolution::Found { line: 6 }
    );
    assert_eq!(found("f.rs", src, "owner"), Resolution::Found { line: 3 });
}

#[test]
fn a_rust_enum_variant_const_static_and_field_are_definitions() {
    let src =
        "enum E {\n    A,\n}\nconst C: u8 = 1;\nstatic S: u8 = 2;\nstruct P {\n    x: u8,\n}\n";
    assert_eq!(found("f.rs", src, "E::A"), Resolution::Found { line: 2 });
    assert_eq!(found("f.rs", src, "E::B"), Resolution::DeadName);
    assert_eq!(found("f.rs", src, "C"), Resolution::Found { line: 4 });
    assert_eq!(found("f.rs", src, "S"), Resolution::Found { line: 5 });
    assert_eq!(found("f.rs", src, "P::x"), Resolution::Found { line: 7 });
    assert_eq!(found("f.rs", "fn f() { C }\n", "C"), Resolution::DeadName);
}

#[test]
fn a_python_method_is_alive_under_its_class() {
    let src = "class K:\n    def m(self):\n        pass\n";
    assert_eq!(found("f.py", src, "K.m"), Resolution::Found { line: 2 });
    assert_eq!(found("f.py", src, "Z.m"), Resolution::DeadName);
}

#[test]
fn a_yaml_nested_key_resolves_by_its_dotted_path_and_a_whole_dotted_key_too() {
    let src = "a:\n  b: 1\nc:\n  d: 2\nx.y: 3\n";
    assert_eq!(found("f.yaml", src, "a.b"), Resolution::Found { line: 2 });
    assert_eq!(found("f.yaml", src, "c.b"), Resolution::DeadName);
    assert_eq!(found("f.yaml", src, "x.y"), Resolution::Found { line: 5 });
}

#[test]
fn an_hcl_block_is_named_by_its_labels() {
    let src = "resource \"aws_s3_bucket\" \"logs\" {\n  bucket = \"b\"\n}\nvariable \"region\" {}\nlocals {}\n";
    assert_eq!(
        found("main.tf", src, "aws_s3_bucket.logs"),
        Resolution::Found { line: 1 }
    );
    assert_eq!(
        found("main.tf", src, "region"),
        Resolution::Found { line: 4 }
    );
    assert_eq!(found("main.tf", src, "logs"), Resolution::DeadName);
    assert_eq!(found("main.tf", src, "bucket"), Resolution::DeadName);
}

#[test]
fn baml_and_markdown_still_resolve_by_text() {
    assert_eq!(
        found("f.baml", "function Target() {}\n", "Target"),
        Resolution::Found { line: 1 }
    );
    assert_eq!(
        found("f.md", "# a\nsee target here\n", "target"),
        Resolution::Found { line: 2 }
    );
}

#[test]
fn a_span_includes_its_doc_comment_and_attribute_and_stops_at_a_blank_line() {
    let t = repo_with(
        "f.rs",
        b"// far\n\n/// Doc.\n#[inline]\nfn f() {\n    1\n}\n\nfn g() {}\n",
    );
    let (start, text) = source_window(t.path(), &anchor("f.rs", "f"), 20, 20).unwrap();
    assert_eq!(
        (start, text.as_str()),
        (3, "/// Doc.\n#[inline]\nfn f() {\n    1\n}")
    );
}

#[test]
fn a_span_holds_a_docstring_blank_line_and_a_raw_string_brace() {
    let t = repo_with(
        "f.py",
        b"def f():\n    \"\"\"Doc.\n\ncol0\n    \"\"\"\n    return 1\n\n\ndef g():\n    pass\n",
    );
    let (_, text) = source_window(t.path(), &anchor("f.py", "f"), 20, 20).unwrap();
    assert_eq!(text.lines().count(), 6);
    let t = repo_with(
        "f.rs",
        b"fn f() {\n    let s = r#\"\n}\n\n\"#;\n}\n\nfn g() {}\n",
    );
    let (_, text) = source_window(t.path(), &anchor("f.rs", "f"), 20, 20).unwrap();
    assert_eq!(text.lines().count(), 6);
}

#[test]
fn a_long_span_is_cut_at_41_lines_then_says_how_many_more() {
    let mut src = String::from("fn f() {\n");
    src += &"    let x = 1;\n".repeat(198);
    src += "}\n";
    let t = repo_with("f.rs", src.as_bytes());
    let (start, text) = source_window(t.path(), &anchor("f.rs", "f"), 20, 20).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!((start, lines.len()), (1, 42));
    assert_eq!(lines[41], "… (159 more lines)");
}

#[test]
fn a_c_function_span_covers_its_body_not_only_its_signature() {
    let t = repo_with(
        "f.c",
        b"/* Doc. */\nstatic int *f(int a)\n{\n    return 0;\n}\n\nint g(void);\n",
    );
    assert_eq!(
        interface_lines(
            "f.c",
            "/* Doc. */\nstatic int *f(int a)\n{\n    return 0;\n}\n\nint g(void);\n",
            "f"
        ),
        [1, 2, 3, 4, 5]
    );
    let (_, text) = source_window(t.path(), &anchor("f.c", "f"), 20, 20).unwrap();
    assert_eq!(text.lines().count(), 5);
}

#[test]
fn a_python_decorator_and_an_exported_ts_function_are_inside_the_span() {
    assert_eq!(
        interface_lines("f.py", "# c\n@d\ndef f():\n    pass\n", "f"),
        [1, 2, 3, 4]
    );
    assert_eq!(
        interface_lines("f.ts", "/** Doc. */\nexport function f() {\n}\n", "f"),
        [1, 2, 3]
    );
}

#[test]
fn a_trailing_comment_of_the_previous_item_is_not_in_the_span() {
    assert_eq!(
        interface_lines("f.rs", "let_x!(); // note\nfn f() {}\n", "f"),
        [2]
    );
}

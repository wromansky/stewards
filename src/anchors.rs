// Anchor resolution and source windows: tree-sitter definitions for the static grammars, a
// whole-word text rule for every other file.
use std::ops::Range;
use std::path::{Component, Path};
use std::sync::OnceLock;

use tree_sitter::{Language, Node, Parser, Query, QueryCursor, StreamingIterator};

use crate::model::Anchor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    Found { line: u32 },
    DeadPath,
    DeadName,
}

/// A static grammar: the extensions it parses and the queries that say what a definition is.
/// A capture `@name` inside a `@definition.*` capture names a definition; one inside a
/// `@container` capture names a scope that qualifies definitions without defining anything.
struct Grammar {
    exts: &'static [&'static str],
    language: fn() -> Language,
    /// The grammar's own tags query, then this repo's supplement (`src/anchors/*.scm`).
    queries: &'static [&'static str],
    query: OnceLock<Query>,
}

const fn grammar(
    exts: &'static [&'static str],
    language: fn() -> Language,
    queries: &'static [&'static str],
) -> Grammar {
    Grammar {
        exts,
        language,
        queries,
        query: OnceLock::new(),
    }
}

// The TypeScript crate's tags query only adds to the JavaScript one.
static GRAMMARS: [Grammar; 12] = [
    grammar(
        &["rs"],
        || tree_sitter_rust::LANGUAGE.into(),
        &[
            tree_sitter_rust::TAGS_QUERY,
            include_str!("anchors/rust.scm"),
        ],
    ),
    grammar(
        &["py"],
        || tree_sitter_python::LANGUAGE.into(),
        &[tree_sitter_python::TAGS_QUERY],
    ),
    grammar(
        &["ts", "mts", "cts"],
        || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        &[
            tree_sitter_javascript::TAGS_QUERY,
            tree_sitter_typescript::TAGS_QUERY,
        ],
    ),
    grammar(
        &["tsx"],
        || tree_sitter_typescript::LANGUAGE_TSX.into(),
        &[
            tree_sitter_javascript::TAGS_QUERY,
            tree_sitter_typescript::TAGS_QUERY,
        ],
    ),
    grammar(
        &["js", "jsx", "mjs", "cjs"],
        || tree_sitter_javascript::LANGUAGE.into(),
        &[tree_sitter_javascript::TAGS_QUERY],
    ),
    grammar(
        &["go"],
        || tree_sitter_go::LANGUAGE.into(),
        &[tree_sitter_go::TAGS_QUERY],
    ),
    grammar(
        &["sh", "bash"],
        || tree_sitter_bash::LANGUAGE.into(),
        &[include_str!("anchors/bash.scm")],
    ),
    // `.h` is C: a C++ header is parsed as C and its classes are not definitions
    grammar(
        &["c", "h"],
        || tree_sitter_c::LANGUAGE.into(),
        &[tree_sitter_c::TAGS_QUERY],
    ),
    grammar(
        &["cc", "cpp", "cxx", "hpp", "hh"],
        || tree_sitter_cpp::LANGUAGE.into(),
        &[tree_sitter_cpp::TAGS_QUERY],
    ),
    grammar(
        &["cs"],
        || tree_sitter_c_sharp::LANGUAGE.into(),
        &[tree_sitter_c_sharp::TAGS_QUERY],
    ),
    grammar(
        &["tf", "hcl"],
        || tree_sitter_hcl::LANGUAGE.into(),
        &[include_str!("anchors/hcl.scm")],
    ),
    grammar(
        &["yml", "yaml"],
        || tree_sitter_yaml::LANGUAGE.into(),
        &[include_str!("anchors/yaml.scm")],
    ),
];

/// Extensions with no grammar that still get the text block rule of `interface_lines`.
const BLOCK_LANGUAGES: &[&str] = &["java", "kt", "rb", "php", "swift", "scala"];

fn extension(path: &str) -> Option<&str> {
    Path::new(path).extension()?.to_str()
}

fn grammar_of(path: &str) -> Option<&'static Grammar> {
    let ext = extension(path)?;
    GRAMMARS.iter().find(|g| g.exts.contains(&ext))
}

/// A definition (or, with `container`, a scope) of a grammar language.
struct Def {
    name: String,
    /// 1-based line of the name.
    line: u32,
    /// The captured definition node's bytes: what a qualified name's scope must contain.
    node: Range<usize>,
    /// 1-based inclusive lines of the span: the node (widened to its declaration wrapper) and the
    /// comments and attributes directly above it.
    lines: (u32, u32),
    container: bool,
}

/// Nodes a definition's own node sits in without them being another item: Python decorators,
/// TypeScript exports, C++ templates, a JavaScript `const f = () => {}`.
const WRAPPERS: &[&str] = &[
    "decorated_definition",
    "export_statement",
    "template_declaration",
    "lexical_declaration",
    "variable_declaration",
    "expression_statement",
];

/// A C declarator (the grammar's tags capture the signature only) up to the whole declaration.
fn outermost(mut n: Node) -> Node {
    while let Some(p) = n.parent() {
        let (k, pk) = (n.kind(), p.kind());
        let declarator = k.ends_with("declarator")
            && (pk.ends_with("declarator")
                || ["function_definition", "declaration", "field_declaration"].contains(&pk));
        if !(declarator || WRAPPERS.contains(&pk)) {
            break;
        }
        n = p;
    }
    n
}

/// The row a node's last character is on: a node that ends with a newline ends on the row above.
fn last_row(n: Node) -> usize {
    let end = n.end_position();
    if end.column == 0 && end.row > n.start_position().row {
        end.row - 1
    } else {
        end.row
    }
}

/// A comment or attribute that directly precedes `next` on its own line(s).
fn leads(prev: Node, next: Node) -> bool {
    let item = prev.kind().contains("comment") || prev.kind() == "attribute_item";
    let trailing = prev
        .prev_sibling()
        .is_some_and(|p| last_row(p) == prev.start_position().row);
    item && !trailing && last_row(prev) + 1 >= next.start_position().row
}

/// An HCL block's labels joined with `.`; `None` for a block with none (`locals`).
fn hcl_name(block: Node, text: &str) -> Option<String> {
    let mut cursor = block.walk();
    let labels: Vec<&str> = block
        .children(&mut cursor)
        .skip(1)
        .take_while(|c| c.kind() != "block_start")
        .filter_map(|c| Some(text.get(c.byte_range())?.trim_matches('"')))
        .collect();
    (!labels.is_empty()).then(|| labels.join("."))
}

/// The definitions and containers of `text` in file order; empty when the grammar can't parse.
fn definitions(g: &Grammar, text: &str) -> Vec<Def> {
    let language = (g.language)();
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return vec![];
    }
    let Some(tree) = parser.parse(text, None) else {
        return vec![];
    };
    let query = g.query.get_or_init(|| {
        Query::new(&language, &g.queries.join("\n")).expect("anchor query compiles")
    });
    let capture = |i: u32| query.capture_names()[i as usize];
    let mut defs = vec![];
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), text.as_bytes());
    while let Some(m) = matches.next() {
        let Some(item) = m.captures().iter().find(|c| {
            capture(c.index).starts_with("definition.") || capture(c.index) == "container"
        }) else {
            continue;
        };
        let mut named: Vec<(String, Node)> = m
            .captures()
            .iter()
            .filter(|c| capture(c.index) == "name")
            .filter_map(|c| Some((text.get(c.node.byte_range())?.to_string(), c.node)))
            .collect();
        if named.is_empty() && item.node.kind() == "block" {
            named.extend(hcl_name(item.node, text).map(|n| (n, item.node)));
        }
        let outer = outermost(item.node);
        let mut first = outer;
        while let Some(p) = first.prev_sibling().filter(|p| leads(*p, first)) {
            first = p;
        }
        defs.extend(named.into_iter().map(|(name, at)| Def {
            name,
            line: at.start_position().row as u32 + 1,
            node: item.node.byte_range(),
            lines: (
                first.start_position().row as u32 + 1,
                last_row(outer) as u32 + 1,
            ),
            container: capture(item.index) == "container",
        }));
    }
    defs.sort_by_key(|d| (d.node.start, d.node.end, d.line));
    // a Rust method is captured as a method and as a function
    defs.dedup_by(|a, b| (&a.name, &a.node, a.line) == (&b.name, &b.node, b.line));
    defs
}

/// The definitions `name` names: those whose name is the whole `name` (HCL blocks, YAML keys
/// with dots), else those named by its last segment inside a scope named by the one before it.
fn matching<'a>(defs: &'a [Def], name: &str) -> Vec<&'a Def> {
    let whole: Vec<&Def> = defs
        .iter()
        .filter(|d| !d.container && d.name == name)
        .collect();
    if !whole.is_empty() {
        return whole;
    }
    let segments: Vec<&str> = name.split([':', '.']).filter(|s| !s.is_empty()).collect();
    let [.., scope, last] = segments[..] else {
        return vec![];
    };
    let inside = |d: &Def| {
        defs.iter().any(|c| {
            c.name == scope
                && c.node != d.node
                && c.node.start <= d.node.start
                && d.node.end <= c.node.end
        })
    };
    defs.iter()
        .filter(|d| !d.container && d.name == last && inside(d))
        .collect()
}

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// The file's text (invalid UTF-8 replaced). `None` when the path is not a plain relative path
/// inside `root`, passes through a symlink, or the file can't be read.
fn read(root: &Path, a: &Anchor) -> Option<String> {
    let mut full = root.to_path_buf();
    for c in Path::new(&a.path).components() {
        let Component::Normal(c) = c else {
            return None;
        };
        full.push(c);
        // a symlink could point outside the repo or at a file excluded from packs
        if std::fs::symlink_metadata(&full)
            .ok()?
            .file_type()
            .is_symlink()
        {
            return None;
        }
    }
    Some(String::from_utf8_lossy(&std::fs::read(&full).ok()?).into_owned())
}

/// Byte offsets of every whole-word `name` in `text`.
fn occurrences<'a>(text: &'a str, name: &'a str) -> impl Iterator<Item = usize> + 'a {
    let bytes = text.as_bytes();
    let mut start = 0;
    std::iter::from_fn(move || {
        while !name.is_empty() {
            let i = start + text[start..].find(name)?;
            // Step one char, not past the match, so an overlapping match is still seen.
            start = i + text[i..].chars().next().map_or(1, char::len_utf8);
            let end = i + name.len();
            if (i == 0 || !is_word(bytes[i - 1])) && (end == bytes.len() || !is_word(bytes[end])) {
                return Some(i);
            }
        }
        None
    })
}

/// The 1-based lines of `name`'s blocks in `text`, sorted; empty when `name` does not occur.
/// A qualified name (`a::b`, `a.b`) also matches its last segment. A block starts at an
/// occurrence's line and runs down until a line whose previous line is blank, whose indent
/// (a tab is 8 columns) is at most the occurrence line's, and before which the brackets opened
/// since the occurrence line are all closed; such a line that is a comment ends the block only
/// when the next line that is neither blank nor a comment would too (or there is none).
/// Trailing blanks are dropped. Above, it takes the contiguous non-blank lines, then, when the
/// nearest non-blank line above their blank lines opens with `/`, `*`, `#` or `@` (a detached
/// doc comment or attribute), those blanks and the run above them, once.
// simplified: text blocks around every whole-word occurrence, for `BLOCK_LANGUAGES` only; a block
// can end early, at a blank line then a column-0 line inside a triple-quoted string; upgrade by
// adding the language's grammar.
fn text_lines(text: &str, name: &str) -> Vec<u32> {
    let lines: Vec<&str> = text.lines().collect();
    let blank = |i: usize| lines[i].trim().is_empty();
    let indent = |i: usize| {
        let pad = &lines[i][..lines[i].len() - lines[i].trim_start().len()];
        pad.chars()
            .map(|c| if c == '\t' { 8 } else { 1 })
            .sum::<usize>()
    };
    let depth = |i: usize| {
        lines[i]
            .chars()
            .map(|c| match c {
                '(' | '[' | '{' => 1,
                ')' | ']' | '}' => -1,
                _ => 0,
            })
            .sum::<i64>()
    };
    let last = name.rsplit("::").next().unwrap_or(name);
    let last = last.rsplit('.').next().unwrap_or(last);
    let mut names = vec![name];
    if last != name {
        names.push(last);
    }
    let comment = |i: usize| {
        let l = lines[i].trim_start();
        ["#", "//", "/*", "*", "--"]
            .iter()
            .any(|m| l.starts_with(m))
    };
    let mut out = Vec::new();
    for at in names.into_iter().flat_map(|n| occurrences(text, n)) {
        let k = text[..at].matches('\n').count();
        let ind = indent(k);
        let mut open = depth(k);
        let mut end = k;
        for j in k + 1..lines.len() {
            if !blank(j) && blank(j - 1) && indent(j) <= ind && open <= 0 {
                // a comment at the item's indent may sit inside its body (Python, Ruby)
                let code = (j + 1..lines.len()).find(|&n| !blank(n) && !comment(n));
                let ends =
                    |n: usize| indent(n) <= ind && open + (j..n).map(depth).sum::<i64>() <= 0;
                if !comment(j) || code.is_none_or(ends) {
                    break;
                }
            }
            open += depth(j);
            end = j;
        }
        while end > k && blank(end) {
            end -= 1;
        }
        let run_above = |mut i: usize| {
            while i > 0 && !blank(i - 1) {
                i -= 1;
            }
            i
        };
        let mut start = run_above(k);
        if let Some(i) = (0..start).rev().find(|&i| !blank(i))
            && lines[i].trim_start().starts_with(['/', '*', '#', '@'])
        {
            start = run_above(i);
        }
        out.extend(start as u32 + 1..=end as u32 + 1);
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// The 1-based lines of every span of a definition named `name` (comments and attributes above
/// included), sorted. For a language without a grammar, the text rule's blocks, only for
/// `BLOCK_LANGUAGES`. Empty when the lines can't be found: another language, `name` not defined.
pub fn interface_lines(path: &str, text: &str, name: &str) -> Vec<u32> {
    if let Some(g) = grammar_of(path) {
        let defs = definitions(g, text);
        let mut lines: Vec<u32> = matching(&defs, name)
            .iter()
            .flat_map(|d| d.lines.0..=d.lines.1)
            .collect();
        lines.sort_unstable();
        lines.dedup();
        lines
    } else if extension(path).is_some_and(|e| BLOCK_LANGUAGES.contains(&e)) {
        text_lines(text, name)
    } else {
        vec![]
    }
}

fn line_of(text: &str, at: usize) -> u32 {
    text[..at].matches('\n').count() as u32 + 1
}

pub fn resolve(root: &Path, a: &Anchor) -> Resolution {
    let Some(text) = read(root, a) else {
        return Resolution::DeadPath;
    };
    let line = match grammar_of(&a.path) {
        Some(g) => matching(&definitions(g, &text), &a.name)
            .first()
            .map(|d| d.line),
        None => occurrences(&text, &a.name)
            .next()
            .map(|at| line_of(&text, at)),
    };
    line.map_or(Resolution::DeadName, |line| Resolution::Found { line })
}

/// The first line of the source shown for the anchor and the source. With a grammar it is the
/// definition's span cut to `before + after + 1` lines, then `… (<n> more lines)`; otherwise the
/// match's line plus `before` lines above and `after` below, clamped to the file.
pub fn source_window(
    root: &Path,
    a: &Anchor,
    before: usize,
    after: usize,
) -> Option<(u32, String)> {
    let text = read(root, a)?;
    let lines: Vec<&str> = text.lines().collect();
    let Some(g) = grammar_of(&a.path) else {
        let line = line_of(&text, occurrences(&text, &a.name).next()?) as usize - 1;
        let last = (line + after).min(lines.len().checked_sub(1)?);
        let first = line.saturating_sub(before);
        return Some((first as u32 + 1, lines.get(first..=last)?.join("\n")));
    };
    let (first, last) = matching(&definitions(g, &text), &a.name).first()?.lines;
    let cap = before + after + 1;
    let span = lines.get(first as usize - 1..last as usize)?;
    let mut shown = span[..span.len().min(cap)].join("\n");
    if span.len() > cap {
        shown += &format!("\n… ({} more lines)", span.len() - cap);
    }
    Some((first, shown))
}

// Anchor resolution and source windows.
use std::path::{Component, Path};

use crate::model::Anchor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    Found { line: u32 },
    DeadPath,
    DeadName,
}

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// The file's text (invalid UTF-8 replaced) and the byte offset of the first whole-word `name`.
/// `None` when the path is not a plain relative path inside `root`, passes through a symlink,
/// or the file can't be read.
fn find(root: &Path, a: &Anchor) -> Option<(String, Option<usize>)> {
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
    let text = String::from_utf8_lossy(&std::fs::read(&full).ok()?).into_owned();
    let at = occurrences(&text, &a.name).next();
    Some((text, at))
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
/// A block is an occurrence's line, the lines below while blank, indented deeper, or at the same
/// indent and opening with `)`, `]` or `}` (trailing blanks dropped), and the contiguous
/// non-blank lines directly above at the same indent (doc comments, attributes).
// simplified: text blocks around every whole-word occurrence; exact spans come with tree-sitter.
pub fn interface_lines(text: &str, name: &str) -> Vec<u32> {
    let lines: Vec<&str> = text.lines().collect();
    let blank = |i: usize| lines[i].trim().is_empty();
    let indent = |i: usize| lines[i].len() - lines[i].trim_start().len();
    let mut out = Vec::new();
    for at in occurrences(text, name) {
        let k = text[..at].matches('\n').count();
        let ind = indent(k);
        let closes =
            |j: usize| indent(j) == ind && lines[j].trim_start().starts_with([')', ']', '}']);
        let mut end = (k + 1..lines.len())
            .take_while(|&j| blank(j) || indent(j) > ind || closes(j))
            .last()
            .unwrap_or(k);
        while end > k && blank(end) {
            end -= 1;
        }
        let mut start = k;
        while start > 0 && !blank(start - 1) && indent(start - 1) == ind {
            start -= 1;
        }
        out.extend(start as u32 + 1..=end as u32 + 1);
    }
    out.sort_unstable();
    out.dedup();
    out
}

pub fn resolve(root: &Path, a: &Anchor) -> Resolution {
    match find(root, a) {
        None => Resolution::DeadPath,
        Some((_, None)) => Resolution::DeadName,
        Some((text, Some(at))) => Resolution::Found {
            line: text[..at].matches('\n').count() as u32 + 1,
        },
    }
}

/// The match's line plus `before` lines above and `after` below, clamped to the file.
pub fn source_window(root: &Path, a: &Anchor, before: usize, after: usize) -> Option<String> {
    let (text, at) = find(root, a)?;
    let line = text[..at?].matches('\n').count();
    let lines: Vec<&str> = text.lines().collect();
    let last = (line + after).min(lines.len().checked_sub(1)?);
    Some(lines.get(line.saturating_sub(before)..=last)?.join("\n"))
}

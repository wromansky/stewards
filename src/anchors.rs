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
    let bytes = text.as_bytes();
    let mut start = 0;
    let mut at = None;
    while !a.name.is_empty() {
        let Some(off) = text[start..].find(&a.name) else {
            break;
        };
        let i = start + off;
        let end = i + a.name.len();
        if (i == 0 || !is_word(bytes[i - 1])) && (end == bytes.len() || !is_word(bytes[end])) {
            at = Some(i);
            break;
        }
        // Step one char, not past the match, so an overlapping match is still seen.
        start = i + text[i..].chars().next().map_or(1, char::len_utf8);
    }
    Some((text, at))
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

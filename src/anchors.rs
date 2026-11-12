// Anchor resolution and source windows.
use std::path::Path;

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
fn find(root: &Path, a: &Anchor) -> Option<(String, Option<usize>)> {
    let text = String::from_utf8_lossy(&std::fs::read(root.join(&a.path)).ok()?).into_owned();
    let bytes = text.as_bytes();
    let at = text.match_indices(&a.name).map(|(i, _)| i).find(|&i| {
        let end = i + a.name.len();
        (i == 0 || !is_word(bytes[i - 1])) && (end == bytes.len() || !is_word(bytes[end]))
    });
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
    let last = (line + after).min(lines.len() - 1);
    Some(lines[line.saturating_sub(before)..=last].join("\n"))
}

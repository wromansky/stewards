// `docs`: renders docs_path/<unit>.md and docs_path/README.md from the cards.
use std::fmt::Write as _;
use std::path::{Component, Path};
use std::process::ExitCode;

use serde::Serialize;

use crate::anchors::{Resolution, resolve};
use crate::model::{Anchor, GapKind, Loaded, UnitDef};
use crate::output::{CliError, Global, Human, emit};
use crate::workspace::discover;

const NOT_WRITTEN: &str = "_Not written yet._";

#[derive(Serialize)]
pub struct DocsReport {
    /// Repo-relative paths of the files whose content changed.
    pub wrote: Vec<String>,
}

impl Human for DocsReport {
    fn render(&self) -> String {
        if self.wrote.is_empty() {
            return "Docs are up to date.".into();
        }
        let lines: Vec<String> = self
            .wrote
            .iter()
            .map(|p| format!("- wrote `{p}`"))
            .collect();
        lines.join("\n")
    }
}

pub fn run(g: &Global, unit: Option<&str>) -> anyhow::Result<ExitCode> {
    let cwd = std::fs::canonicalize(std::env::current_dir()?)?;
    let ws = discover(&cwd, g.repo.as_deref())?;
    let repo = match ws.repos.as_slice() {
        [only] => only,
        many => many
            .iter()
            .find(|r| std::fs::canonicalize(&r.root).is_ok_and(|root| cwd.starts_with(root)))
            .ok_or_else(|| {
                CliError::new(
                    "not_in_repo",
                    "the current directory is in no repo of this workspace",
                    "run inside a repo or pass --repo <name>",
                )
            })?,
    };
    let loaded = repo.loaded.as_ref().ok_or_else(|| {
        CliError::new(
            "no_stewards_dir",
            format!("no .stewards/repo.toml in {}", repo.root.display()),
            "stewards init",
        )
    })?;

    let docs_path = Path::new(&loaded.repo.docs_path);
    let depth = docs_path
        .components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .count();
    // Depth 0 (`""`, `.`) would overwrite the repo's own README.md.
    if depth == 0
        || !docs_path
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err(CliError::new(
            "invalid_toml",
            format!(
                ".stewards/repo.toml: docs_path `{}` must be a directory inside the repo",
                loaded.repo.docs_path
            ),
            "set docs_path to a relative subdirectory without `..`, e.g. docs/units",
        )
        .into());
    }
    let up = "../".repeat(depth);

    let chosen: Vec<&UnitDef> = match unit {
        None => loaded.units.iter().collect(),
        Some(name) => match loaded.units.iter().find(|u| u.name == name) {
            Some(u) => vec![u],
            None => {
                let names: Vec<_> = loaded.units.iter().map(|u| u.name.as_str()).collect();
                return Err(CliError::new(
                    "unknown_unit",
                    format!("no unit named `{name}`"),
                    format!("one of: {}", names.join(", ")),
                )
                .into());
            }
        },
    };

    let mut pages: Vec<(String, String)> = chosen
        .iter()
        .map(|u| (format!("{}.md", u.name), unit_page(loaded, u, &up)))
        .collect();
    pages.push(("README.md".into(), index_page(loaded)));

    let mut wrote = Vec::new();
    for (name, text) in pages {
        let path = loaded.root.join(docs_path).join(&name);
        if std::fs::read_to_string(&path).is_ok_and(|old| old == text) {
            continue;
        }
        std::fs::create_dir_all(path.parent().expect("a page has a parent"))?;
        std::fs::write(&path, &text)?;
        wrote.push(
            docs_path
                .join(&name)
                .components()
                .filter(|c| matches!(c, Component::Normal(_)))
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/"),
        );
    }
    emit(&DocsReport { wrote }, g.human);
    Ok(ExitCode::SUCCESS)
}

/// Card and repo.toml text outside fenced blocks: one line, every ASCII punctuation
/// character backslash-escaped (CommonMark), so it renders as plain text.
fn esc(s: &str) -> String {
    let mut out = String::new();
    for c in s.split_whitespace().collect::<Vec<_>>().join(" ").chars() {
        if c.is_ascii_punctuation() {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The documenter's own markdown; only raw HTML and entities are neutralised.
fn prose_html_safe(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;")
}

/// Percent-encodes a path for a markdown link destination.
fn link_path(path: &str) -> String {
    let mut out = String::new();
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || b"/._-~".contains(&b) {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

fn table(out: &mut String, head: &[&str], rows: &[Vec<String>]) {
    if rows.is_empty() {
        out.push_str("_None._\n\n");
        return;
    }
    let _ = writeln!(out, "| {} |", head.join(" | "));
    let _ = writeln!(out, "|{}", "---|".repeat(head.len()));
    for row in rows {
        let _ = writeln!(out, "| {} |", row.join(" | "));
    }
    out.push('\n');
}

fn prose(text: Option<&str>) -> String {
    match text.map(str::trim) {
        Some(t) if !t.is_empty() => prose_html_safe(t),
        _ => NOT_WRITTEN.into(),
    }
}

fn anchor_cell(loaded: &Loaded, text: &str, up: &str) -> String {
    let plain = esc(text);
    let Ok(anchor) = Anchor::parse(text) else {
        return plain;
    };
    match resolve(&loaded.root, &anchor) {
        Resolution::Found { line } => {
            format!("[{plain}]({up}{}#L{line})", link_path(&anchor.path))
        }
        Resolution::DeadPath | Resolution::DeadName => plain,
    }
}

fn unit_page(loaded: &Loaded, unit: &UnitDef, up: &str) -> String {
    let name = &unit.name;
    let card = loaded.cards.get(name);
    let doc = card.and_then(|c| c.doc.as_ref());
    let mut out = format!(
        "> Generated by stewards from `.stewards/units/{name}.toml`; change prose through the documenter.\n\n"
    );
    if unit.purpose.is_empty() {
        let _ = writeln!(out, "# {}\n", esc(name));
    } else {
        let _ = writeln!(out, "# {}: {}\n", esc(name), esc(&unit.purpose));
    }
    for (heading, text) in [
        ("Overview", doc.map(|d| d.overview.as_str())),
        ("How it works", doc.map(|d| d.how_it_works.as_str())),
        ("How to change it", doc.map(|d| d.how_to_change.as_str())),
    ] {
        let _ = writeln!(out, "## {heading}\n\n{}\n", prose(text));
    }

    out.push_str("## Interface\n\n");
    let rows: Vec<Vec<String>> = card
        .map(|c| &c.interface[..])
        .unwrap_or_default()
        .iter()
        .map(|i| {
            vec![
                esc(&i.id),
                esc(&i.contract),
                anchor_cell(loaded, &i.anchor, up),
            ]
        })
        .collect();
    table(&mut out, &["id", "contract", "anchor"], &rows);

    out.push_str("## Scenarios\n\n");
    let rows: Vec<Vec<String>> = loaded
        .tests
        .get(name)
        .map(|t| &t.scenarios[..])
        .unwrap_or_default()
        .iter()
        .map(|s| {
            [&s.id, &s.interface, &s.given, &s.expect, &s.level]
                .map(|f| esc(f))
                .to_vec()
        })
        .collect();
    table(
        &mut out,
        &["id", "interface", "given", "expect", "level"],
        &rows,
    );

    out.push_str("## Flows\n\n");
    let flows: Vec<_> = loaded
        .repo
        .flows
        .iter()
        .filter(|f| f.units.contains(name))
        .collect();
    if flows.is_empty() {
        out.push_str("_None._\n");
    }
    for f in flows {
        let units: Vec<String> = f.units.iter().map(|n| esc(n)).collect();
        let _ = writeln!(out, "- {}: {}", esc(&f.name), units.join(" → "));
    }

    out.push_str("\n## Gaps\n\n");
    let rows: Vec<Vec<String>> = card
        .map(|c| &c.gaps[..])
        .unwrap_or_default()
        .iter()
        .map(|g| {
            let kind = match (g.kind, &g.severity) {
                (GapKind::Bug, Some(sev)) => format!("bug ({})", esc(sev)),
                (GapKind::Bug, None) => "bug".into(),
                (GapKind::Gap, _) => "gap".into(),
                (GapKind::Future, _) => "future".into(),
            };
            vec![kind, esc(&g.title), esc(&g.note)]
        })
        .collect();
    table(&mut out, &["kind", "title", "note"], &rows);

    out.push_str("## Last reviewed\n\n");
    match card.and_then(|c| c.reviewed.as_ref()) {
        Some(r) => {
            let _ = writeln!(out, "{}", esc(&r.commit));
        }
        None => out.push_str("never reviewed\n"),
    }
    out
}

fn index_page(loaded: &Loaded) -> String {
    let mut out = format!(
        "> Generated by stewards from `.stewards/`; change prose through the documenter.\n\n# {}\n\n## Units\n\n",
        esc(&loaded.repo.name)
    );
    for u in &loaded.units {
        if u.purpose.is_empty() {
            let _ = writeln!(out, "- [{}]({}.md)", esc(&u.name), u.name);
        } else {
            let _ = writeln!(
                out,
                "- [{}]({}.md): {}",
                esc(&u.name),
                u.name,
                esc(&u.purpose)
            );
        }
    }
    out.push_str("\n## Flows\n\n");
    if loaded.repo.flows.is_empty() {
        out.push_str("_None._\n");
    }
    for f in &loaded.repo.flows {
        let chain: Vec<String> = f
            .units
            .iter()
            .map(|n| {
                if loaded.units.iter().any(|u| &u.name == n) {
                    format!("[{}]({n}.md)", esc(n))
                } else {
                    esc(n)
                }
            })
            .collect();
        let _ = writeln!(out, "- {}: {}", esc(&f.name), chain.join(" → "));
    }
    out.push_str("\n## Decisions\n\n");
    if loaded.repo.decisions.is_empty() {
        out.push_str("_None._\n");
    }
    for d in &loaded.repo.decisions {
        let _ = writeln!(out, "- {}: {}", esc(&d.title), esc(&d.why));
    }
    out
}

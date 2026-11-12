use std::collections::BTreeMap;
use std::process::ExitCode;

use anyhow::Result;
use serde::Serialize;

use crate::git;
use crate::output::{CliError, Global, Human, emit};
use crate::route::Router;
use crate::workspace::{Repo, discover};

#[derive(Debug, Clone, Serialize)]
pub struct DebtItem {
    pub path: String,
    pub line: u32,
    pub text: String,
}

const MARKERS: [&str; 7] = ["#", "//", "--", ";", "/*", "*", "<!--"];

/// True when a comment marker appears before `simplified:` (any case) on the line.
pub fn is_debt_line(line: &str) -> bool {
    // ASCII lowercasing keeps byte offsets aligned with `line`.
    let lower = line.to_ascii_lowercase();
    // The first `simplified:` has the most room before it, so it decides.
    lower
        .find("simplified:")
        .is_some_and(|at| MARKERS.iter().any(|m| lower[..at].contains(m)))
}

/// Debt lines of the tracked files, keyed by owning unit (None = no unit owns the file).
pub fn scan(repo: &Repo) -> Result<BTreeMap<Option<String>, Vec<DebtItem>>> {
    let loaded = repo.loaded.as_ref().ok_or_else(|| {
        CliError::new(
            "no_stewards_dir",
            format!("no .stewards/ in {}", repo.root.display()),
            "stewards init",
        )
    })?;
    let router = Router::new(&loaded.units, &loaded.repo.docs_path)?;
    let mut found: BTreeMap<Option<String>, Vec<DebtItem>> = BTreeMap::new();
    for path in git::ls_files(&repo.root)? {
        if path.starts_with(".stewards/") {
            continue;
        }
        let bytes = match std::fs::read(repo.root.join(&path)) {
            Ok(b) => b,
            // tracked but deleted from the working tree
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(anyhow::Error::new(e).context(path)),
        };
        if bytes[..bytes.len().min(8192)].contains(&0) {
            continue;
        }
        let items: Vec<DebtItem> = String::from_utf8_lossy(&bytes)
            .lines()
            .enumerate()
            .filter(|(_, l)| is_debt_line(l))
            .map(|(i, l)| DebtItem {
                path: path.clone(),
                line: i as u32 + 1,
                text: l.trim().to_string(),
            })
            .collect();
        if !items.is_empty() {
            found
                .entry(router.owner(&path).map(String::from))
                .or_default()
                .extend(items);
        }
    }
    Ok(found)
}

#[derive(Serialize)]
struct UnitDebt {
    unit: Option<String>,
    items: Vec<DebtItem>,
}

#[derive(Serialize)]
struct Debt {
    units: Vec<UnitDebt>,
}

impl Human for Debt {
    fn render(&self) -> String {
        if self.units.is_empty() {
            return "No `simplified:` debt.".into();
        }
        self.units
            .iter()
            .map(|u| {
                let items: String = u
                    .items
                    .iter()
                    .map(|i| format!("- {}:{}: {}\n", i.path, i.line, i.text))
                    .collect();
                format!("## {}\n\n{items}", u.unit.as_deref().unwrap_or("(no unit)"))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

pub fn run(g: &Global) -> Result<ExitCode> {
    let ws = discover(&std::env::current_dir()?.canonicalize()?, g.repo.as_deref())?;
    let mut units = Vec::new();
    for repo in ws.repos.iter().filter(|r| r.loaded.is_some()) {
        for (unit, items) in scan(repo)? {
            units.push(UnitDebt {
                unit: unit.map(|u| ws.display_name(repo, &u)),
                items,
            });
        }
    }
    emit(&Debt { units }, g.human);
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::is_debt_line;

    #[test]
    fn each_marker_counts() {
        for line in [
            "# simplified: x",
            "x = 1  // simplified: x",
            "-- simplified: x",
            "; simplified: x",
            "/* simplified: x */",
            " * simplified: x",
            "<!-- simplified: x -->",
        ] {
            assert!(is_debt_line(line), "{line}");
        }
    }

    #[test]
    fn no_marker_before_it_does_not_count() {
        assert!(!is_debt_line("simplified: x"));
        assert!(!is_debt_line("simplified: x # later"));
        assert!(!is_debt_line("# nothing here"));
    }

    #[test]
    fn match_ignores_case() {
        assert!(is_debt_line("# SIMPLIFIED: x"));
    }
}

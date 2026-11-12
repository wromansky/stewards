// The `status` command: units, freshness, coverage, gaps, and debt across the workspace.
use std::process::ExitCode;

use anyhow::Result;
use serde::Serialize;

use crate::check::{fingerprint, one_line};
use crate::debt;
use crate::git;
use crate::model::GapKind;
use crate::output::{Global, Human, emit};
use crate::route::Router;
use crate::workspace::{Workspace, here};

#[derive(Debug, Clone, Serialize)]
pub struct UnitStatus {
    pub unit: String,
    pub purpose: String,
    pub paths: Vec<String>,
    /// `fresh`, `stale`, or `unreviewed`.
    pub freshness: &'static str,
    pub interfaces: usize,
    /// Interfaces that at least one scenario covers.
    pub covered: usize,
    /// Open gaps, `future` items included; bugs are counted apart.
    pub gaps: usize,
    pub bugs: usize,
    pub debt: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusReport {
    pub units: Vec<UnitStatus>,
    pub uninitialized: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telemetry: Option<serde_json::Value>,
}

pub fn status(ws: &Workspace) -> Result<StatusReport> {
    let mut report = StatusReport {
        units: vec![],
        uninitialized: vec![],
        telemetry: None,
    };
    for repo in &ws.repos {
        let Some(loaded) = &repo.loaded else {
            report.uninitialized.push(repo.name.clone());
            continue;
        };
        let router = Router::new(&loaded.units, &loaded.repo.docs_path)?;
        let tree = git::ls_tree(&repo.root, &git::snapshot(&repo.root)?)?;
        let debt = debt::scan(repo)?;
        for def in &loaded.units {
            let card = loaded.cards.get(&def.name);
            let freshness = match card.and_then(|c| c.reviewed.as_ref()) {
                None => "unreviewed",
                Some(r) if r.fingerprint == fingerprint(&tree, &def.name, &router) => "fresh",
                Some(_) => "stale",
            };
            let interfaces = card.map_or(&[][..], |c| &c.interface);
            let scenarios = loaded
                .tests
                .get(&def.name)
                .map_or(&[][..], |t| &t.scenarios);
            let bugs = card.map_or(0, |c| {
                c.gaps.iter().filter(|g| g.kind == GapKind::Bug).count()
            });
            report.units.push(UnitStatus {
                unit: ws.display_name(repo, &def.name),
                purpose: def.purpose.clone(),
                paths: def.paths.clone(),
                freshness,
                interfaces: interfaces.len(),
                covered: interfaces
                    .iter()
                    .filter(|i| scenarios.iter().any(|s| s.interface == i.id))
                    .count(),
                gaps: card.map_or(0, |c| c.gaps.len()) - bugs,
                bugs,
                debt: debt.get(&Some(def.name.clone())).map_or(0, Vec::len),
            });
        }
    }
    Ok(report)
}

impl Human for StatusReport {
    fn render(&self) -> String {
        let mut out = vec![
            "| unit | freshness | covered | gaps | bugs | debt | purpose |".to_string(),
            "|---|---|---|---|---|---|---|".to_string(),
        ];
        out.extend(self.units.iter().map(|u| {
            format!(
                "| {} | {} | {}/{} | {} | {} | {} | {} |",
                u.unit,
                u.freshness,
                u.covered,
                u.interfaces,
                u.gaps,
                u.bugs,
                u.debt,
                one_line(&u.purpose).replace('|', "\\|")
            )
        }));
        if !self.uninitialized.is_empty() {
            out.push(String::new());
            out.push(format!(
                "Uninitialized (run `stewards init`): {}",
                self.uninitialized.join(", ")
            ));
        }
        if let Some(t) = self.telemetry.as_ref().filter(|t| !t.is_null()) {
            out.push(String::new());
            out.push("Telemetry:".into());
            out.extend(crate::telemetry::human(t));
        }
        out.join("\n")
    }
}

pub fn run(g: &Global, telemetry: bool) -> Result<ExitCode> {
    let (cwd, ws) = here(g)?;
    let mut report = status(&ws)?;
    if telemetry {
        // Telemetry is per repo; with no current repo (a workspace root) it is null.
        let repo = ws.current_repo(&cwd).ok();
        report.telemetry = Some(repo.map_or(serde_json::Value::Null, |r| {
            crate::telemetry::summary(&r.root)
        }));
    }
    emit(&report, g.human);
    Ok(ExitCode::SUCCESS)
}

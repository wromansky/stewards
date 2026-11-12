// Fingerprints, stamps, and the `check` and `stamp` commands.
use std::path::Path;
use std::process::ExitCode;

use anyhow::Result;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::anchors::{Resolution, resolve};
use crate::git;
use crate::model::{Anchor, Reviewed, UnitCard};
use crate::output::{Global, Human, emit};
use crate::route::Router;
use crate::workspace::{Repo, Workspace, here};

/// Unowned files named in the one `unowned` finding.
const UNOWNED_LISTED: usize = 10;

/// Card or `--task` text printed on one line: control characters, newlines included, and the
/// Unicode line and paragraph separators become spaces.
pub fn one_line(s: &str) -> String {
    s.replace(
        |c: char| c.is_control() || c == '\u{2028}' || c == '\u{2029}',
        " ",
    )
}

/// True when the unit has no `[reviewed]`, or its fingerprint differs from `current`.
pub fn is_stale(reviewed: Option<&Reviewed>, current: &str) -> bool {
    reviewed.is_none_or(|r| r.fingerprint != current)
}

/// `sha256:<hex>` over the sorted `path blob` lines of the files `unit` owns in `tree`.
pub fn fingerprint(tree: &[(String, String)], unit: &str, router: &Router) -> String {
    let mut lines: Vec<String> = tree
        .iter()
        .filter(|(path, _)| router.owner(path) == Some(unit))
        .map(|(path, blob)| format!("{path} {blob}\n"))
        .collect();
    lines.sort();
    let digest = Sha256::digest(lines.concat().as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
}

/// Writes `[reviewed]` for `unit` at snapshot `tree` into its card, keeping the role marks and the rest.
pub fn stamp(repo: &Repo, unit: &str, tree: &str) -> Result<Reviewed> {
    let loaded = repo.loaded()?;
    loaded.unit(unit)?;
    let router = Router::new(&loaded.units, &loaded.repo.docs_path)?;
    let entries = git::ls_tree(&repo.root, tree)?;
    let mut card = loaded.cards.get(unit).cloned().unwrap_or_else(|| UnitCard {
        unit: unit.to_string(),
        reviewed: None,
        interface: vec![],
        anchors: vec![],
        gaps: vec![],
        doc: None,
    });
    let marks = card.reviewed.take();
    let reviewed = Reviewed {
        // no commits yet: the commit is display-only, so empty is fine
        commit: git::head_commit(&repo.root)?.unwrap_or_default(),
        fingerprint: fingerprint(&entries, unit, &router),
        tree: tree.to_string(),
        tester: marks.as_ref().and_then(|r| r.tester.clone()),
        documenter: marks.and_then(|r| r.documenter),
    };
    card.reviewed = Some(reviewed.clone());
    repo.write(
        &format!(".stewards/units/{unit}.toml"),
        toml::to_string(&card)?.as_bytes(),
    )?;
    Ok(reviewed)
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub kind: &'static str,
    pub unit: Option<String>,
    pub path: Option<String>,
    pub anchor: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct CheckReport {
    pub failures: Vec<Finding>,
    pub warnings: Vec<Finding>,
}

/// Files changed since `tree`; every file `unit` owns when there is no tree or git has pruned it.
pub fn changed_since(
    root: &Path,
    snapshot: &str,
    entries: &[(String, String)],
    router: &Router,
    unit: &str,
    tree: Option<&str>,
) -> Result<Vec<String>> {
    match tree.filter(|t| git::has_object(root, t)) {
        Some(t) => git::diff_names(root, t, snapshot),
        None => Ok(entries
            .iter()
            .map(|(p, _)| p.clone())
            .filter(|p| router.owner(p) == Some(unit))
            .collect()),
    }
}

/// The files `unit` owns that changed since its stamp; `None` when the unit is fresh.
pub fn stale_files(
    root: &Path,
    snapshot: &str,
    entries: &[(String, String)],
    router: &Router,
    unit: &str,
    reviewed: Option<&Reviewed>,
) -> Result<Option<Vec<String>>> {
    if !is_stale(reviewed, &fingerprint(entries, unit, router)) {
        return Ok(None);
    }
    let changed = changed_since(
        root,
        snapshot,
        entries,
        router,
        unit,
        reviewed.map(|r| r.tree.as_str()),
    )?;
    Ok(Some(
        changed
            .into_iter()
            .filter(|p| router.owner(p) == Some(unit))
            .collect(),
    ))
}

/// Stamps, anchors, and scenario ids across the workspace; `filter` keeps one unit's findings
/// (by plain or display name).
pub fn check(ws: &Workspace, filter: Option<&str>) -> Result<CheckReport> {
    let snapshots = ws
        .loaded_repos()?
        .iter()
        .map(|(repo, _)| git::snapshot(&repo.root))
        .collect::<Result<Vec<_>>>()?;
    check_at(ws, filter, &snapshots)
}

/// `check` against snapshot trees the caller already took, one per `ws.loaded_repos()` in order.
pub fn check_at(ws: &Workspace, filter: Option<&str>, snapshots: &[String]) -> Result<CheckReport> {
    let repos = ws.loaded_repos()?;
    anyhow::ensure!(
        repos.len() == snapshots.len(),
        "check_at: {} snapshots for {} repos",
        snapshots.len(),
        repos.len()
    );
    let mut report = CheckReport::default();
    for ((repo, loaded), snapshot) in repos.into_iter().zip(snapshots) {
        let root = &repo.root;
        let router = Router::new(&loaded.units, &loaded.repo.docs_path)?;
        let entries = git::ls_tree(root, snapshot)?;
        let tracked = git::ls_files(root)?;

        for def in &loaded.units {
            let name = ws.display_name(repo, &def.name);
            if filter.is_some_and(|f| f != def.name && f != name) {
                continue;
            }
            let finding =
                |kind, path: Option<&str>, anchor: Option<&str>, message: String| Finding {
                    kind,
                    unit: Some(name.clone()),
                    path: path.map(String::from),
                    anchor: anchor.map(String::from),
                    message,
                };
            let card = loaded.cards.get(&def.name);
            let reviewed = card.and_then(|c| c.reviewed.as_ref());
            if is_stale(reviewed, &fingerprint(&entries, &def.name, &router)) {
                report.failures.push(match reviewed {
                    None => finding(
                        "unreviewed",
                        None,
                        None,
                        format!("{name} has never been reviewed"),
                    ),
                    Some(_) => finding(
                        "stale",
                        None,
                        None,
                        format!("{name} changed since its stamp"),
                    ),
                });
            }

            let interfaces = card.map_or(&[][..], |c| &c.interface);
            let anchors = card.map_or(&[][..], |c| &c.anchors);
            // Only an interface's anchor is text; a card `[[anchor]]` is already parsed.
            let targets = interfaces
                .iter()
                .map(|i| (i.anchor.clone(), Anchor::parse(&i.anchor).ok()))
                .chain(
                    anchors
                        .iter()
                        .map(|a| (format!("{}::{}", a.path, a.name), Some(a.clone()))),
                );
            for (text, anchor) in targets {
                let dead =
                    anchor.is_none_or(|a| !matches!(resolve(root, &a), Resolution::Found { .. }));
                if dead {
                    report.failures.push(finding(
                        "dead_anchor",
                        None,
                        Some(&text),
                        format!("{name}: anchor `{text}` does not resolve"),
                    ));
                }
            }

            let scenarios = loaded
                .tests
                .get(&def.name)
                .map_or(&[][..], |t| &t.scenarios);
            for s in scenarios {
                if !interfaces.iter().any(|i| i.id == s.interface) {
                    report.failures.push(finding(
                        "unknown_interface",
                        None,
                        None,
                        format!(
                            "{name}: scenario `{}` references unknown interface `{}`",
                            s.id, s.interface
                        ),
                    ));
                }
            }
            for i in interfaces {
                if !scenarios.iter().any(|s| s.interface == i.id) {
                    report.warnings.push(finding(
                        "uncovered_interface",
                        None,
                        Some(&i.anchor),
                        format!("{name}: no scenario covers interface `{}`", i.id),
                    ));
                }
            }
        }

        // Overlaps and unowned files belong to no single unit, so a filter drops them.
        if filter.is_some() {
            continue;
        }
        for (path, units) in router.overlaps(&tracked) {
            let units: Vec<_> = units.iter().map(|u| ws.display_name(repo, u)).collect();
            report.warnings.push(Finding {
                kind: "overlap",
                unit: None,
                message: format!("{path} is matched equally by {}", units.join(", ")),
                path: Some(path),
                anchor: None,
            });
        }
        let mut unowned: Vec<_> = tracked
            .iter()
            .filter(|p| router.owner(p).is_none() && !router.tool_owned(p))
            .collect();
        unowned.sort();
        if !unowned.is_empty() {
            let more = match unowned.len().saturating_sub(UNOWNED_LISTED) {
                0 => String::new(),
                n => format!(" (+{n} more)"),
            };
            let listed: Vec<_> = unowned
                .iter()
                .take(UNOWNED_LISTED)
                .map(|p| p.as_str())
                .collect();
            report.warnings.push(Finding {
                kind: "unowned",
                unit: None,
                path: None,
                anchor: None,
                message: format!(
                    "{} tracked file(s) have no owner: {}{more}",
                    unowned.len(),
                    listed.join(", ")
                ),
            });
        }
    }
    Ok(report)
}

impl Human for CheckReport {
    fn render(&self) -> String {
        if self.failures.is_empty() && self.warnings.is_empty() {
            return "check: ok".into();
        }
        let mut out = Vec::new();
        for (title, list) in [("Failures", &self.failures), ("Warnings", &self.warnings)] {
            if !list.is_empty() {
                out.push(format!("## {title}\n"));
                out.extend(
                    list.iter()
                        .map(|f| format!("- `{}` {}", f.kind, one_line(&f.message))),
                );
                out.push(String::new());
            }
        }
        out.join("\n").trim_end().to_string()
    }
}

pub fn run(g: &Global) -> Result<ExitCode> {
    let (_, ws) = here(g)?;
    let report = check(&ws, None)?;
    emit(&report, g.human);
    Ok(if report.failures.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

#[derive(Serialize)]
struct Stamped {
    unit: String,
    reviewed: Reviewed,
}

impl Human for Stamped {
    fn render(&self) -> String {
        format!(
            "stamped {} at tree {} (commit {})",
            self.unit, self.reviewed.tree, self.reviewed.commit
        )
    }
}

pub fn run_stamp(g: &Global, unit: &str) -> Result<ExitCode> {
    let (cwd, ws) = here(g)?;
    let repo = ws.current_repo(&cwd)?;
    let reviewed = stamp(repo, unit, &git::snapshot(&repo.root)?)?;
    emit(
        &Stamped {
            unit: unit.to_string(),
            reviewed,
        },
        g.human,
    );
    Ok(ExitCode::SUCCESS)
}

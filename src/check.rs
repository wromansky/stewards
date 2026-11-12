// Fingerprints, stamps, and the `check` and `stamp` commands.
use std::process::ExitCode;

use anyhow::{Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::anchors::{Resolution, resolve};
use crate::git;
use crate::model::{Anchor, Loaded, Reviewed, UnitCard, UnitDef};
use crate::output::{CliError, Global, Human, emit};
use crate::route::Router;
use crate::workspace::{Repo, Workspace, discover};

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

fn require_loaded(repo: &Repo) -> Result<&Loaded> {
    Ok(repo.loaded.as_ref().ok_or_else(|| {
        CliError::new(
            "no_stewards_dir",
            format!("no .stewards/ in {}", repo.root.display()),
            "stewards init",
        )
    })?)
}

/// Writes `[reviewed]` for `unit` at snapshot `tree` into its card, keeping the role marks and the rest.
pub fn stamp(repo: &Repo, unit: &str, tree: &str) -> Result<Reviewed> {
    let loaded = require_loaded(repo)?;
    if !loaded.units.iter().any(|u| u.name == unit) {
        let names: Vec<_> = loaded.units.iter().map(|u| u.name.as_str()).collect();
        anyhow::bail!(CliError::new(
            "unknown_unit",
            format!("no unit named `{unit}`; units: {}", names.join(", ")),
            "one of the listed units",
        ));
    }
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
        commit: git::head(&repo.root).unwrap_or_default(),
        fingerprint: fingerprint(&entries, unit, &router),
        tree: tree.to_string(),
        tester: marks.as_ref().and_then(|r| r.tester.clone()),
        documenter: marks.and_then(|r| r.documenter),
    };
    card.reviewed = Some(reviewed.clone());
    let path = repo.root.join(format!(".stewards/units/{unit}.toml"));
    std::fs::create_dir_all(path.parent().expect("a file has a parent"))?;
    std::fs::write(&path, toml::to_string(&card)?)
        .with_context(|| format!("write {}", path.display()))?;
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

/// Stamps, anchors, and scenario ids across the workspace; `filter` keeps one unit's findings
/// (by plain or display name).
pub fn check(ws: &Workspace, filter: Option<&str>) -> Result<CheckReport> {
    let mut report = CheckReport::default();
    for repo in &ws.repos {
        let loaded = match &repo.loaded {
            None if ws.is_workspace => continue,
            _ => require_loaded(repo)?,
        };
        let root = &repo.root;
        let router = Router::new(&loaded.units, &loaded.repo.docs_path)?;
        let entries = git::ls_tree(root, &git::snapshot(root)?)?;
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
            match card.and_then(|c| c.reviewed.as_ref()) {
                None => report.failures.push(finding(
                    "unreviewed",
                    None,
                    None,
                    format!("{name} has never been reviewed"),
                )),
                Some(r) if r.fingerprint != fingerprint(&entries, &def.name, &router) => {
                    report.failures.push(finding(
                        "stale",
                        None,
                        None,
                        format!("{name} changed since its stamp"),
                    ))
                }
                Some(_) => {}
            }

            let interfaces = card.map_or(&[][..], |c| &c.interface);
            let anchors = card.map_or(&[][..], |c| &c.anchors);
            let texts = interfaces
                .iter()
                .map(|i| i.anchor.clone())
                .chain(anchors.iter().map(|a| format!("{}::{}", a.path, a.name)));
            for text in texts {
                let dead = match Anchor::parse(&text) {
                    Ok(a) => !matches!(resolve(root, &a), Resolution::Found { .. }),
                    Err(_) => true,
                };
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
        // A catch-all unit owns every path except the tool-owned ones, which are never unowned.
        let all = UnitDef {
            name: "all".into(),
            purpose: String::new(),
            paths: vec!["**".into()],
        };
        let routable = Router::new(&[all], &loaded.repo.docs_path)?;
        for path in tracked
            .iter()
            .filter(|p| router.owner(p).is_none() && routable.owner(p).is_some())
        {
            report.warnings.push(Finding {
                kind: "unowned",
                unit: None,
                path: Some(path.clone()),
                anchor: None,
                message: format!("no unit owns {path}"),
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
                out.extend(list.iter().map(|f| format!("- `{}` {}", f.kind, f.message)));
                out.push(String::new());
            }
        }
        out.join("\n").trim_end().to_string()
    }
}

pub fn run(g: &Global) -> Result<ExitCode> {
    let ws = discover(&std::env::current_dir()?.canonicalize()?, g.repo.as_deref())?;
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
    let cwd = std::env::current_dir()?.canonicalize()?;
    let ws = discover(&cwd, g.repo.as_deref())?;
    // With `--repo` the one repo may sit beside the current directory rather than around it.
    let repo = ws
        .repos
        .iter()
        .find(|r| {
            r.root
                .canonicalize()
                .is_ok_and(|root| cwd.starts_with(root))
        })
        .or(if ws.repos.len() == 1 {
            ws.repos.first()
        } else {
            None
        })
        .ok_or_else(|| {
            CliError::new(
                "not_in_repo",
                "the current directory is not inside a repo",
                "run inside a repo or pass --repo <name>",
            )
        })?;
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

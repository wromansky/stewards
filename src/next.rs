// The `next` command: which review jobs are due before reporting done.
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use serde::Serialize;

use crate::brief;
use crate::check::{CheckReport, changed_since, check_at, one_line, stale_files};
use crate::debt::{DebtItem, is_debt_line};
use crate::git;
use crate::llm::Mode;
use crate::model::{Anchor, GapKind, Role};
use crate::output::{Global, Human, emit, not_implemented};
use crate::route::Router;
use crate::workspace::{Workspace, here};

#[derive(Debug, Clone, Serialize)]
pub struct Job {
    /// Empty for the architect.
    pub unit: String,
    pub role: Role,
    pub task: String,
    /// The snapshot tree of the job's repo that `changed` was computed against.
    pub snapshot: String,
    pub reasons: Vec<String>,
    pub changed: Vec<String>,
    pub brief_path: Option<PathBuf>,
    pub command: String,
    pub debt_added: Vec<DebtItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FlowCheck {
    pub flow: String,
    pub command: String,
    pub pass: bool,
    pub output: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct NextReport {
    pub jobs: Vec<Job>,
    pub architect: bool,
    pub flow_checks: Vec<FlowCheck>,
    pub check: CheckReport,
}

/// With `write_briefs`, writes each job's brief and fills its `brief_path`.
pub fn next(ws: &Workspace, task: Option<&str>, write_briefs: bool) -> Result<NextReport> {
    let mut jobs = Vec::new();
    // Stamped units changed since their stamps: (name, repo name, task, snapshot, changed files).
    let mut moved: Vec<(String, String, String, String, Vec<String>)> = Vec::new();
    let mut snapshots = Vec::new();
    for (repo, loaded) in ws.loaded_repos()? {
        let root = &repo.root;
        let router = Router::new(&loaded.units, &loaded.repo.docs_path)?;
        let snapshot = git::snapshot(root)?;
        snapshots.push(snapshot.clone());
        let entries = git::ls_tree(root, &snapshot)?;
        let task = match task {
            Some(t) => t.to_string(),
            None => git::task_text(root)?,
        };
        let under_test_root = |path: &str| {
            loaded.repo.test_roots.iter().any(|r| {
                let r = r.trim_end_matches('/');
                path.strip_prefix(r)
                    .is_some_and(|rest| rest.starts_with('/'))
            })
        };
        for def in &loaded.units {
            let first_job = jobs.len();
            let name = ws.display_name(repo, &def.name);
            let owns = |path: &str| router.owner(path) == Some(def.name.as_str());
            let since = |tree: Option<&String>| {
                changed_since(
                    root,
                    &snapshot,
                    &entries,
                    &router,
                    &def.name,
                    tree.map(String::as_str),
                )
            };
            let job = |role: Role, reasons: Vec<String>, changed: Vec<String>| Job {
                unit: name.clone(),
                role,
                task: task.clone(),
                snapshot: snapshot.clone(),
                reasons,
                changed,
                brief_path: None,
                command: if ws.is_workspace {
                    format!(
                        "stewards --repo {} brief {} --role {}",
                        repo.name,
                        def.name,
                        role.as_str()
                    )
                } else {
                    format!("stewards brief {name} --role {}", role.as_str())
                },
                debt_added: vec![],
            };
            let card = loaded.cards.get(&def.name);
            let reviewed = card.and_then(|c| c.reviewed.as_ref());
            let stamp = reviewed.map(|r| &r.tree);

            if let Some(changed) =
                stale_files(root, &snapshot, &entries, &router, &def.name, reviewed)?
            {
                let debt_added = match stamp.filter(|t| git::has_object(root, t)) {
                    Some(t) => git::diff_added_lines(root, t, &snapshot)?
                        .into_iter()
                        .filter(|(p, _, text)| owns(p) && is_debt_line(text))
                        .map(|(path, line, text)| DebtItem {
                            path,
                            line,
                            text: text.trim().to_string(),
                        })
                        .collect(),
                    None => vec![],
                };
                let reason = if reviewed.is_some() {
                    "stale"
                } else {
                    "never reviewed"
                };
                if reviewed.is_some() {
                    // paths of several repos share the architect's list, so name the repo
                    let changed = changed
                        .iter()
                        .map(|p| {
                            if ws.is_workspace {
                                format!("{}/{p}", repo.name)
                            } else {
                                p.clone()
                            }
                        })
                        .collect();
                    moved.push((
                        name.clone(),
                        repo.name.clone(),
                        task.clone(),
                        snapshot.clone(),
                        changed,
                    ));
                }
                jobs.push(Job {
                    debt_added,
                    ..job(Role::Maintainer, vec![reason.into()], changed)
                });
            }

            let anchor_paths: Vec<String> = card
                .map_or(&[][..], |c| &c.interface)
                .iter()
                .filter_map(|i| Anchor::parse(&i.anchor).ok().map(|a| a.path))
                .collect();
            let tester_mark = reviewed.and_then(|r| r.tester.as_ref()).or(stamp);
            let (mut reasons, mut changed) = (vec![], vec![]);
            for p in since(tester_mark)? {
                if anchor_paths.contains(&p) {
                    reasons.push(format!("interface file changed: {p}"));
                } else if owns(&p) && under_test_root(&p) {
                    reasons.push(format!("test file changed: {p}"));
                } else {
                    continue;
                }
                changed.push(p);
            }
            if !changed.is_empty() {
                jobs.push(job(Role::Tester, reasons, changed));
            }

            let documenter_mark = reviewed.and_then(|r| r.documenter.as_ref()).or(stamp);
            let changed: Vec<String> = since(documenter_mark)?
                .into_iter()
                .filter(|p| anchor_paths.contains(p))
                .collect();
            if !changed.is_empty() {
                let reasons = changed
                    .iter()
                    .map(|p| format!("interface file changed: {p}"))
                    .collect();
                jobs.push(job(Role::Documenter, reasons, changed));
            }

            let bug = card.and_then(|c| {
                c.gaps
                    .iter()
                    .find(|g| g.kind == GapKind::Bug && g.severity.is_none())
            });
            if let Some(bug) = bug {
                let reasons = vec![format!("bug not judged: {}", bug.title)];
                jobs.push(job(Role::Validator, reasons, vec![]));
            }
            if write_briefs {
                for j in &mut jobs[first_job..] {
                    let unit = Some(def.name.as_str());
                    let (path, _) =
                        brief::write(repo, unit, j.role, Mode::Review, &j.task, &j.snapshot)?;
                    j.brief_path = Some(path);
                }
            }
        }
    }

    // No plans exist yet, so no plan ever covers the moved units.
    let architect = moved.len() >= 2;
    if architect {
        let names: Vec<&str> = moved.iter().map(|(n, ..)| n.as_str()).collect();
        let (_, repo_name, task, snapshot, _) = &moved[0];
        let brief_path = if write_briefs {
            let repo = ws.repos.iter().find(|r| &r.name == repo_name);
            let repo = repo.expect("moved names a loaded repo");
            Some(brief::write(repo, None, Role::Architect, Mode::Review, task, snapshot)?.0)
        } else {
            None
        };
        jobs.insert(
            0,
            Job {
                unit: String::new(),
                role: Role::Architect,
                task: task.clone(),
                snapshot: snapshot.clone(),
                reasons: vec![format!(
                    "units changed since their stamps: {}",
                    names.join(", ")
                )],
                changed: moved.iter().flat_map(|(.., c)| c.clone()).collect(),
                brief_path,
                command: if ws.is_workspace {
                    format!("stewards --repo {repo_name} brief --role architect")
                } else {
                    "stewards brief --role architect".into()
                },
                debt_added: vec![],
            },
        );
    }
    Ok(NextReport {
        jobs,
        architect,
        flow_checks: vec![],
        check: check_at(ws, None, &snapshots)?,
    })
}

impl Human for NextReport {
    fn render(&self) -> String {
        let mut out = vec!["## Jobs\n".to_string()];
        if self.jobs.is_empty() {
            out.push("No jobs.".into());
        }
        for j in &self.jobs {
            let who = format!("{} {}", j.unit, j.role.as_str());
            let reasons = one_line(&j.reasons.join("; "));
            out.push(format!("- `{}`: {reasons}", who.trim()));
            out.extend(j.debt_added.iter().map(|d| {
                let path = one_line(&d.path);
                format!("  - debt added {path}:{}: {}", d.line, one_line(&d.text))
            }));
        }
        out.push(String::new());
        out.push(self.check.render());
        out.join("\n")
    }
}

pub fn run(
    g: &Global,
    task: Option<&str>,
    no_brief: bool,
    _no_run: bool,
    pr: Option<u64>,
) -> Result<ExitCode> {
    if pr.is_some() {
        return not_implemented("next --pr");
    }
    let (_, ws) = here(g)?;
    let report = next(&ws, task, !no_brief)?;
    emit(&report, g.human);
    Ok(
        if report.jobs.is_empty() && report.check.failures.is_empty() {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(1)
        },
    )
}

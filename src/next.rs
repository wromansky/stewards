// The `next` command: which review jobs are due before reporting done.
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Result;
use serde::Serialize;

use crate::anchors;
use crate::brief;
use crate::check::{CheckReport, changed_since, check_at, one_line, stale_files};
use crate::debt::{DebtItem, is_debt_line};
use crate::git;
use crate::llm::Mode;
use crate::model::{Anchor, GapKind, Interface, Role};
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

            let interfaces = card.map_or(&[][..], |c| &c.interface);
            let touched = |mark: Option<&String>| {
                let changed = since(mark)?;
                let mark = mark.map(String::as_str);
                let touched =
                    interfaces_touched(root, &snapshot, &entries, interfaces, mark, &changed)?;
                anyhow::Ok((touched, changed))
            };
            let tester_mark = reviewed.and_then(|r| r.tester.as_ref()).or(stamp);
            let ((mut reasons, mut changed), since_mark) = touched(tester_mark)?;
            for p in since_mark {
                if owns(&p) && under_test_root(&p) && !changed.contains(&p) {
                    reasons.push(format!("test file changed: {p}"));
                    changed.push(p);
                }
            }
            // a never-stamped unit's first review comes first: a mark needs a `[reviewed]` block
            if reviewed.is_some() && !changed.is_empty() {
                jobs.push(job(Role::Tester, reasons, changed));
            }

            let documenter_mark = reviewed.and_then(|r| r.documenter.as_ref()).or(stamp);
            let ((mut reasons, changed), _) = touched(documenter_mark)?;
            // a card a first review drafted has an interface to document
            let no_docs = card.is_some_and(|c| {
                !c.interface.is_empty()
                    && c.doc.as_ref().is_none_or(|d| {
                        [&d.overview, &d.how_it_works, &d.how_to_change]
                            .iter()
                            .all(|s| s.trim().is_empty())
                    })
            });
            if no_docs {
                reasons.push("no docs yet".into());
            }
            if reviewed.is_some() && !reasons.is_empty() {
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

/// `(reasons, files)` for the interfaces a change since `mark` touched, in card order: a hunk
/// (`mark` -> `snapshot`) whose range or the line just before or after it, on the old side or
/// the new, is one of the interface's lines in that side's file. `changed` is the files changed
/// since `mark`. An interface whose lines can't be found (no mark tree, file gone or without
/// hunks, name absent, a blob unreadable) counts when its file changed.
fn interfaces_touched(
    root: &Path,
    snapshot: &str,
    entries: &[(String, String)],
    interfaces: &[Interface],
    mark: Option<&str>,
    changed: &[String],
) -> Result<(Vec<String>, Vec<String>)> {
    // an anchor that does not parse names no file to watch
    let watched: Vec<(&Interface, Anchor)> = interfaces
        .iter()
        .filter_map(|i| Some((i, Anchor::parse(&i.anchor).ok()?)))
        .filter(|(_, a)| changed.contains(&a.path))
        .collect();
    let (mut reasons, mut files) = (Vec::new(), Vec::new());
    if watched.is_empty() {
        return Ok((reasons, files));
    }
    let mark = mark.filter(|t| git::has_object(root, t));
    let hunks = match mark {
        Some(t) => git::diff_hunks(root, t, snapshot)?,
        None => vec![],
    };
    for (i, a) in watched {
        let file_hunks: Vec<_> = hunks.iter().filter(|(p, ..)| *p == a.path).collect();
        // (old, new) lines; `None` when they can't be found
        let sides = || {
            if file_hunks.is_empty() {
                return None;
            }
            let (_, sha) = entries.iter().find(|(p, _)| *p == a.path)?;
            let new = anchors::interface_lines(&git::blob_text(root, sha).ok()?, &a.name);
            let old = git::blob_text(root, &format!("{}:{}", mark?, a.path)).ok()?;
            let old = anchors::interface_lines(&old, &a.name);
            (!new.is_empty()).then_some((old, new))
        };
        // a side's range `(start, count)` plus the line just before and after it
        let hit = |lines: &[u32], (start, count): (u32, u32)| {
            let first = if count == 0 { start + 1 } else { start };
            (first.saturating_sub(1)..=first + count).any(|l| lines.binary_search(&l).is_ok())
        };
        let reason = match sides() {
            None => format!("interface file changed: {}", a.path),
            Some((old, new))
                if file_hunks
                    .iter()
                    .any(|(_, o, n)| hit(&old, *o) || hit(&new, *n)) =>
            {
                format!("interface changed: {}", i.anchor)
            }
            Some(_) => continue,
        };
        if !reasons.contains(&reason) {
            reasons.push(reason);
        }
        if !files.contains(&a.path) {
            files.push(a.path);
        }
    }
    Ok((reasons, files))
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

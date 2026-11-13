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
use crate::model::{Anchor, GapKind, Interface, Role, UnitCard};
use crate::output::{Global, Human, emit, not_implemented};
use crate::route::Router;
use crate::workspace::{Repo, Workspace, here};

#[derive(Debug, Clone, Serialize)]
pub struct Job {
    /// Empty for the architect and the reviewer.
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

/// A per-role job with its repo and its plain unit name (`None` for the architect).
struct RoleJob<'a> {
    repo: &'a Repo,
    unit: Option<String>,
    job: Job,
}

/// The jobs due. With `roles`, one job per unit and role as `next --roles` lists them; without,
/// each repo's stamped units those jobs name are folded into one reviewer job, and only a
/// never-stamped unit keeps its own (maintainer) job. With `write_briefs`, writes each job's brief
/// and fills its `brief_path`.
pub fn next(
    ws: &Workspace,
    task: Option<&str>,
    write_briefs: bool,
    roles: bool,
) -> Result<NextReport> {
    let (role_jobs, snapshots) = role_jobs(ws, task)?;
    let mut jobs = vec![];
    if roles {
        for RoleJob {
            repo,
            unit,
            mut job,
        } in role_jobs
        {
            if write_briefs {
                let (path, _) = brief::write(
                    repo,
                    unit.as_deref(),
                    job.role,
                    Mode::Review,
                    &job.task,
                    &job.snapshot,
                )?;
                job.brief_path = Some(path);
            }
            jobs.push(job);
        }
    } else {
        let mut first_reviews = vec![];
        for repo in &ws.repos {
            let of_repo: Vec<&RoleJob> = role_jobs
                .iter()
                .filter(|j| std::ptr::eq(j.repo, repo))
                .collect();
            let due = due_for_review(&of_repo);
            let Some(first) = of_repo.first() else {
                continue;
            };
            if !due.is_empty() {
                let (task, snapshot) = (&first.job.task, &first.job.snapshot);
                jobs.push(reviewer_job(ws, repo, &due, task, snapshot, write_briefs)?);
            }
            for j in of_repo {
                if j.job.role == Role::Maintainer && j.job.reasons == [NEVER_REVIEWED] {
                    let mut job = j.job.clone();
                    if write_briefs {
                        let unit = j.unit.as_deref();
                        let (path, _) = brief::write(
                            repo,
                            unit,
                            job.role,
                            Mode::Review,
                            &job.task,
                            &job.snapshot,
                        )?;
                        job.brief_path = Some(path);
                    }
                    first_reviews.push(job);
                }
            }
        }
        jobs.extend(first_reviews);
    }
    Ok(NextReport {
        architect: jobs.iter().any(|j| j.role == Role::Architect),
        jobs,
        flow_checks: vec![],
        check: check_at(ws, None, &snapshots)?,
    })
}

const NEVER_REVIEWED: &str = "never reviewed";

/// A unit the review brief covers and the reasons it is due.
pub type DueUnit = (String, Vec<String>);

/// A stamped unit the reviewer covers, with why: the reasons of its maintainer (stale), tester,
/// and documenter jobs.
struct Due {
    unit: String,
    reasons: Vec<String>,
    changed: Vec<String>,
    debt_added: Vec<DebtItem>,
}

/// The stamped units of one repo's per-role `jobs` that have a maintainer, tester, or documenter
/// job, in units.toml order.
fn due_for_review(jobs: &[&RoleJob]) -> Vec<Due> {
    let mut due: Vec<Due> = vec![];
    for j in jobs {
        let stamped_review = match j.job.role {
            Role::Maintainer => j.job.reasons != [NEVER_REVIEWED],
            Role::Tester | Role::Documenter => true,
            _ => false,
        };
        let Some(unit) = j.unit.as_ref().filter(|_| stamped_review) else {
            continue;
        };
        let at = match due.iter().position(|d| &d.unit == unit) {
            Some(at) => at,
            None => {
                due.push(Due {
                    unit: unit.clone(),
                    reasons: vec![],
                    changed: vec![],
                    debt_added: vec![],
                });
                due.len() - 1
            }
        };
        let d = &mut due[at];
        for r in &j.job.reasons {
            if !d.reasons.contains(r) {
                d.reasons.push(r.clone());
            }
        }
        for c in &j.job.changed {
            if !d.changed.contains(c) {
                d.changed.push(c.clone());
            }
        }
        d.debt_added.extend(j.job.debt_added.iter().cloned());
    }
    due
}

/// The one reviewer job of `repo` for the `due` units its review brief covers; the units over the
/// review budget are named as next round and stay due.
fn reviewer_job(
    ws: &Workspace,
    repo: &Repo,
    due: &[Due],
    task: &str,
    snapshot: &str,
    write_brief: bool,
) -> Result<Job> {
    let units: Vec<DueUnit> = due
        .iter()
        .map(|d| (d.unit.clone(), d.reasons.clone()))
        .collect();
    let (text, pack) = brief::build_review(repo, &units, task, snapshot)?;
    let brief_path = if write_brief {
        Some(brief::write_text(repo, None, Role::Reviewer, &text)?)
    } else {
        None
    };
    let covered = &due[..pack.units.len()];
    let mut reasons: Vec<String> = covered
        .iter()
        .flat_map(|d| {
            let name = ws.display_name(repo, &d.unit);
            d.reasons.iter().map(move |r| format!("{name}: {r}"))
        })
        .collect();
    if !pack.next_round.is_empty() {
        let later: Vec<String> = pack
            .next_round
            .iter()
            .map(|u| ws.display_name(repo, u))
            .collect();
        reasons.push(format!(
            "next round, over the review budget: {}",
            later.join(", ")
        ));
    }
    Ok(Job {
        unit: String::new(),
        role: Role::Reviewer,
        task: task.to_string(),
        snapshot: snapshot.to_string(),
        reasons,
        changed: covered.iter().flat_map(|d| d.changed.clone()).collect(),
        brief_path,
        command: if ws.is_workspace {
            format!("stewards --repo {} brief --role reviewer", repo.name)
        } else {
            "stewards brief --role reviewer".into()
        },
        debt_added: covered.iter().flat_map(|d| d.debt_added.clone()).collect(),
    })
}

/// The snapshot, task, and the stamped units of `repo` due for review (units.toml order) with
/// their reasons, for `brief --role reviewer` and `pack --role reviewer`.
pub fn review_due(repo: &Repo, task: Option<&str>) -> Result<(String, String, Vec<DueUnit>)> {
    let ws = Workspace {
        repos: vec![Repo {
            name: repo.name.clone(),
            root: repo.root.clone(),
            loaded: Some(repo.loaded()?.clone()),
        }],
        is_workspace: false,
        selected: true,
    };
    let (jobs, snapshots) = role_jobs(&ws, task)?;
    let task = match task {
        Some(t) => t.to_string(),
        None => git::task_text(&repo.root)?,
    };
    let jobs: Vec<&RoleJob> = jobs.iter().collect();
    let due = due_for_review(&jobs)
        .into_iter()
        .map(|d| (d.unit, d.reasons))
        .collect();
    Ok((snapshots[0].clone(), task, due))
}

/// Every job `next --roles` lists, and each repo's snapshot.
fn role_jobs<'a>(ws: &'a Workspace, task: Option<&str>) -> Result<(Vec<RoleJob<'a>>, Vec<String>)> {
    let mut jobs: Vec<RoleJob> = Vec::new();
    // Stamped units changed since their stamps: (name, repo, task, snapshot, changed files).
    let mut moved: Vec<(String, &Repo, String, String, Vec<String>)> = Vec::new();
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
            let mut unit_jobs = vec![];
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
                    NEVER_REVIEWED
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
                    moved.push((name.clone(), repo, task.clone(), snapshot.clone(), changed));
                }
                unit_jobs.push(Job {
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
                unit_jobs.push(job(Role::Tester, reasons, changed));
            }

            let documenter_mark = reviewed.and_then(|r| r.documenter.as_ref()).or(stamp);
            let ((mut reasons, changed), _) = touched(documenter_mark)?;
            // a card a first review drafted has an interface to document
            if card.is_some_and(needs_docs) {
                reasons.push("no docs yet".into());
            }
            if reviewed.is_some() && !reasons.is_empty() {
                unit_jobs.push(job(Role::Documenter, reasons, changed));
            }

            let bug = card.and_then(|c| {
                c.gaps
                    .iter()
                    .find(|g| g.kind == GapKind::Bug && g.severity.is_none())
            });
            if let Some(bug) = bug {
                let reasons = vec![format!("bug not judged: {}", bug.title)];
                unit_jobs.push(job(Role::Validator, reasons, vec![]));
            }
            jobs.extend(unit_jobs.into_iter().map(|job| RoleJob {
                repo,
                unit: Some(def.name.clone()),
                job,
            }));
        }
    }

    // No plans exist yet, so no plan ever covers the moved units.
    if moved.len() >= 2 {
        let names: Vec<&str> = moved.iter().map(|(n, ..)| n.as_str()).collect();
        let (_, repo, task, snapshot, _) = &moved[0];
        let repo_name = &repo.name;
        let job = Job {
            unit: String::new(),
            role: Role::Architect,
            task: task.clone(),
            snapshot: snapshot.clone(),
            reasons: vec![format!(
                "units changed since their stamps: {}",
                names.join(", ")
            )],
            changed: moved.iter().flat_map(|(.., c)| c.clone()).collect(),
            brief_path: None,
            command: if ws.is_workspace {
                format!("stewards --repo {repo_name} brief --role architect")
            } else {
                "stewards brief --role architect".into()
            },
            debt_added: vec![],
        };
        jobs.insert(
            0,
            RoleJob {
                repo,
                unit: None,
                job,
            },
        );
    }
    Ok((jobs, snapshots))
}

/// A card with an interface and no doc section written: `next` lists it until one is.
pub(crate) fn needs_docs(card: &UnitCard) -> bool {
    !card.interface.is_empty()
        && card.doc.as_ref().is_none_or(|d| {
            [&d.overview, &d.how_it_works, &d.how_to_change]
                .iter()
                .all(|s| s.trim().is_empty())
        })
}

/// Extensions of the languages `anchors::interface_lines`'s block rule was written for.
const BLOCK_LANGUAGES: &[&str] = &[
    "rs", "py", "ts", "tsx", "js", "jsx", "mjs", "cjs", "go", "java", "kt", "c", "h", "cc", "cpp",
    "hpp", "cs", "rb", "php", "swift", "scala", "sh",
];

/// `(reasons, files)` for the interfaces a change since `mark` touched, in card order: a hunk
/// (`mark` -> `snapshot`) whose range or the line just before or after it, on the old side or
/// the new, is one of the interface's lines in that side's file. `changed` is the files changed
/// since `mark`. An interface whose lines can't be found (a file not in `BLOCK_LANGUAGES`, no
/// mark tree, file gone or without hunks, name absent, a blob unreadable) counts when its file
/// changed.
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
            let ext = Path::new(&a.path).extension()?.to_str()?;
            if file_hunks.is_empty() || !BLOCK_LANGUAGES.contains(&ext) {
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
    roles: bool,
    _no_run: bool,
    pr: Option<u64>,
) -> Result<ExitCode> {
    if pr.is_some() {
        return not_implemented("next --pr");
    }
    let (_, ws) = here(g)?;
    let report = next(&ws, task, !no_brief, roles)?;
    emit(&report, g.human);
    Ok(
        if report.jobs.is_empty() && report.check.failures.is_empty() {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(1)
        },
    )
}

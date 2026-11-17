// The context pack and the `pack` command.
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Result, bail};
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::Serialize;

use crate::anchors::{self, Resolution};
use crate::check::{changed_since, in_snapshot, one_line, stale_files};
use crate::git;
use crate::llm::types::Task;
use crate::model::{
    Anchor, CommandEntry, Commands, Decision, Doc, Flow, Gap, GapKind, Interface, Loaded, Role,
    Scenario,
};
use crate::next::DueUnit;
use crate::output::{CliError, Global, Human, emit};
use crate::route::Router;
use crate::submit::{StoredFinding, read_findings};
use crate::workspace::{Repo, here};

/// Lines of source on each side of an anchor's match.
const WINDOW: usize = 20;
const HUNK_LINES: usize = 40;
const HALVED: &str = "source windows halved";
/// Owned files rendered in a first review's list; the JSON carries them all.
const OWNED_SHOWN: usize = 200;
/// Bytes of the whole review pack, every unit's section together.
pub const REVIEW_BUDGET: usize = 48_000;

#[derive(Debug, Clone, Serialize)]
pub struct Pack {
    pub unit: Option<String>,
    pub role: Role,
    pub task: String,
    /// Why the review brief covers the unit: the reasons of its jobs `next --roles` lists.
    pub due_because: Option<Vec<String>>,
    pub budget: usize,
    pub bytes: usize,
    pub over_budget: bool,
    pub purpose: Option<String>,
    /// The unit's globs from units.toml.
    pub paths: Option<Vec<String>>,
    pub interface: Option<Vec<Interface>>,
    pub anchors: Option<Vec<PackAnchor>>,
    pub scenarios: Option<Vec<Scenario>>,
    pub test_command: Option<String>,
    pub primary_paths: Option<Vec<String>>,
    pub test_commands: Option<Vec<String>>,
    pub gaps: Option<Vec<Gap>>,
    /// The tester's and reviewer's view of the unit's bug gaps, so a defect already recorded is
    /// reported under its title rather than again under a new one.
    pub known_bugs: Option<Vec<Gap>>,
    /// The reviewer's view of the unit's findings the agent answered (`review --answer`): pending
    /// ones to accept or contest, accepted and contested ones as they stand.
    pub answered: Option<Vec<StoredFinding>>,
    pub flows: Option<Vec<Flow>>,
    pub decisions: Option<Vec<Decision>>,
    pub units: Option<Vec<UnitSummary>>,
    pub doc: Option<Doc>,
    pub changed_files: Option<Vec<String>>,
    pub hunks: Option<Vec<Hunk>>,
    pub other_changed_files: Option<Vec<String>>,
    pub memory: Option<Vec<String>>,
    pub bug: Option<Gap>,
    pub dev_task: Option<serde_json::Value>,
    pub repro_result: Option<serde_json::Value>,
    pub dropped: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackAnchor {
    pub path: String,
    pub name: String,
    /// None = the anchor is dead.
    pub line: Option<u32>,
    pub source: Option<String>,
    /// The file line `source` starts at.
    pub source_start: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnitSummary {
    pub name: String,
    pub purpose: String,
    pub interface_ids: Vec<String>,
    pub paths: Vec<String>,
    /// How many files the unit owns in the snapshot.
    pub files: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Hunk {
    pub path: String,
    /// None = the file matches `[pack] exclude`.
    pub diff: Option<String>,
}

fn usage(message: String, fix: &str) -> anyhow::Error {
    CliError::new("usage", message, fix).into()
}

/// Matches the full path or the file name, so `.env*` also catches `app/.env`.
fn is_excluded(exclude: &GlobSet, path: &str) -> bool {
    exclude.is_match(path)
        || Path::new(path)
            .file_name()
            .is_some_and(|n| exclude.is_match(n))
}

fn window(
    root: &Path,
    entries: &[(String, String)],
    a: Anchor,
    exclude: &GlobSet,
    dropped: &mut Vec<String>,
) -> PackAnchor {
    let resolution = if in_snapshot(entries, &a.path) {
        anchors::resolve(root, &a)
    } else {
        Resolution::DeadPath
    };
    let line = match resolution {
        Resolution::Found { line } => Some(line),
        Resolution::DeadPath | Resolution::DeadName => None,
    };
    let shown = if is_excluded(exclude, &a.path) {
        dropped.push(format!(
            "source {}::{} (excluded by [pack] exclude)",
            a.path, a.name
        ));
        None
    } else {
        line.and(anchors::source_window(root, &a, WINDOW, WINDOW))
    };
    PackAnchor {
        path: a.path,
        name: a.name,
        line,
        source_start: shown.as_ref().map(|(start, _)| *start),
        source: shown.map(|(_, source)| source),
    }
}

/// The pack for `unit` in `role`; `--diff` hunks run from the stamped tree to `snapshot`. The
/// developer's pack needs its task, so it comes from `developer`.
pub fn pack(
    repo: &Repo,
    unit: Option<&str>,
    role: Role,
    task: &str,
    diff: bool,
    snapshot: &str,
) -> Result<Pack> {
    assemble(repo, unit, role, task, diff, snapshot, Extra::default())
}

/// The validator's pack for the bug titled `bug` (the first unjudged one when `None`), with the
/// result of running its repro (`None`: not run).
pub fn validator(
    repo: &Repo,
    unit: &str,
    task: &str,
    snapshot: &str,
    bug: Option<&str>,
    repro: Option<serde_json::Value>,
) -> Result<Pack> {
    let extra = Extra {
        bug,
        repro,
        ..Extra::default()
    };
    assemble(
        repo,
        Some(unit),
        Role::Validator,
        task,
        false,
        snapshot,
        extra,
    )
}

/// What only one role's pack takes.
#[derive(Default)]
struct Extra<'a> {
    /// The developer's task number and task.
    dev: Option<(usize, &'a Task)>,
    /// The validator's bug title and repro result.
    bug: Option<&'a str>,
    repro: Option<serde_json::Value>,
}

/// The developer's pack for task `n` of `unit`'s task list.
pub fn developer(
    repo: &Repo,
    unit: &str,
    task: &str,
    n: usize,
    dev: &Task,
    snapshot: &str,
) -> Result<Pack> {
    assemble(
        repo,
        Some(unit),
        Role::Developer,
        task,
        false,
        snapshot,
        Extra {
            dev: Some((n, dev)),
            ..Extra::default()
        },
    )
}

fn assemble(
    repo: &Repo,
    unit: Option<&str>,
    role: Role,
    task: &str,
    diff: bool,
    snapshot: &str,
    extra: Extra,
) -> Result<Pack> {
    let dev = extra.dev;
    match (role, unit) {
        (Role::Architect, Some(u)) => bail!(usage(
            format!("the architect's pack takes no unit, got `{u}`"),
            "stewards pack --role architect",
        )),
        (Role::Architect, None) | (_, Some(_)) => {}
        (_, None) => bail!(usage(
            format!("the {} pack needs a unit", role.as_str()),
            "stewards pack <unit> --role <role>",
        )),
    }
    let loaded = repo.loaded()?;
    let root = &repo.root;
    let b = &loaded.repo.budgets;
    let budget = match role {
        Role::Developer => b.developer,
        Role::Maintainer => b.maintainer,
        Role::Tester => b.tester,
        Role::Documenter => b.documenter,
        Role::Validator => b.validator,
        Role::Architect => b.architect,
        Role::Reviewer => REVIEW_BUDGET,
    };
    let mut exclude = GlobSetBuilder::new();
    for g in &loaded.repo.pack.exclude {
        exclude.add(Glob::new(g).map_err(|e| {
            CliError::new(
                "invalid_toml",
                format!(".stewards/repo.toml: bad [pack] exclude glob `{g}`: {e}"),
                "fix [pack] exclude in .stewards/repo.toml",
            )
        })?);
    }
    let exclude = exclude.build()?;

    let mut p = Pack {
        unit: unit.map(String::from),
        role,
        task: task.to_string(),
        due_because: None,
        budget,
        bytes: 0,
        over_budget: false,
        purpose: None,
        paths: None,
        interface: None,
        anchors: None,
        scenarios: None,
        test_command: None,
        primary_paths: None,
        test_commands: None,
        gaps: None,
        known_bugs: None,
        answered: None,
        flows: None,
        decisions: None,
        units: None,
        doc: None,
        changed_files: None,
        hunks: None,
        other_changed_files: None,
        memory: None,
        bug: None,
        dev_task: None,
        repro_result: None,
        dropped: vec![],
    };

    let Some(name) = unit else {
        let router = Router::new(&loaded.units, &loaded.repo.docs_path)?;
        let entries = git::ls_tree(root, snapshot)?;
        p.units = Some(
            loaded
                .units
                .iter()
                .map(|u| UnitSummary {
                    name: u.name.clone(),
                    purpose: u.purpose.clone(),
                    interface_ids: loaded.cards.get(&u.name).map_or(vec![], |c| {
                        c.interface.iter().map(|i| i.id.clone()).collect()
                    }),
                    paths: u.paths.clone(),
                    files: entries
                        .iter()
                        .filter(|(path, _)| router.owner(path) == Some(u.name.as_str()))
                        .count(),
                })
                .collect(),
        );
        p.flows = Some(loaded.repo.flows.clone());
        p.decisions = Some(loaded.repo.decisions.clone());
        p.fit(budget);
        return Ok(p);
    };

    let def = loaded.unit(name)?;
    p.paths = Some(def.paths.clone());
    let card = loaded.cards.get(name);
    let test = loaded.tests.get(name);
    let interface = card.map_or(vec![], |c| c.interface.clone());
    let flows = loaded
        .repo
        .flows
        .iter()
        .filter(|f| f.units.iter().any(|u| u == name))
        .cloned()
        .collect();
    let primary_paths = test.map_or(vec![], |t| t.primary_paths.clone());
    let test_command = test.and_then(|t| t.test_command.clone());

    let mut anchor_specs = match role {
        Role::Developer => {
            let Some((n, t)) = dev else {
                bail!(usage(
                    "the developer pack is built by `brief`, from the unit's task list".into(),
                    &format!("stewards brief {name} --role developer"),
                ));
            };
            p.purpose = Some(def.purpose.clone());
            p.gaps = Some(card.map_or(vec![], |c| c.gaps.clone()));
            p.flows = Some(flows);
            p.dev_task = Some(
                serde_json::json!({"n": n, "title": t.title, "change": t.change,
                                                 "files": t.files, "check": t.check}),
            );
            // a developer sees only the anchors in its lane
            let anchors = card.map_or(vec![], |c| c.anchors.clone());
            Some(
                anchors
                    .into_iter()
                    .filter(|a| t.files.contains(&a.path))
                    .collect(),
            )
        }
        Role::Maintainer | Role::Reviewer => {
            p.purpose = Some(def.purpose.clone());
            p.gaps = Some(card.map_or(vec![], |c| c.gaps.clone()));
            p.flows = Some(flows);
            // the reviewer also sees what the tester and documenter would
            if role == Role::Reviewer {
                let (bugs, gaps) = p
                    .gaps
                    .take()
                    .unwrap_or_default()
                    .into_iter()
                    .partition(is_bug);
                p.gaps = Some(gaps);
                p.known_bugs = Some(bugs);
                p.scenarios = Some(test.map_or(vec![], |t| t.scenarios.clone()));
                p.test_command = test_command;
                p.test_commands = Some(test_commands(&loaded.repo.commands));
                p.doc = Some(card.and_then(|c| c.doc.clone()).unwrap_or_default());
                let mut answered = read_findings(repo, name)?;
                answered.retain(|f| f.answer.is_some());
                p.answered = Some(answered);
            }
            Some(card.map_or(vec![], |c| c.anchors.clone()))
        }
        Role::Tester => {
            p.known_bugs = Some(card.map_or(vec![], |c| {
                c.gaps.iter().filter(|g| is_bug(g)).cloned().collect()
            }));
            p.scenarios = Some(test.map_or(vec![], |t| t.scenarios.clone()));
            p.test_command = test_command;
            p.primary_paths = Some(primary_paths);
            // An unparseable anchor stays listed, as dead.
            Some(
                interface
                    .iter()
                    .map(|i| {
                        Anchor::parse(&i.anchor).unwrap_or_else(|_| Anchor {
                            path: i.anchor.clone(),
                            name: String::new(),
                        })
                    })
                    .collect(),
            )
        }
        Role::Documenter => {
            p.purpose = Some(def.purpose.clone());
            p.doc = Some(card.and_then(|c| c.doc.clone()).unwrap_or_default());
            p.flows = Some(flows);
            None
        }
        Role::Validator => {
            p.purpose = Some(def.purpose.clone());
            p.primary_paths = Some(primary_paths);
            p.test_command = test_command;
            p.test_commands = Some(test_commands(&loaded.repo.commands));
            p.bug = card
                .and_then(|c| {
                    c.gaps.iter().find(|g| match extra.bug {
                        Some(t) => g.kind == GapKind::Bug && g.title == t,
                        None => g.kind == GapKind::Bug && g.severity.is_none(),
                    })
                })
                .cloned();
            p.repro_result = extra.repro;
            None
        }
        Role::Architect => unreachable!("the architect has no unit"),
    };
    p.interface = Some(interface);

    let stamped = card
        .and_then(|c| c.reviewed.as_ref())
        .filter(|r| git::has_object(root, &r.tree));
    if diff
        && matches!(role, Role::Maintainer | Role::Tester | Role::Reviewer)
        && let Some(reviewed) = stamped
    {
        let router = Router::new(&loaded.units, &loaded.repo.docs_path)?;
        let entries = git::ls_tree(root, snapshot)?;
        let mut tree = reviewed.tree.as_str();
        let mut owned = stale_files(root, snapshot, &entries, &router, name, Some(reviewed))?
            .unwrap_or_default();
        if role == Role::Reviewer {
            // From the oldest of the stamp and the role marks, so a unit due only because a mark
            // is behind shows the change that mark has not seen.
            // simplified: a tree has no time, so the oldest is the one the most owned files
            // changed since; a revert can fool it. Record each mark's time if that shows up.
            let marks = [reviewed.tester.as_deref(), reviewed.documenter.as_deref()];
            for mark in marks.into_iter().flatten() {
                if !git::has_object(root, mark) {
                    continue;
                }
                let since: Vec<String> =
                    changed_since(root, snapshot, &entries, &router, name, Some(mark))?
                        .into_iter()
                        .filter(|p| router.owner(p) == Some(name))
                        .collect();
                if since.len() > owned.len() {
                    (tree, owned) = (mark, since);
                }
            }
        }
        if let Some(specs) = &mut anchor_specs {
            specs.retain(|a| owned.contains(&a.path));
        }
        let mut hunks = Vec::new();
        for path in &owned {
            let diff = if is_excluded(&exclude, path) {
                p.dropped
                    .push(format!("hunk {path} (excluded by [pack] exclude)"));
                None
            } else {
                Some(git::diff_file(root, tree, snapshot, path)?)
            };
            hunks.push(Hunk {
                path: path.clone(),
                diff,
            });
        }
        p.hunks = Some(hunks);
        p.changed_files = Some(owned);
        // the review brief lists them once, for every unit
        if role == Role::Maintainer {
            let other = other_changed(root, snapshot, &entries, &router, loaded, &[name])?;
            p.other_changed_files = Some(other);
        }
    } else if diff && matches!(role, Role::Maintainer | Role::Reviewer) {
        // A first review: every file the unit owns, as `next` lists for a never-stamped unit.
        let router = Router::new(&loaded.units, &loaded.repo.docs_path)?;
        let entries = git::ls_tree(root, snapshot)?;
        let mut owned =
            stale_files(root, snapshot, &entries, &router, name, None)?.unwrap_or_default();
        owned.sort();
        p.changed_files = Some(owned);
    }
    if let Some(specs) = anchor_specs {
        let entries = git::ls_tree(root, snapshot)?;
        p.anchors = Some(
            specs
                .into_iter()
                .map(|a| window(root, &entries, a, &exclude, &mut p.dropped))
                .collect(),
        );
    }
    // `review` fits the review pack's units together
    if role != Role::Reviewer {
        p.fit(budget);
    }
    Ok(p)
}

fn is_bug(g: &Gap) -> bool {
    g.kind == GapKind::Bug
}

/// The repo's `commands.test` entries.
fn test_commands(commands: &Commands) -> Vec<String> {
    commands
        .test
        .iter()
        .map(|c| match c {
            CommandEntry::Plain(cmd) | CommandEntry::Timed { cmd, .. } => cmd.clone(),
        })
        .collect()
}

/// The files of every unit but `skip` changed since that unit's own stamp; fresh units contribute
/// nothing.
fn other_changed(
    root: &Path,
    snapshot: &str,
    entries: &[(String, String)],
    router: &Router,
    loaded: &Loaded,
    skip: &[&str],
) -> Result<Vec<String>> {
    let mut other = Vec::new();
    for u in loaded
        .units
        .iter()
        .filter(|u| !skip.contains(&u.name.as_str()))
    {
        let reviewed = loaded.cards.get(&u.name).and_then(|c| c.reviewed.as_ref());
        other.extend(
            stale_files(root, snapshot, entries, router, &u.name, reviewed)?.unwrap_or_default(),
        );
    }
    other.sort();
    Ok(other)
}

/// One review brief's pack: the reviewer's section of each of its units, every section together
/// within `REVIEW_BUDGET`. The files changed in units not due are listed once.
#[derive(Debug, Clone, Serialize)]
pub struct ReviewPack {
    pub budget: usize,
    pub bytes: usize,
    pub units: Vec<Pack>,
    pub other_changed_files: Vec<String>,
}

/// Every due unit of a round, as `review` groups them: one `ReviewPack` per review brief.
#[derive(Debug, Clone, Serialize)]
pub struct ReviewRound {
    pub groups: Vec<ReviewPack>,
}

/// The review packs of `due` (in units.toml order): units join a group while the whole rendering
/// fits `REVIEW_BUDGET`, and the first that does not starts the next group. A unit alone over the
/// budget is shrunk by `squeeze` and no other unit joins it. Every due unit is in exactly one group.
pub fn review(repo: &Repo, due: &[DueUnit], task: &str, snapshot: &str) -> Result<ReviewRound> {
    let loaded = repo.loaded()?;
    let router = Router::new(&loaded.units, &loaded.repo.docs_path)?;
    let entries = git::ls_tree(&repo.root, snapshot)?;
    let names: Vec<&str> = due.iter().map(|(u, _)| u.as_str()).collect();
    let other = other_changed(&repo.root, snapshot, &entries, &router, loaded, &names)?;
    let empty = || ReviewPack {
        budget: REVIEW_BUDGET,
        bytes: 0,
        units: vec![],
        other_changed_files: other.clone(),
    };
    // what a unit section may take: the budget less the rest and the newline joining them
    let room = |r: &ReviewPack| REVIEW_BUDGET.saturating_sub(r.render().len() + 1);
    let mut groups = vec![empty()];
    for (unit, reasons) in due {
        let mut p = assemble(
            repo,
            Some(unit),
            Role::Reviewer,
            task,
            true,
            snapshot,
            Extra::default(),
        )?;
        p.due_because = Some(reasons.clone());
        p.bytes = p.render().len();
        let mut r = groups.last_mut().expect("one group at least");
        if p.bytes > room(r) && !r.units.is_empty() {
            groups.push(empty());
            r = groups.last_mut().expect("just pushed");
        }
        if p.bytes > room(r) {
            p.squeeze(room(r));
            r.units.push(p);
            groups.push(empty());
        } else {
            r.units.push(p);
        }
    }
    groups.retain(|r| !r.units.is_empty());
    for r in &mut groups {
        r.bytes = r.render().len();
    }
    Ok(ReviewRound { groups })
}

impl ReviewPack {
    pub fn render(&self) -> String {
        let mut out: Vec<String> = self.units.iter().map(Pack::render).collect();
        if !self.other_changed_files.is_empty() {
            out.push(format!(
                "# Other changed files\n\nIn units not under review:\n\n{}",
                paths_table(&self.other_changed_files)
            ));
        }
        out.join("\n")
    }
}

impl Human for ReviewRound {
    fn render(&self) -> String {
        let groups: Vec<String> = self.groups.iter().map(ReviewPack::render).collect();
        groups.join("\n")
    }
}

/// The part of `source` (starting at file line `start`) within half the window's reach of the
/// match at file line `line`, and where it starts; the match line stays.
fn halve(source: &str, start: u32, line: u32) -> (u32, String) {
    let lines: Vec<&str> = source.lines().collect();
    let at = (line.saturating_sub(start) as usize).min(lines.len().saturating_sub(1));
    let from = at.saturating_sub(WINDOW / 2);
    let end = (at + WINDOW / 2).min(lines.len().saturating_sub(1));
    let kept = lines
        .get(from..=end)
        .map_or_else(|| source.to_string(), |l| l.join("\n"));
    (start + from as u32, kept)
}

fn table(headers: &[&str], rows: Vec<Vec<String>>) -> String {
    if rows.is_empty() {
        return "(none)\n".into();
    }
    let mut t = format!(
        "| {} |\n|{}\n",
        headers.join(" | "),
        "---|".repeat(headers.len())
    );
    for row in rows {
        let cells: Vec<String> = row
            .iter()
            .map(|c| one_line(&c.replace('|', "\\|")))
            .collect();
        t += &format!("| {} |\n", cells.join(" | "));
    }
    t
}

pub(crate) fn fenced(lang: &str, body: &str) -> String {
    // A fence longer than any backtick run in the body cannot be closed from inside it.
    let longest = body.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest.max(2) + 1);
    format!("{fence}{lang}\n{body}\n{fence}\n")
}

fn bullets(items: &[String]) -> String {
    if items.is_empty() {
        return "(none)\n".into();
    }
    items
        .iter()
        .map(|i| format!("- {}\n", one_line(i)))
        .collect()
}

fn paths_table(paths: &[String]) -> String {
    table(&["path"], paths.iter().map(|p| vec![p.clone()]).collect())
}

impl Pack {
    /// The --human markdown; its length is what the budget measures.
    pub fn render(&self) -> String {
        let mut out = format!(
            "# Pack: {} ({})\n\nTask: {}\n",
            self.unit.as_deref().unwrap_or("repo"),
            self.role.as_str(),
            one_line(&self.task)
        );
        if let Some(reasons) = &self.due_because {
            out += &format!("Due because: {}\n", one_line(&reasons.join("; ")));
        }
        if self.role == Role::Reviewer && !self.dropped.is_empty() {
            out += &format!("Left out to fit: {}\n", one_line(&self.dropped.join("; ")));
        }
        let mut section = |title: &str, body: String| {
            out += &format!("\n## {title}\n\n{body}");
        };
        if let Some(purpose) = &self.purpose {
            section("Purpose", fenced("", purpose));
        }
        if let Some(list) = &self.paths {
            section("Paths", bullets(list));
        }
        if let Some(t) = &self.dev_task {
            let text = |v: &serde_json::Value| v.as_str().unwrap_or_default().to_string();
            let files = t["files"]
                .as_array()
                .map_or(vec![], |f| f.iter().map(text).collect());
            let check = t["check"].as_str().map(|c| vec![c.to_string()]);
            section(
                "Developer task",
                format!(
                    "{}\n### Change\n\n{}\n### Lane\n\n{}\n### Check\n\n{}",
                    table(
                        &["n", "title"],
                        vec![vec![t["n"].to_string(), text(&t["title"])]]
                    ),
                    fenced("", &text(&t["change"])),
                    paths_table(&files),
                    table(&["command"], check.into_iter().collect())
                ),
            );
        }
        if let Some(list) = &self.interface {
            let rows = list
                .iter()
                .map(|i| vec![i.id.clone(), i.anchor.clone(), i.contract.clone()])
                .collect();
            section("Interface", table(&["id", "anchor", "contract"], rows));
        }
        if let Some(list) = &self.anchors {
            let mut body = String::new();
            for a in list {
                let mut line = a.line.map_or("dead".into(), |l| format!("line {l}"));
                if let (Some(_), Some(start)) = (&a.source, a.source_start) {
                    line += &format!(", source from line {start}");
                }
                body += &format!(
                    "### `{}::{}` ({line})\n\n",
                    one_line(&a.path),
                    one_line(&a.name)
                );
                body += &match &a.source {
                    Some(s) => fenced("", s),
                    None => "(source not inlined)\n".into(),
                };
            }
            section(
                "Anchors",
                if body.is_empty() {
                    "(none)\n".into()
                } else {
                    body
                },
            );
        }
        if let Some(list) = &self.scenarios {
            let rows = list
                .iter()
                .map(|s| {
                    vec![
                        s.id.clone(),
                        s.interface.clone(),
                        s.given.clone(),
                        s.expect.clone(),
                        s.level.clone(),
                        s.command.clone().unwrap_or_default(),
                    ]
                })
                .collect();
            let headers = ["id", "interface", "given", "expect", "level", "command"];
            section("Scenarios", table(&headers, rows));
        }
        if let Some(cmd) = &self.test_command {
            section("Test command", format!("`{}`\n", one_line(cmd)));
        }
        if let Some(list) = &self.primary_paths {
            let rows = list.iter().map(|id| vec![id.clone()]).collect();
            section("Primary paths", table(&["interface id"], rows));
        }
        if let Some(list) = &self.test_commands {
            let rows = list.iter().map(|c| vec![c.clone()]).collect();
            section("Test commands", table(&["command"], rows));
        }
        if let Some(list) = &self.gaps {
            let rows = list
                .iter()
                .map(|g| {
                    vec![
                        format!("{:?}", g.kind).to_lowercase(),
                        g.title.clone(),
                        g.anchor.clone().unwrap_or_default(),
                        g.note.clone(),
                    ]
                })
                .collect();
            section("Gaps", table(&["kind", "title", "anchor", "note"], rows));
        }
        if let Some(list) = self.answered.as_ref().filter(|l| !l.is_empty()) {
            let rows = list
                .iter()
                .map(|f| {
                    let state = match (&f.contested, f.waived) {
                        (Some(why), _) => format!("contested: {why}"),
                        (None, true) => "accepted: do not raise it again".into(),
                        (None, false) => "answered: accept or contest".into(),
                    };
                    vec![
                        format!("{}:{}", f.file, f.line),
                        format!("cut {}; replace with {}", f.cut, f.replace_with),
                        f.answer.clone().unwrap_or_default(),
                        state,
                    ]
                })
                .collect();
            section(
                "Answered findings",
                "Findings of an earlier round the agent gave a reason not to apply. For each \
                 answered one, accept the reason (do not raise the finding again) or contest it \
                 once in `contested`.\n\n"
                    .to_string()
                    + &table(&["file:line", "finding", "agent's reason", "state"], rows),
            );
        }
        if let Some(list) = &self.known_bugs {
            let rows = list
                .iter()
                .map(|g| {
                    vec![
                        g.title.clone(),
                        g.anchor.clone().unwrap_or_default(),
                        g.note.clone(),
                        g.severity.clone().unwrap_or_else(|| "not judged".into()),
                    ]
                })
                .collect();
            section(
                "Known bugs",
                "When you report a defect listed here, reuse its exact title so the entry is \
                 updated; a bug with a new title on a listed bug's anchor is not recorded.\n\n"
                    .to_string()
                    + &table(&["title", "anchor", "note", "severity"], rows),
            );
        }
        if let Some(list) = &self.flows {
            let rows = list
                .iter()
                .map(|f| {
                    vec![
                        f.name.clone(),
                        f.units.join(", "),
                        f.entry.clone(),
                        f.check.clone().unwrap_or_default(),
                    ]
                })
                .collect();
            section("Flows", table(&["name", "units", "entry", "check"], rows));
        }
        if let Some(list) = &self.decisions {
            let rows = list
                .iter()
                .map(|d| vec![d.title.clone(), d.why.clone()])
                .collect();
            section("Decisions", table(&["title", "why"], rows));
        }
        if let Some(list) = &self.units {
            let rows = list
                .iter()
                .map(|u| {
                    vec![
                        u.name.clone(),
                        u.purpose.clone(),
                        u.interface_ids.join(", "),
                        u.paths.join(", "),
                        u.files.to_string(),
                    ]
                })
                .collect();
            let headers = ["name", "purpose", "interface ids", "paths", "files"];
            section("Units", table(&headers, rows));
        }
        if let Some(doc) = &self.doc {
            section(
                "Doc",
                format!(
                    "### Overview\n\n{}\n### How it works\n\n{}\n### How to change\n\n{}",
                    fenced("", &doc.overview),
                    fenced("", &doc.how_it_works),
                    fenced("", &doc.how_to_change)
                ),
            );
        }
        match (&self.changed_files, &self.hunks) {
            (Some(list), Some(_)) => section("Changed files", paths_table(list)),
            (Some(list), None) => {
                let mut body = "This unit has not been reviewed yet: these are all its files.\n\n"
                    .to_string()
                    + &bullets(&list[..list.len().min(OWNED_SHOWN)]);
                if list.len() > OWNED_SHOWN {
                    body += &format!("(+{} more)\n", list.len() - OWNED_SHOWN);
                }
                section("Owned files", body);
            }
            (None, _) => {}
        }
        if let Some(list) = &self.hunks {
            let mut body = String::new();
            for h in list {
                body += &format!("### {}\n\n", one_line(&h.path));
                body += &match &h.diff {
                    Some(d) => fenced("diff", d.trim_end()),
                    None => "(not inlined: matches [pack] exclude)\n".into(),
                };
            }
            section(
                "Hunks",
                if body.is_empty() {
                    "(none)\n".into()
                } else {
                    body
                },
            );
        }
        if let Some(list) = &self.other_changed_files {
            section("Other changed files", paths_table(list));
        }
        if let Some(bug) = &self.bug {
            let field = |v: Option<&str>| v.map_or("(none)\n".into(), |v| fenced("", v));
            section(
                "Bug",
                format!(
                    "### Title\n\n{}\n### Repro\n\n{}\n### Note\n\n{}\n### Anchor\n\n{}",
                    field(Some(&bug.title)),
                    field(bug.repro.as_deref()),
                    field(Some(&bug.note)),
                    field(bug.anchor.as_deref())
                ),
            );
        }
        if self.role == Role::Validator && self.bug.is_some() {
            let body = self.repro_result.as_ref().map_or("not run\n".into(), |r| {
                fenced("json", &serde_json::to_string_pretty(r).unwrap_or_default())
            });
            out += &format!("\n## Repro result\n\n{body}");
        }
        out
    }

    /// Shrinks the pack in the fixed order until its rendering fits `budget`.
    pub fn fit(&mut self, budget: usize) {
        self.budget = budget;
        let fits = |p: &Pack| p.render().len() <= budget;
        let has_source = |p: &Pack| p.anchors.iter().flatten().any(|a| a.source.is_some());
        if !fits(self) && has_source(self) && !self.dropped.iter().any(|d| d == HALVED) {
            for a in self.anchors.iter_mut().flatten() {
                if let (Some(source), Some(start), Some(line)) = (&a.source, a.source_start, a.line)
                {
                    let (start, source) = halve(source, start, line);
                    (a.source_start, a.source) = (Some(start), Some(source));
                }
            }
            self.dropped.push(HALVED.into());
        }
        while !fits(self)
            && let Some(a) = self
                .anchors
                .iter_mut()
                .flatten()
                .rev()
                .find(|a| a.source.is_some())
        {
            a.source = None;
            self.dropped.push(format!("source {}::{}", a.path, a.name));
        }
        if !fits(self) && self.gaps.take().is_some() {
            self.dropped.push("gaps".into());
        }
        if !fits(self) && self.known_bugs.take().is_some() {
            self.dropped.push("known bugs".into());
        }
        if !fits(self) && self.flows.take().is_some() {
            self.dropped.push("flows".into());
        }
        // a first review's owned-files list; a re-review's changed files stay with their hunks
        if !fits(self) && self.hunks.is_none() && self.changed_files.take().is_some() {
            self.dropped.push("owned files".into());
        }
        let long = |h: &Hunk| {
            h.diff
                .as_ref()
                .is_some_and(|d| d.lines().count() > HUNK_LINES)
        };
        if !fits(self) && self.hunks.iter().flatten().any(long) {
            for h in self.hunks.iter_mut().flatten().filter(|h| long(h)) {
                h.diff = h
                    .diff
                    .as_ref()
                    .map(|d| d.lines().take(HUNK_LINES).collect::<Vec<_>>().join("\n"));
            }
            self.dropped
                .push(format!("hunks truncated to {HUNK_LINES} lines"));
        }
        self.bytes = self.render().len();
        self.over_budget = self.bytes > budget;
    }

    /// `fit`, then the review pack's last cuts until the rendering fits `budget`: the doc, the
    /// scenarios, then every fenced body left (purpose, sources, hunks) cut to one length.
    fn squeeze(&mut self, budget: usize) {
        self.fit(budget);
        let fits = |p: &Pack| p.render().len() <= budget;
        if !fits(self) && self.doc.take().is_some() {
            self.dropped.push("doc".into());
        }
        if !fits(self) && self.scenarios.take().is_some() {
            self.dropped.push("scenarios".into());
        }
        if !fits(self) {
            let start = self.bodies().map(|b| b.len()).max().unwrap_or(0);
            let mut cap = start;
            while !fits(self) && cap > 0 {
                cap /= 2;
                for body in self.bodies() {
                    let mut end = cap.min(body.len());
                    while !body.is_char_boundary(end) {
                        end -= 1;
                    }
                    body.truncate(end);
                }
            }
            if cap < start {
                self.dropped
                    .push(format!("fenced bodies cut to {cap} bytes"));
            }
        }
        self.bytes = self.render().len();
        self.over_budget = self.bytes > budget;
    }

    /// The fenced bodies a cut can shorten: the purpose, anchor sources, and hunks.
    fn bodies(&mut self) -> impl Iterator<Item = &mut String> {
        let sources = self.anchors.iter_mut().flatten();
        let hunks = self.hunks.iter_mut().flatten();
        self.purpose
            .iter_mut()
            .chain(sources.filter_map(|a| a.source.as_mut()))
            .chain(hunks.filter_map(|h| h.diff.as_mut()))
    }
}

impl Human for Pack {
    fn render(&self) -> String {
        Pack::render(self)
    }
}

pub fn run(
    g: &Global,
    unit: Option<&str>,
    role: Role,
    diff: bool,
    task: Option<&str>,
) -> Result<ExitCode> {
    let (cwd, ws) = here(g)?;
    let repo = ws.current_repo(&cwd)?;
    let task = match task {
        Some(t) => t.to_string(),
        None => git::task_text(&repo.root)?,
    };
    if role == Role::Reviewer {
        if let Some(u) = unit {
            bail!(usage(
                format!("the reviewer's pack takes no unit, got `{u}`"),
                "stewards pack --role reviewer",
            ));
        }
        let (snapshot, task, due) = crate::next::review_due(repo, Some(&task))?;
        emit(&review(repo, &due, &task, &snapshot)?, g.human);
        return Ok(ExitCode::SUCCESS);
    }
    let snapshot = git::snapshot(&repo.root)?;
    emit(&pack(repo, unit, role, &task, diff, &snapshot)?, g.human);
    Ok(ExitCode::SUCCESS)
}

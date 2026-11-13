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
    let source = if is_excluded(exclude, &a.path) {
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
        source,
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
    assemble(repo, unit, role, task, diff, snapshot, None)
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
        Some((n, dev)),
    )
}

fn assemble(
    repo: &Repo,
    unit: Option<&str>,
    role: Role,
    task: &str,
    diff: bool,
    snapshot: &str,
    dev: Option<(usize, &Task)>,
) -> Result<Pack> {
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
                p.scenarios = Some(test.map_or(vec![], |t| t.scenarios.clone()));
                p.test_command = test_command;
                p.test_commands = Some(test_commands(&loaded.repo.commands));
                p.doc = Some(card.and_then(|c| c.doc.clone()).unwrap_or_default());
            }
            Some(card.map_or(vec![], |c| c.anchors.clone()))
        }
        Role::Tester => {
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
                    c.gaps
                        .iter()
                        .find(|g| g.kind == GapKind::Bug && g.severity.is_none())
                })
                .cloned();
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

/// The review brief's pack: the reviewer's section of each unit of `due` (in units.toml order),
/// added while the whole rendering fits `REVIEW_BUDGET`. The first unit is always in; alone over
/// the budget, it is shrunk by `squeeze` and no other unit joins it. From the first unit that
/// does not fit on, the units are `next_round`. The files changed in units not due are listed
/// once.
#[derive(Debug, Clone, Serialize)]
pub struct ReviewPack {
    pub budget: usize,
    pub bytes: usize,
    pub units: Vec<Pack>,
    pub other_changed_files: Vec<String>,
    pub next_round: Vec<String>,
}

pub fn review(repo: &Repo, due: &[DueUnit], task: &str, snapshot: &str) -> Result<ReviewPack> {
    let loaded = repo.loaded()?;
    let router = Router::new(&loaded.units, &loaded.repo.docs_path)?;
    let entries = git::ls_tree(&repo.root, snapshot)?;
    let names: Vec<&str> = due.iter().map(|(u, _)| u.as_str()).collect();
    let other = other_changed(&repo.root, snapshot, &entries, &router, loaded, &names)?;
    let mut r = ReviewPack {
        budget: REVIEW_BUDGET,
        bytes: 0,
        units: vec![],
        other_changed_files: other,
        next_round: vec![],
    };
    // what a unit section may take: the budget less the rest and the newline joining them
    let room = |r: &ReviewPack| REVIEW_BUDGET.saturating_sub(r.render().len() + 1);
    for (i, (unit, reasons)) in due.iter().enumerate() {
        let mut p = assemble(repo, Some(unit), Role::Reviewer, task, true, snapshot, None)?;
        p.due_because = Some(reasons.clone());
        p.bytes = p.render().len();
        if p.bytes <= room(&r) {
            r.units.push(p);
            continue;
        }
        let alone = r.units.is_empty();
        r.next_round = names[i + usize::from(alone)..]
            .iter()
            .map(|u| u.to_string())
            .collect();
        if alone {
            p.squeeze(room(&r));
            r.units.push(p);
        }
        break;
    }
    // the next-round line may push the last units out
    while r.render().len() > REVIEW_BUDGET && r.units.len() > 1 {
        let last = r.units.pop().and_then(|p| p.unit).unwrap_or_default();
        r.next_round.insert(0, last);
    }
    r.bytes = r.render().len();
    Ok(r)
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
        if !self.next_round.is_empty() {
            out.push(format!(
                "# Next round\n\nDue, but over this pack's budget; not in this review: {}\n",
                one_line(&self.next_round.join(", "))
            ));
        }
        out.join("\n")
    }
}

impl Human for ReviewPack {
    fn render(&self) -> String {
        ReviewPack::render(self)
    }
}

/// The window around the match at file line `line`, cut to half its reach on each side.
fn halve(source: &str, line: u32) -> String {
    let lines: Vec<&str> = source.lines().collect();
    // source_window starts WINDOW lines above the match, or at the file's top.
    let at = (line as usize).saturating_sub(1).min(WINDOW);
    let end = (at + WINDOW / 2).min(lines.len().saturating_sub(1));
    lines
        .get(at.saturating_sub(WINDOW / 2)..=end)
        .map_or_else(|| source.to_string(), |l| l.join("\n"))
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

fn fenced(lang: &str, body: &str) -> String {
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
                let line = a.line.map_or("dead".into(), |l| format!("line {l}"));
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
        out
    }

    /// Shrinks the pack in the fixed order until its rendering fits `budget`.
    pub fn fit(&mut self, budget: usize) {
        self.budget = budget;
        let fits = |p: &Pack| p.render().len() <= budget;
        let has_source = |p: &Pack| p.anchors.iter().flatten().any(|a| a.source.is_some());
        if !fits(self) && has_source(self) && !self.dropped.iter().any(|d| d == HALVED) {
            for a in self.anchors.iter_mut().flatten() {
                if let (Some(source), Some(line)) = (&a.source, a.line) {
                    a.source = Some(halve(source, line));
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

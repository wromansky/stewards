// The context pack and the `pack` command.
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Result, bail};
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::Serialize;

use crate::anchors::{self, Resolution};
use crate::git;
use crate::model::{Anchor, CommandEntry, Decision, Doc, Flow, Gap, Interface, Role, Scenario};
use crate::output::{CliError, Global, Human, emit, not_implemented};
use crate::route::Router;
use crate::workspace::{Repo, discover};

/// Lines of source on each side of an anchor's match.
const WINDOW: usize = 20;
const HUNK_LINES: usize = 40;
const HALVED: &str = "source windows halved";

#[derive(Debug, Clone, Serialize)]
pub struct Pack {
    pub unit: Option<String>,
    pub role: Role,
    pub task: String,
    pub budget: usize,
    pub bytes: usize,
    pub over_budget: bool,
    pub purpose: Option<String>,
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

fn window(root: &Path, a: Anchor, exclude: &GlobSet, dropped: &mut Vec<String>) -> PackAnchor {
    let line = match anchors::resolve(root, &a) {
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

pub fn pack(repo: &Repo, unit: Option<&str>, role: Role, task: &str, diff: bool) -> Result<Pack> {
    match (role, unit) {
        (Role::Architect, Some(u)) => bail!(usage(
            format!("the architect's pack takes no unit, got `{u}`"),
            "stewards pack --role architect",
        )),
        (Role::Architect, None) | (_, Some(_)) => {}
        (_, None) => bail!(usage(
            format!("the {role:?} pack needs a unit").to_lowercase(),
            "stewards pack <unit> --role <role>",
        )),
    }
    let loaded = repo.loaded.as_ref().ok_or_else(|| {
        CliError::new(
            "no_stewards_dir",
            format!("no .stewards/ in {}", repo.root.display()),
            "stewards init",
        )
    })?;
    let root = &repo.root;
    let b = &loaded.repo.budgets;
    let budget = match role {
        Role::Developer => b.developer,
        Role::Maintainer => b.maintainer,
        Role::Tester => b.tester,
        Role::Documenter => b.documenter,
        Role::Validator => b.validator,
        Role::Architect => b.architect,
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
        budget,
        bytes: 0,
        over_budget: false,
        purpose: None,
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
                })
                .collect(),
        );
        p.flows = Some(loaded.repo.flows.clone());
        p.decisions = Some(loaded.repo.decisions.clone());
        p.fit(budget);
        return Ok(p);
    };

    let def = loaded
        .units
        .iter()
        .find(|u| u.name == name)
        .ok_or_else(|| {
            let names: Vec<_> = loaded.units.iter().map(|u| u.name.as_str()).collect();
            CliError::new(
                "unknown_unit",
                format!("no unit named `{name}`; units: {}", names.join(", ")),
                "one of the listed units",
            )
        })?;
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
            not_implemented("pack --role developer")?;
            unreachable!("not_implemented always errs")
        }
        Role::Maintainer => {
            p.purpose = Some(def.purpose.clone());
            p.gaps = Some(card.map_or(vec![], |c| c.gaps.clone()));
            p.flows = Some(flows);
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
            p.test_commands = Some(
                loaded
                    .repo
                    .commands
                    .test
                    .iter()
                    .map(|c| match c {
                        CommandEntry::Plain(cmd) | CommandEntry::Timed { cmd, .. } => cmd.clone(),
                    })
                    .collect(),
            );
            None
        }
        Role::Architect => unreachable!("the architect has no unit"),
    };
    p.interface = Some(interface);

    let stamped = card
        .and_then(|c| c.reviewed.as_ref())
        .map(|r| r.tree.as_str())
        .filter(|tree| git::has_object(root, tree));
    if diff
        && matches!(role, Role::Maintainer | Role::Tester)
        && let Some(tree) = stamped
    {
        let snapshot = git::snapshot(root)?;
        let router = Router::new(&loaded.units, &loaded.repo.docs_path)?;
        let (owned, other): (Vec<String>, Vec<String>) = git::diff_names(root, tree, &snapshot)?
            .into_iter()
            .filter(|f| router.owner(f).is_some())
            .partition(|f| router.owner(f) == Some(name));
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
                Some(git::diff_file(root, tree, &snapshot, path)?)
            };
            hunks.push(Hunk {
                path: path.clone(),
                diff,
            });
        }
        p.hunks = Some(hunks);
        p.changed_files = Some(owned);
        if role == Role::Maintainer {
            p.other_changed_files = Some(other);
        }
    }
    p.anchors = anchor_specs.map(|specs| {
        specs
            .into_iter()
            .map(|a| window(root, a, &exclude, &mut p.dropped))
            .collect()
    });
    p.fit(budget);
    Ok(p)
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
            .map(|c| c.replace('|', "\\|").replace('\n', " "))
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

fn paths_table(paths: &[String]) -> String {
    table(&["path"], paths.iter().map(|p| vec![p.clone()]).collect())
}

impl Pack {
    /// The --human markdown; its length is what the budget measures.
    pub fn render(&self) -> String {
        let role = format!("{:?}", self.role).to_lowercase();
        let mut out = format!(
            "# Pack: {} ({role})\n\nTask: {}\n",
            self.unit.as_deref().unwrap_or("repo"),
            self.task
        );
        let mut section = |title: &str, body: String| {
            out += &format!("\n## {title}\n\n{body}");
        };
        if let Some(purpose) = &self.purpose {
            section("Purpose", format!("{purpose}\n"));
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
                body += &format!("### `{}::{}` ({line})\n\n", a.path, a.name);
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
            section("Test command", format!("`{cmd}`\n"));
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
                    ]
                })
                .collect();
            section("Units", table(&["name", "purpose", "interface ids"], rows));
        }
        if let Some(doc) = &self.doc {
            section(
                "Doc",
                format!(
                    "### Overview\n\n{}\n\n### How it works\n\n{}\n\n### How to change\n\n{}\n",
                    doc.overview, doc.how_it_works, doc.how_to_change
                ),
            );
        }
        if let Some(list) = &self.changed_files {
            section("Changed files", paths_table(list));
        }
        if let Some(list) = &self.hunks {
            let mut body = String::new();
            for h in list {
                body += &format!("### {}\n\n", h.path);
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
    let cwd = std::env::current_dir()?.canonicalize()?;
    let ws = discover(&cwd, g.repo.as_deref())?;
    // As for `stamp`: the repo around the current directory, else the only one.
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
    let task = match task {
        Some(t) => t.to_string(),
        None => git::task_text(&repo.root)?,
    };
    emit(&pack(repo, unit, role, &task, diff)?, g.human);
    Ok(ExitCode::SUCCESS)
}

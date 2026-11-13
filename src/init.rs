// `init` step 1: propose units from repository facts (no LLM) and write the starting files.
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::process::ExitCode;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::check::one_line;
use crate::launch::{self, Launcher, Outcome};
use crate::llm::types::UnitProposal;
use crate::llm::{self, Mode};
use crate::model::{self, Role, SCHEMA, UnitDef};
use crate::next::{self, Job};
use crate::output::{CliError, Global, Human, emit, not_implemented, with_note};
use crate::route::Router;
use crate::submit::{self, Submitted};
use crate::workspace::{self, Repo, Workspace};
use crate::{brief, git, guide};

/// Commits of history that co-change is counted over.
const COMMITS: usize = 500;
/// Two candidates merge when commits touching both over commits touching either reach this.
const MERGE_RATIO: f64 = 0.6;
/// A candidate with fewer files merges into its most co-changed sibling.
const MIN_FILES: usize = 3;
/// More units than this in a root keeps the largest and folds the rest into `<root>-misc`.
const MAX_UNITS: usize = 8;
/// A dir with a larger share of generated files is skipped.
const GENERATED_SHARE: f64 = 0.6;
const SKIP_DIRS: [&str; 5] = ["vendor", "node_modules", "target", "dist", "build"];
const SOURCE_CONTAINERS: [&str; 6] = ["src", "lib", "app", "pkg", "cmd", "internal"];
const TEST_DIRS: [&str; 4] = ["tests", "test", "spec", "__tests__"];
const IGNORE_LINES: [&str; 2] = [".stewards/briefs/", ".stewards/telemetry/"];
/// Step 1 writes these; either existing means the repo is initialized.
const STARTING_FILES: [&str; 2] = [".stewards/units.toml", ".stewards/repo.toml"];
const ARCHITECT_TASK: &str =
    "name and describe each proposed unit; propose merges or splits only where a boundary is wrong";
const FIRST_REVIEW_TASK: &str = "first review: read the unit's files, draft its card, and approve";

/// `Some(test command)` for a manifest file name (empty when it implies none), `None` otherwise.
fn manifest(name: &str) -> Option<&'static str> {
    Some(match name {
        "Cargo.toml" => "cargo test",
        "package.json" => "npm test",
        "pyproject.toml" | "setup.py" => "pytest",
        "go.mod" => "go test ./...",
        "Gemfile" => "bundle exec rspec",
        "pom.xml" => "mvn test",
        "main.tf" => "",
        n if n.ends_with(".csproj") || n.ends_with(".sln") => "dotnet test",
        n if n.starts_with("build.gradle") => "gradle test",
        _ => return None,
    })
}

fn is_generated(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.ends_with(".lock")
        || name.contains(".min.")
        || name.ends_with("_pb2.py")
        || name.contains(".generated.")
}

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

fn under(path: &str, dir: &str) -> bool {
    path.strip_prefix(dir)
        .is_some_and(|rest| rest.starts_with('/'))
}

fn sanitize(s: &str) -> String {
    let s: String = s
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    match s.trim_start_matches(['-', '_']) {
        "" => "unit".into(),
        s => s.into(),
    }
}

/// A valid, unused unit name: `name`, else prefixed with the root name, else numbered.
fn unique_name(name: &str, root_name: &str, taken: &mut HashSet<String>) -> String {
    let mut pick = sanitize(name);
    if taken.contains(&pick) {
        pick = sanitize(&format!("{root_name}-{pick}"));
    }
    let stem = pick.clone();
    let mut n = 2;
    while taken.contains(&pick) {
        pick = format!("{stem}-{n}");
        n += 1;
    }
    taken.insert(pick.clone());
    pick
}

/// A top-level source dir of a root, with the test dirs that mirror its name.
struct Candidate {
    base: String,
    /// Repo-relative.
    dirs: Vec<String>,
    files: usize,
}

/// The candidates of one root and the repo-relative dirs skipped as mostly generated.
/// `files` are the repo-relative paths of the root.
fn candidates(root: &str, files: &[&str]) -> (Vec<Candidate>, Vec<String>) {
    let rel: Vec<&str> = files
        .iter()
        .map(|f| f.strip_prefix(root).unwrap_or(f).trim_start_matches('/'))
        .collect();
    let mut dirs = BTreeSet::new();
    for r in &rel {
        let parts: Vec<&str> = r.split('/').collect();
        if SOURCE_CONTAINERS.contains(&parts[0]) && parts.len() > 2 {
            dirs.insert(format!("{}/{}", parts[0], parts[1]));
        }
    }
    if dirs.is_empty() {
        for r in &rel {
            let top = r.split('/').next().unwrap_or_default();
            if r.contains('/') && !TEST_DIRS.contains(&top) && !top.starts_with('.') {
                dirs.insert(top.to_string());
            }
        }
    }
    let count = |dir: &str| rel.iter().filter(|r| under(r, dir)).count();
    let mut kept = Vec::new();
    let mut skipped = Vec::new();
    for dir in dirs {
        let base = dir.rsplit('/').next().unwrap_or(&dir).to_string();
        let mut own = vec![dir.clone()];
        own.extend(
            TEST_DIRS
                .iter()
                .map(|t| format!("{t}/{base}"))
                .filter(|t| count(t) > 0),
        );
        // judged on the dir's own files; mirrored tests would dilute the share
        let generated = rel
            .iter()
            .filter(|r| under(r, &dir) && is_generated(r))
            .count();
        if generated as f64 > GENERATED_SHARE * count(&dir) as f64 {
            skipped.push(join(root, &dir));
            continue;
        }
        kept.push(Candidate {
            base,
            dirs: own.iter().map(|d| join(root, d)).collect(),
            files: own.iter().map(|d| count(d)).sum(),
        });
    }
    (kept, skipped)
}

/// Globs for the files not under a skipped dir; one `dir/**` for each untouched subtree.
fn globs_outside(prefix: &str, files: &[&str], skipped: &[String]) -> Vec<String> {
    let mut children: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for f in files {
        let rest = f.strip_prefix(prefix).unwrap_or(f).trim_start_matches('/');
        let (head, _) = rest.split_once('/').unwrap_or((rest, ""));
        children.entry(head).or_default().push(f);
    }
    let mut out = Vec::new();
    for (name, below) in children {
        let path = join(prefix, name);
        if skipped.contains(&path) {
            continue;
        }
        if below == [path.as_str()] {
            out.push(path);
        } else if skipped.iter().any(|s| under(s, &path)) {
            out.extend(globs_outside(&path, &below, skipped));
        } else {
            out.push(format!("{path}/**"));
        }
    }
    out
}

/// Groups of candidate indices after the co-change merges, ordered by their first member.
/// `commits` holds, per commit, the candidates it touched.
fn merge_groups(cands: &[Candidate], commits: &[BTreeSet<usize>]) -> Vec<Vec<usize>> {
    let n = cands.len();
    let mut label: Vec<usize> = (0..n).collect();
    for a in 0..n {
        for b in a + 1..n {
            let both = commits
                .iter()
                .filter(|c| c.contains(&a) && c.contains(&b))
                .count();
            let either = commits
                .iter()
                .filter(|c| c.contains(&a) || c.contains(&b))
                .count();
            if either > 0 && both as f64 / either as f64 >= MERGE_RATIO {
                let (keep, gone) = (label[a].min(label[b]), label[a].max(label[b]));
                label
                    .iter_mut()
                    .filter(|l| **l == gone)
                    .for_each(|l| *l = keep);
            }
        }
    }
    let mut by_label: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (i, l) in label.into_iter().enumerate() {
        by_label.entry(l).or_default().push(i);
    }
    let mut groups: Vec<Vec<usize>> = by_label.into_values().collect();
    let files = |g: &[usize]| g.iter().map(|&i| cands[i].files).sum::<usize>();
    while groups.len() > 1 {
        let Some(small) = (0..groups.len())
            .filter(|&i| files(&groups[i]) < MIN_FILES)
            .min_by_key(|&i| files(&groups[i]))
        else {
            break;
        };
        let co = |j: usize| {
            let touches = |c: &BTreeSet<usize>, g: &[usize]| g.iter().any(|i| c.contains(i));
            commits
                .iter()
                .filter(|c| touches(c, &groups[small]) && touches(c, &groups[j]))
                .count()
        };
        let target = (0..groups.len())
            .filter(|&j| j != small)
            .max_by_key(|&j| (co(j), files(&groups[j]), Reverse(j)))
            .expect("another group exists");
        let moved = groups.remove(small);
        let target = if target > small { target - 1 } else { target };
        groups[target].extend(moved);
        groups[target].sort();
    }
    groups
}

#[derive(Serialize)]
struct Proposed {
    name: String,
    paths: Vec<String>,
}

/// The units of one root. `files` are its repo-relative paths, `commits` the repo's recent history.
fn root_units(
    root: &str,
    root_name: &str,
    files: &[&str],
    commits: &[Vec<String>],
    taken: &mut HashSet<String>,
) -> Vec<Proposed> {
    let (cands, skipped) = candidates(root, files);
    let touched: Vec<BTreeSet<usize>> = commits
        .iter()
        .map(|commit| {
            commit
                .iter()
                .flat_map(|f| {
                    cands
                        .iter()
                        .position(|c| c.dirs.iter().any(|d| under(f, d)))
                })
                .collect()
        })
        .collect();
    let mut groups = merge_groups(&cands, &touched);
    if groups.len() < 2 {
        let paths = if skipped.is_empty() {
            vec![join(root, "**")]
        } else {
            globs_outside(root, files, &skipped)
        };
        if paths.is_empty() {
            return vec![];
        }
        return vec![Proposed {
            name: unique_name(root_name, root_name, taken),
            paths,
        }];
    }
    let size = |g: &[usize]| g.iter().map(|&i| cands[i].files).sum::<usize>();
    let mut rest = Vec::new();
    if groups.len() > MAX_UNITS {
        let mut order: Vec<usize> = (0..groups.len()).collect();
        order.sort_by_key(|&i| (Reverse(size(&groups[i])), i));
        let mut drop: Vec<usize> = order.split_off(MAX_UNITS);
        drop.sort_by(|a, b| b.cmp(a));
        for i in drop {
            rest.extend(groups.remove(i));
        }
    }
    rest.sort();
    let paths = |g: &[usize]| -> Vec<String> {
        g.iter()
            .flat_map(|&i| cands[i].dirs.iter().map(|d| format!("{d}/**")))
            .collect()
    };
    let mut units: Vec<Proposed> = groups
        .iter()
        .map(|g| {
            let biggest = g
                .iter()
                .max_by_key(|&&i| (cands[i].files, Reverse(i)))
                .expect("a group has members");
            Proposed {
                name: unique_name(&cands[*biggest].base, root_name, taken),
                paths: paths(g),
            }
        })
        .collect();
    if !rest.is_empty() {
        units.push(Proposed {
            name: unique_name(&format!("{root_name}-misc"), root_name, taken),
            paths: paths(&rest),
        });
    }
    units
}

#[derive(Serialize)]
struct UnitsOut<'a> {
    unit: &'a [UnitDef],
}

#[derive(Serialize)]
struct RepoOut<'a> {
    schema: u32,
    name: &'a str,
    test_roots: &'a [String],
    commands: CommandsOut<'a>,
}

#[derive(Serialize)]
struct CommandsOut<'a> {
    test: &'a [String],
}

#[derive(Serialize)]
struct Init {
    units: Vec<Proposed>,
    unowned: Vec<String>,
    wrote: Vec<String>,
}

impl Human for Init {
    fn render(&self) -> String {
        let mut out = String::from("# stewards init\n\n| unit | paths |\n|---|---|\n");
        for u in &self.units {
            out += &format!("| {} | {} |\n", u.name, u.paths.join(", "));
        }
        out += "\nWrote:\n";
        for w in &self.wrote {
            out += &format!("- {w}\n");
        }
        if !self.unowned.is_empty() {
            out += "\nNo unit owns:\n";
            for p in &self.unowned {
                out += &format!("- {p}\n");
            }
        }
        out.trim_end().to_string()
    }
}

/// Appends the `.stewards` ignore lines that are missing; returns whether it wrote.
fn append_gitignore(repo: &Repo) -> Result<bool> {
    let path = repo.root.join(".gitignore");
    let old = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| path.display().to_string()),
    };
    // `/.stewards/briefs` ignores the same directory as `.stewards/briefs/`
    let bare = |l: &str| {
        let l = l.trim();
        let l = l.strip_prefix('/').unwrap_or(l);
        l.strip_suffix('/').unwrap_or(l).to_string()
    };
    let missing: Vec<&str> = IGNORE_LINES
        .into_iter()
        .filter(|l| !old.lines().any(|x| bare(x) == bare(l)))
        .collect();
    if missing.is_empty() {
        return Ok(false);
    }
    let mut new = old.clone();
    if !new.is_empty() && !new.ends_with('\n') {
        new.push('\n');
    }
    new += &missing.join("\n");
    new.push('\n');
    repo.write(".gitignore", new.as_bytes())?;
    Ok(true)
}

pub fn run(
    g: &Global,
    draft: bool,
    run: Option<&launch::Options>,
    hooks: bool,
    ci: bool,
) -> Result<ExitCode> {
    for (set, flag) in [(hooks, "--hooks"), (ci, "--ci")] {
        if set {
            return not_implemented(&format!("init {flag}"));
        }
    }
    if run.is_some() {
        if let Some(code) = launch::guard(g, "init --draft --run") {
            return Ok(code);
        }
        if g.direct {
            return Err(CliError::new(
                "usage",
                "init --draft --run launches sessions and --direct calls the model itself; pick one",
                "run `stewards init --help`",
            )
            .into());
        }
    }
    let (cwd, ws) = workspace::here(g)?;
    let repo = ws.current_repo(&cwd)?;
    let initialized = STARTING_FILES.iter().any(|f| repo.root.join(f).exists());
    // --draft continues an initialized repo; plain init only starts one
    let step1 = if draft && initialized {
        None
    } else {
        Some(propose(repo)?)
    };
    if !draft {
        emit(&step1.expect("plain init ran step 1"), g.human);
        return Ok(ExitCode::SUCCESS);
    }
    let launcher = run
        .map(|o| Launcher::new(o, "init --draft --run"))
        .transpose()?;
    draft_jobs(g, ws.is_workspace, repo, step1, launcher.as_ref())
}

/// The last path segment of a remote URL without a trailing `.git`: `widgets` for
/// `git@host:acme/widgets.git` or `https://host/acme/widgets/`; `None` when that is empty.
fn repo_name_of(url: &str) -> Option<String> {
    let last = url.trim_end_matches('/').rsplit(['/', ':']).next()?;
    let name = last.strip_suffix(".git").unwrap_or(last);
    (!name.is_empty()).then(|| name.to_string())
}

/// Step 1: proposes the units and writes the starting files.
fn propose(repo: &Repo) -> Result<Init> {
    let root = repo.root.canonicalize()?;
    // repo.toml too: rewriting it would drop the user's flows, decisions, and budgets
    for file in STARTING_FILES {
        if root.join(file).exists() {
            return Err(CliError::new(
                "already_initialized",
                format!("{file} already exists in {}", root.display()),
                format!("edit {file} by hand; init only starts a repo"),
            )
            .into());
        }
    }
    // a link at any path written below is refused before the first write, so no rerun is blocked
    for rel in [
        ".stewards/units.toml",
        ".stewards/repo.toml",
        ".gitignore",
        "AGENTS.md",
    ] {
        workspace::safe_path(&repo.root, rel, false)?;
    }

    let tracked = git::ls_files(&root)?;
    let files: Vec<&str> = tracked
        .iter()
        .map(String::as_str)
        .filter(|f| {
            f.rsplit_once('/')
                .is_none_or(|(dir, _)| !dir.split('/').any(|c| SKIP_DIRS.contains(&c)))
        })
        .collect();
    let mut roots = BTreeSet::new();
    let mut test_commands: Vec<String> = Vec::new();
    for f in &files {
        let (dir, name) = f.rsplit_once('/').unwrap_or(("", f));
        if let Some(cmd) = manifest(name) {
            roots.insert(dir.to_string());
            if !cmd.is_empty() && !test_commands.iter().any(|c| c == cmd) {
                test_commands.push(cmd.to_string());
            }
        }
    }
    if roots.is_empty() {
        roots.insert(String::new());
    }
    let mut by_root: BTreeMap<&str, Vec<&str>> =
        roots.iter().map(|r| (r.as_str(), vec![])).collect();
    for f in &files {
        // the nearest manifest dir wins
        let owner = roots
            .iter()
            .filter(|r| r.is_empty() || under(f, r))
            .max_by_key(|r| r.len());
        if let Some(r) = owner {
            by_root.get_mut(r.as_str()).expect("root listed").push(f);
        }
    }

    let repo_name = git::origin_url(&root)
        .and_then(|url| repo_name_of(&url))
        .unwrap_or_else(|| {
            root.file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
        });
    let commits = git::recent_commit_files(&root, COMMITS)?;
    let mut taken = HashSet::new();
    let mut proposed = Vec::new();
    let mut test_roots = BTreeSet::new();
    for (dir, root_files) in &by_root {
        let root_name = match *dir {
            "" => repo_name.as_str(),
            d => d.rsplit('/').next().unwrap_or(d),
        };
        proposed.extend(root_units(dir, root_name, root_files, &commits, &mut taken));
        for f in root_files {
            let rel = f.strip_prefix(dir).unwrap_or(f).trim_start_matches('/');
            if let Some((top, _)) = rel.split_once('/')
                && TEST_DIRS.contains(&top)
            {
                test_roots.insert(join(dir, top));
            }
        }
    }

    let defs: Vec<UnitDef> = proposed
        .iter()
        .map(|u| UnitDef {
            name: u.name.clone(),
            purpose: String::new(),
            paths: u.paths.clone(),
        })
        .collect();
    let router = Router::new(&defs, "docs/units")?;
    let unowned = tracked
        .iter()
        .filter(|f| router.owner(f).is_none())
        .cloned()
        .collect();

    let test_roots: Vec<String> = test_roots.into_iter().collect();
    repo.write(
        ".stewards/units.toml",
        toml::to_string(&UnitsOut { unit: &defs })?.as_bytes(),
    )?;
    let repo_toml = RepoOut {
        schema: SCHEMA,
        name: &repo_name,
        test_roots: &test_roots,
        commands: CommandsOut {
            test: &test_commands,
        },
    };
    repo.write(
        ".stewards/repo.toml",
        toml::to_string(&repo_toml)?.as_bytes(),
    )?;
    let mut wrote = vec![
        ".stewards/units.toml".to_string(),
        ".stewards/repo.toml".to_string(),
    ];
    if append_gitignore(repo)? {
        wrote.push(".gitignore".into());
    }
    if guide::upsert_block(repo)? {
        wrote.push("AGENTS.md".into());
    }
    Ok(Init {
        units: proposed,
        unowned,
        wrote,
    })
}

/// How adoption finishes: the per-role flow, so each unit's first docs come from its documenter.
/// By hand: the briefs are dispatched, then `next --roles`'s.
const THEN: &str = "after these jobs, run `stewards next --roles` and dispatch every brief it writes until it lists no jobs";
/// With `--run`, which ran these jobs.
const THEN_RUN: &str = "run `stewards review --roles` until it exits 0";

#[derive(Serialize)]
struct Draft {
    #[serde(flatten)]
    init: Option<Init>,
    jobs: Vec<Job>,
    then: &'static str,
    /// What each job's submit printed, with `--direct`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    submitted: Vec<Submitted>,
    /// How each job ended, with `--run`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    launched: Vec<Outcome>,
}

impl Human for Draft {
    fn render(&self) -> String {
        let mut out: Vec<String> = self.init.iter().map(|i| i.render() + "\n").collect();
        out.push("## Jobs\n".into());
        for j in &self.jobs {
            let who = format!("{} {}", j.unit, j.role.as_str());
            out.push(format!("- `{}`: {}", who.trim(), j.reasons.join("; ")));
        }
        out.extend(self.submitted.iter().map(Submitted::render));
        if !self.launched.is_empty() {
            out.push("\n## Launched\n".into());
            out.extend(self.launched.iter().map(Outcome::render));
        }
        out.push(format!("\nThen: {}.", self.then));
        out.join("\n")
    }
}

/// The bare unit name of a job, `None` for the architect.
fn unit_of(job: &Job) -> Option<&str> {
    job.unit.rsplit('/').next().filter(|u| !u.is_empty())
}

/// Cap on each summary or proposal carried in a failed `--direct` run.
const CARRIED_CAP: usize = 300;

/// The failure text for a `--direct` run that stopped: how many jobs were submitted, and what they
/// printed (summaries and unit proposals not applied), which exists nowhere else.
fn failure_note(submitted: &[Submitted], total: usize) -> String {
    let done: Vec<_> = submitted
        .iter()
        .map(|s| (s.summary.as_str(), s.unit_proposals.as_slice()))
        .collect();
    failure_text(&done, total)
}

/// `failure_note` over each submitted job's summary and unapplied proposals.
fn failure_text(submitted: &[(&str, &[UnitProposal])], total: usize) -> String {
    let mut note = format!(
        "{} of {total} init --draft jobs submitted before this one; their results stay",
        submitted.len()
    );
    let carried = |s: &str| one_line(s).chars().take(CARRIED_CAP).collect::<String>();
    for (summary, proposals) in submitted {
        note += &format!("; submitted: {}", carried(summary));
        for u in *proposals {
            note += &format!("; not applied: {}", carried(&submit::proposal_line(u)));
        }
    }
    note
}

/// `init --draft`: the architect's naming job and a first review for every never-stamped unit,
/// briefed for agents, or run through `llm::direct` and submitted with `--direct`.
fn draft_jobs(
    g: &Global,
    is_workspace: bool,
    repo: &Repo,
    init: Option<Init>,
    launcher: Option<&Launcher>,
) -> Result<ExitCode> {
    let loaded = model::load_repo(&repo.root)?;
    let ws = Workspace {
        repos: vec![Repo {
            name: loaded.repo.name.clone(),
            root: repo.root.clone(),
            loaded: Some(loaded),
        }],
        is_workspace,
        selected: true,
    };
    let repo = &ws.repos[0];
    let snapshot = git::snapshot(&repo.root)?;
    let mut jobs = vec![Job {
        unit: String::new(),
        role: Role::Architect,
        task: ARCHITECT_TASK.into(),
        snapshot,
        reasons: vec!["init --draft".into()],
        changed: vec![],
        brief_path: None,
        command: if is_workspace {
            format!(
                "stewards --repo {} brief --role architect --task '{ARCHITECT_TASK}'",
                repo.name
            )
        } else {
            format!("stewards brief --role architect --task '{ARCHITECT_TASK}'")
        },
        debt_added: vec![],
    }];
    jobs.extend(
        next::next(&ws, None, false, true)?
            .jobs
            .into_iter()
            .filter(|j| j.role == Role::Maintainer && j.reasons == ["never reviewed"])
            .map(|j| Job {
                task: FIRST_REVIEW_TASK.into(),
                ..j
            }),
    );
    let mut submitted = vec![];
    if g.direct {
        let texts = jobs
            .iter()
            .map(|j| brief::build(repo, unit_of(j), j.role, Mode::Review, &j.task, &j.snapshot))
            .collect::<Result<Vec<_>>>()?;
        // every guard before the first model call, so a missing model or key writes nothing
        for j in &jobs {
            llm::direct_guards(j.role)?;
        }
        for (j, text) in jobs.iter().zip(&texts) {
            let unit = unit_of(j);
            let done = || -> Result<Submitted> {
                let answer = llm::direct(j.role, Mode::Review, unit.unwrap_or_default(), text)?;
                brief::write_text(repo, unit, j.role, text)?;
                let answer_rel = brief::answer_rel(&brief::rel_path(unit, j.role, None));
                repo.write(&answer_rel, answer.as_bytes())?;
                submit::submit_file(repo, unit, j.role, &repo.root.join(answer_rel))
            };
            let out = done().map_err(|e| with_note(e, failure_note(&submitted, jobs.len())))?;
            submitted.push(out);
        }
    } else {
        for j in &mut jobs {
            let unit = unit_of(j);
            let (path, _) = brief::write(repo, unit, j.role, Mode::Review, &j.task, &j.snapshot)?;
            j.brief_path = Some(path);
        }
    }
    let launched = match launcher {
        Some(l) => jobs
            .iter()
            .map(|j| l.run_job(repo, j.brief_path.as_ref().expect("written above")))
            .collect::<Result<Vec<_>>>()?,
        None => vec![],
    };
    let code = launch::exit_code(&launched);
    emit(
        &Draft {
            init,
            jobs,
            then: if launcher.is_some() { THEN_RUN } else { THEN },
            submitted,
            launched,
        },
        g.human,
    );
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_note_carries_cleaned_capped_summaries_and_unapplied_proposals() {
        let proposals = [UnitProposal {
            name: "auth".into(),
            purpose: "login\nflow".into(),
            paths: vec!["src/auth/**".into()],
        }];
        let long = format!("architect: 0 purposes set\n{}", "x".repeat(400));
        let note = failure_text(&[(&long, &proposals), ("api maintainer: approve", &[])], 3);
        assert!(
            note.starts_with("2 of 3 init --draft jobs submitted before this one"),
            "{note}"
        );
        assert!(!note.contains('\n'), "{note}");
        assert!(
            note.contains("; not applied: unit proposal: auth: login flow; paths src/auth/**"),
            "{note}"
        );
        assert!(
            note.contains("; submitted: api maintainer: approve"),
            "{note}"
        );
        let first = note.split("; submitted: ").nth(1).unwrap();
        let first = first.split("; not applied: ").next().unwrap();
        assert_eq!(first.chars().count(), 300, "{first}");
    }

    #[test]
    fn repo_name_of_takes_the_last_segment_without_dot_git() {
        for (url, name) in [
            ("https://github.com/acme/widgets.git", Some("widgets")),
            ("git@github.com:acme/widgets.git", Some("widgets")),
            ("git@host:widgets", Some("widgets")),
            ("/srv/git/widgets/", Some("widgets")),
            ("https://host/acme/", Some("acme")),
            ("https://host/.git", None),
        ] {
            assert_eq!(repo_name_of(url).as_deref(), name, "{url}");
        }
    }

    #[test]
    fn failure_note_with_nothing_submitted_is_the_count_alone() {
        assert_eq!(
            failure_text(&[], 2),
            "0 of 2 init --draft jobs submitted before this one; their results stay"
        );
    }
}

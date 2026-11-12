// The `submit` command: validate a role's typed answer against its brief header and write it.
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::SystemTime;

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::anchors::{Resolution, resolve};
use crate::brief::{self, Header};
use crate::check::{self, in_snapshot, invisible, one_line};
use crate::docs;
use crate::git;
use crate::harness;
use crate::llm::types::{self, CardPatch, Verdict};
use crate::llm::{self, Answer, FieldError, Mode};
use crate::model::{self, CommandEntry, Commands, GapKind, Loaded, Role, TestCard, UnitCard};
use crate::output::{CliError, Global, Human, emit};
use crate::route::Router;
use crate::telemetry;
use crate::workspace::{Repo, discover, safe_path};

/// Ids and titles.
const ID_CAP: usize = 120;
/// Contracts, anchors, paths, and commands.
const LINE_CAP: usize = 300;
/// Notes, cut, replace_with, gap notes, task changes, and every other prose field but explanations.
const PROSE_CAP: usize = 2000;
const EXPLANATION_CAP: usize = 4000;
/// How much of an explanation submit prints.
const EXPLANATION_SHOWN: usize = 2000;
/// The largest answer file read.
const ANSWER_BYTES: u64 = 256 * 1024;
/// Entries in any one list of an answer.
const LIST_CAP: usize = 200;
/// How much of an error's field and problem is reported.
const ERROR_SHOWN: usize = 200;
/// How much of an error's example is reported.
const EXAMPLE_SHOWN: usize = 300;
/// Errors reported in full; the rest are counted in one last entry.
const ERRORS_SHOWN: usize = 20;
const UNITS_TOML: &str = ".stewards/units.toml";

/// Each field error with the error code its kind of problem maps to.
type Problems = Vec<(&'static str, FieldError)>;

/// Each file a submit may write, with its bytes when submit read it; `None` when it was absent.
pub type Before = Vec<(String, Option<Vec<u8>>)>;

/// Characters a shell gives meaning to: a stored command is plain argv without them, and
/// without a newline.
const SHELL_CHARS: &[char] = &[
    '\n', ';', '&', '|', '<', '>', '$', '`', '\\', '\'', '"', '(', ')', '{', '}', '*', '?', '~',
    '#', '!',
];

#[derive(Serialize, Default)]
pub(crate) struct Submitted {
    summary: String,
    explanation: Option<String>,
    tester_brief: Option<PathBuf>,
    /// The architect's that set no purpose; printed for the user to put in units.toml.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    unit_proposals: Vec<types::UnitProposal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    warning: Option<String>,
    #[serde(flatten)]
    developer: Option<DeveloperReport>,
    #[serde(skip)]
    pulled_anchors: Vec<String>,
}

/// What a developer submit computed from git, beside the summary.
#[derive(Serialize)]
struct DeveloperReport {
    out_of_lane: Vec<String>,
    blocked_on: Option<String>,
}

impl Human for Submitted {
    fn render(&self) -> String {
        let mut out = vec![self.summary.clone()];
        out.extend(self.explanation.clone());
        out.extend(
            self.tester_brief
                .as_ref()
                .map(|p| format!("tester brief: {}", p.display())),
        );
        out.extend(self.unit_proposals.iter().map(|u| {
            one_line(&format!(
                "unit proposal: {}: {}; paths {}",
                u.name,
                u.purpose,
                u.paths.join(", ")
            ))
        }));
        out.extend(self.warning.as_ref().map(|w| format!("warning: {w}")));
        out.join("\n")
    }
}

pub fn run(g: &Global, unit: Option<&str>, role: Role, file: &Path) -> Result<ExitCode> {
    let cwd = std::env::current_dir()?.canonicalize()?;
    let ws = discover(&cwd, g.repo.as_deref())?;
    let out = submit_file(ws.current_repo(&cwd)?, unit, role, file)?;
    emit(&out, g.human);
    Ok(ExitCode::SUCCESS)
}

/// Validates the answer in `file` for the job's brief in `repo`, writes the update, and appends
/// telemetry.
pub(crate) fn submit_file(
    repo: &Repo,
    unit: Option<&str>,
    role: Role,
    file: &Path,
) -> Result<Submitted> {
    // the architect takes no unit and gets ""
    let submit: fn(&Repo, &str, &Header, &str, &Before) -> Result<Submitted> = match role {
        Role::Maintainer => maintainer,
        Role::Tester => tester,
        Role::Documenter => documenter,
        Role::Validator => validator,
        Role::Architect => architect,
        Role::Developer => developer,
    };
    let loaded = repo.loaded()?;
    if unit.is_some() == (role == Role::Architect) {
        let wants = if role == Role::Architect {
            "takes no unit"
        } else {
            "needs a unit"
        };
        bail!(CliError::new(
            "usage",
            format!("submit --role {} {wants}", role.as_str()),
            "run `stewards submit --help`",
        ));
    }
    // The unit names the files submit writes, so only a units.toml name (checked on load) passes.
    if let Some(unit) = unit
        && !loaded.units.iter().any(|u| u.name == unit)
    {
        let names: Vec<_> = loaded.units.iter().map(|u| u.name.as_str()).collect();
        bail!(CliError::new(
            "unknown_unit",
            format!("no unit named `{unit}`; units: {}", names.join(", ")),
            "one of the listed units",
        ));
    }
    // a developer's task number is in its answer file's name
    let n = match unit.filter(|_| role == Role::Developer) {
        Some(u) => Some(task_number(u, file)?),
        None => None,
    };
    let brief_rel = match (unit, n) {
        (Some(u), Some(n)) => format!(".stewards/briefs/{u}-developer-{n}.md"),
        _ => brief::rel_path(unit, role),
    };
    let header = brief::read_header(&repo.root.join(&brief_rel))?;
    check_header(&repo.root, unit, role, n, &header, &brief_rel)?;
    let text = read_answer(file)?;
    // Every file the job may write, read before the repo is loaded again for the job, so a
    // write by another submit after this read is caught before this one writes.
    let mut files = vec![".stewards/repo.toml".to_string()];
    if role == Role::Architect {
        files.push(UNITS_TOML.into());
    }
    if let Some(u) = unit {
        files.push(format!(".stewards/units/{u}.toml"));
        files.push(format!(".stewards/units/{u}.test.toml"));
        if role == Role::Maintainer {
            files.push(tasks_rel(u));
        }
        // a bad docs_path is the documenter's own error, reported after the answer's
        if role == Role::Documenter
            && let Ok(pages) = docs::pages(loaded, Some(u))
        {
            files.extend(pages.into_iter().map(|(rel, _)| rel));
        }
    }
    let before = record(&repo.root, &files)?;
    let repo = &Repo {
        name: repo.name.clone(),
        root: repo.root.clone(),
        loaded: Some(model::load_repo(&repo.root)?),
    };
    let mut out = submit(repo, unit.unwrap_or_default(), &header, &text, &before)?;
    let record = telemetry::Record {
        unit: unit.map(String::from),
        role: role.as_str(),
        mode: match header.mode {
            Mode::Review => "review",
            Mode::Explain => "explain",
        },
        task: header.task.clone(),
        bytes: header.bytes,
        anchors: header.anchors,
        pulled_anchors: std::mem::take(&mut out.pulled_anchors),
        harness: harness::detect().map(|h| h.name),
        out_of_lane: out.developer.as_ref().map(|d| d.out_of_lane.len()),
        timestamp: telemetry::rfc3339(SystemTime::now()),
    };
    if let Err(e) = telemetry::append(repo, &record) {
        out.warning = Some(format!("telemetry not written: {e:#}"));
    }
    Ok(out)
}

/// The `n` of a developer answer file named `<unit>-developer-<n>.answer.json`; `usage` for any
/// other name.
fn task_number(unit: &str, file: &Path) -> Result<usize> {
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    let n = name
        .strip_prefix(&format!("{unit}-developer-"))
        .and_then(|r| r.strip_suffix(".answer.json"))
        .and_then(|d| {
            d.parse::<usize>()
                .ok()
                .filter(|n| *n > 0 && n.to_string() == d)
        });
    n.ok_or_else(|| {
        CliError::new(
            "usage",
            format!(
                "{}: a developer answer file is named `{unit}-developer-<n>.answer.json`, n the task number",
                file.display()
            ),
            "write the answer to the path the developer brief names",
        )
        .into()
    })
}

/// `no_brief` unless the header names this unit, role, and developer task number and a full
/// snapshot tree id: a hand-edited header must not stamp another unit, or stamp a commit or a ref.
fn check_header(
    root: &Path,
    unit: Option<&str>,
    role: Role,
    n: Option<usize>,
    h: &Header,
    rel: &str,
) -> Result<()> {
    let hex = matches!(h.snapshot.len(), 40 | 64)
        && h.snapshot
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    if h.unit.as_deref() != unit
        || h.role != role
        || h.n != n
        || !hex
        || !git::is_tree(root, &h.snapshot)
    {
        let (job, brief) = match unit {
            Some(u) => (format!("{u} {}", role.as_str()), format!("{u} ")),
            None => (role.as_str().to_string(), String::new()),
        };
        bail!(CliError::new(
            "no_brief",
            format!("the header of {rel} is not usable for {job}"),
            format!(
                "write a fresh brief with `stewards next` or `stewards brief {brief}--role {}`",
                role.as_str()
            ),
        ));
    }
    Ok(())
}

/// The answer file's text; `usage` unless it is a regular file of at most `ANSWER_BYTES`.
fn read_answer(file: &Path) -> Result<String> {
    let usage = |why: String| {
        CliError::new(
            "usage",
            format!("{}: {why}", file.display()),
            "pass the answer file the brief names: a regular file of at most 256 KiB",
        )
    };
    // metadata before open: opening a FIFO would block
    let meta = std::fs::metadata(file).map_err(|e| usage(format!("cannot read: {e}")))?;
    if !meta.is_file() || meta.len() > ANSWER_BYTES {
        bail!(usage("not a regular file of at most 256 KiB".into()));
    }
    let mut text = String::new();
    std::fs::File::open(file)
        .and_then(|f| f.take(ANSWER_BYTES + 1).read_to_string(&mut text))
        .map_err(|e| usage(format!("cannot read: {e}")))?;
    if text.len() as u64 > ANSWER_BYTES {
        bail!(usage("not a regular file of at most 256 KiB".into()));
    }
    Ok(text)
}

/// The role's answer, strictly parsed; `answer_parse_failed` with each field error.
fn parse_answer(role: Role, mode: Mode, text: &str) -> Result<Answer> {
    llm::parse(role, mode, text).map_err(|errors| {
        rejected(
            errors
                .into_iter()
                .map(|e| ("answer_parse_failed", e))
                .collect(),
        )
    })
}

/// The unit's card, or an empty one when it has none yet.
fn card_of(loaded: &Loaded, unit: &str) -> UnitCard {
    loaded.cards.get(unit).cloned().unwrap_or_else(|| UnitCard {
        unit: unit.to_string(),
        reviewed: None,
        interface: vec![],
        anchors: vec![],
        gaps: vec![],
        doc: None,
    })
}

fn maintainer(
    repo: &Repo,
    unit: &str,
    header: &Header,
    text: &str,
    before: &Before,
) -> Result<Submitted> {
    let answer = parse_answer(Role::Maintainer, header.mode, text)?;
    let root = &repo.root;
    let loaded = repo.loaded()?;
    // Taken before any write: patch anchors are checked in it and the tester brief packs it.
    let snapshot = git::snapshot(root)?;
    let entries = git::ls_tree(root, &snapshot)?;
    let router = Router::new(&loaded.units, &loaded.repo.docs_path)?;
    let rung = |f: &types::Finding| {
        let rung = serde_json::to_value(&f.rung).expect("a rung serializes");
        rung.as_str().unwrap_or_default().to_string()
    };
    let mut problems = vec![];
    // `tasks` is the developers' task list to write; None removes it.
    let (patch, pulled, notes, tester_needed, tasks) = match &answer {
        Answer::MaintainerReview(a) => {
            let empty = a.findings.is_empty();
            if (a.verdict == Verdict::Approve) != empty {
                let (problem, example) = if empty {
                    (
                        "changes with no findings; a changes verdict needs at least one finding"
                            .to_string(),
                        "\"approve\"",
                    )
                } else {
                    (
                        format!(
                            "approve with {}; an approval has no findings",
                            count(a.findings.len(), "finding")
                        ),
                        "\"changes\"",
                    )
                };
                problems.push((
                    "answer_parse_failed",
                    FieldError {
                        field: "verdict".into(),
                        problem,
                        example: example.into(),
                    },
                ));
            }
            check_list(&mut problems, "findings", a.findings.len());
            for (i, f) in a.findings.iter().enumerate() {
                let p = &mut problems;
                let file = format!("findings[{i}].file");
                check_text(p, &file, &f.file, LINE_CAP, true);
                check_owned(p, &file, &f.file, root, unit, &router);
                check_text(p, &format!("findings[{i}].cut"), &f.cut, PROSE_CAP, false);
                let field = format!("findings[{i}].replace_with");
                check_text(p, &field, &f.replace_with, PROSE_CAP, false);
            }
            // The findings are the one fix task: its lane is the files they name.
            let fix = (a.verdict == Verdict::Changes).then(|| {
                let mut files: Vec<String> = vec![];
                for f in &a.findings {
                    if !files.contains(&f.file) {
                        files.push(f.file.clone());
                    }
                }
                let lines: Vec<String> = a
                    .findings
                    .iter()
                    .map(|f| {
                        one_line(&format!(
                            "{}:{} {}: cut {}; replace with {}",
                            f.file,
                            f.line,
                            rung(f),
                            f.cut,
                            f.replace_with
                        ))
                    })
                    .collect();
                types::Task {
                    title: "fix review findings".into(),
                    change: lines.join("\n"),
                    files,
                    check: None,
                }
            });
            if let Some(t) = &fix {
                check_text(&mut problems, "findings", &t.change, PROSE_CAP, false);
            }
            let tasks = fix.map(|t| vec![t]);
            (
                &a.card_patch,
                &a.pulled_anchors,
                &a.notes,
                a.tester_needed,
                tasks,
            )
        }
        Answer::MaintainerExplain(a) => {
            let p = &mut problems;
            check_text(p, "explanation", &a.explanation, EXPLANATION_CAP, false);
            let commands = &loaded.repo.commands;
            check_tasks(p, &a.tasks, root, unit, &router, commands);
            let tasks = (!a.tasks.is_empty()).then(|| a.tasks.clone());
            (&a.card_patch, &a.pulled_anchors, &a.notes, false, tasks)
        }
        _ => unreachable!("a maintainer answer parses to a maintainer variant"),
    };
    // An anchor must live in a file of this unit in the snapshot: never `.git/`, an ignored
    // file, or another unit's file.
    let alive = |a: &model::Anchor| {
        in_snapshot(&entries, &a.path)
            && router.owner(&a.path) == Some(unit)
            && matches!(resolve(root, a), Resolution::Found { .. })
    };
    check_patch(&mut problems, patch.as_ref(), alive);
    check_common(&mut problems, pulled, notes);
    if !problems.is_empty() {
        return Err(rejected(problems));
    }

    let old = card_of(loaded, unit);
    let mut card = old.clone();
    let patch = patch.clone().unwrap_or_default();
    let patched = patch.interface.is_some() || patch.anchors.is_some() || patch.gaps.is_some();
    if let Some(list) = patch.interface {
        card.interface = list
            .into_iter()
            .map(|i| model::Interface {
                id: i.id,
                anchor: i.anchor,
                contract: i.contract,
            })
            .collect();
    }
    if let Some(list) = patch.anchors {
        card.anchors = list
            .into_iter()
            .map(|a| model::Anchor {
                path: a.path,
                name: a.name,
            })
            .collect();
    }
    if let Some(list) = patch.gaps {
        card.gaps = list
            .into_iter()
            .map(|g| {
                // severity and issue are the tool's: a gap keeps them across a patch, by title
                let kept = old.gaps.iter().find(|o| o.title == g.title);
                model::Gap {
                    kind: match g.kind {
                        types::GapKind::Gap => GapKind::Gap,
                        types::GapKind::Future => GapKind::Future,
                        types::GapKind::Bug => GapKind::Bug,
                    },
                    title: g.title,
                    anchor: g.anchor,
                    note: g.note,
                    repro: g.repro,
                    severity: kept.and_then(|o| o.severity.clone()),
                    issue: kept.and_then(|o| o.issue.clone()),
                }
            })
            .collect();
    }
    let mut changed = vec![];
    changes(
        &mut changed,
        "interface",
        &old.interface,
        &card.interface,
        |i| i.id.clone(),
    );
    changes(&mut changed, "anchor", &old.anchors, &card.anchors, |a| {
        format!("{}::{}", a.path, a.name)
    });
    changes(&mut changed, "gap", &old.gaps, &card.gaps, |g| {
        g.title.clone()
    });
    let card_line = if changed.is_empty() {
        "card unchanged".to_string()
    } else {
        format!("card: {}", changed.join(", "))
    };

    let approve = matches!(&answer, Answer::MaintainerReview(a) if a.verdict == Verdict::Approve);
    if approve {
        // At the brief's snapshot, so a file edited since stays stale.
        let marks = old.reviewed.as_ref();
        card.reviewed = Some(check::reviewed_at(repo, unit, &header.snapshot, marks)?);
    }
    let card_text = if patched || approve {
        Some(toml::to_string(&card)?)
    } else {
        None
    };
    let tester_text = if tester_needed {
        // The tester's pack sees the card as it is about to be written.
        let mut fresh = loaded.clone();
        fresh.cards.insert(unit.to_string(), card.clone());
        let fresh = Repo {
            name: repo.name.clone(),
            root: root.clone(),
            loaded: Some(fresh),
        };
        Some(brief::build(
            &fresh,
            Some(unit),
            Role::Tester,
            Mode::Review,
            &header.task,
            &snapshot,
        )?)
    } else {
        None
    };

    let mut out = Submitted {
        pulled_anchors: pulled.clone(),
        ..Default::default()
    };
    out.summary = match &answer {
        Answer::MaintainerReview(_) if approve => {
            let short: String = header.snapshot.chars().take(7).collect();
            format!("{unit} maintainer: approve; {card_line}; stamped {short}")
        }
        Answer::MaintainerReview(a) => {
            let list: Vec<String> = a
                .findings
                .iter()
                .map(|f| format!("{} {}:{}", rung(f), f.file, f.line))
                .collect();
            let mut findings = count(list.len(), "finding");
            if !list.is_empty() {
                findings += &format!(" ({})", list.join(", "));
            }
            brief::check_root(root)?;
            format!(
                "{unit} maintainer: changes; {findings}; {card_line}; next: stewards -C {} brief {unit} --role developer",
                brief::shell_quote(root)
            )
        }
        Answer::MaintainerExplain(a) => {
            out.explanation = Some(a.explanation.chars().take(EXPLANATION_SHOWN).collect());
            let mut summary = format!(
                "{unit} maintainer: explained; {card_line}; {}",
                count(a.tasks.len(), "task")
            );
            if !a.tasks.is_empty() {
                brief::check_root(root)?;
                summary += &format!(
                    "; next: stewards -C {} brief {unit} --role developer",
                    brief::shell_quote(root)
                );
            }
            summary
        }
        _ => unreachable!("a maintainer answer parses to a maintainer variant"),
    };

    // The writes, last, in one checked step: the card, the task list (and with none, every
    // developer job file of the unit), the tester brief, then the retired job's brief.
    let card_rel = format!(".stewards/units/{unit}.toml");
    let tasks_rel = tasks_rel(unit);
    let mut checked = vec![tasks_rel.as_str()];
    let mut changes = vec![];
    if let Some(text) = card_text {
        checked.push(&card_rel);
        changes.push((card_rel.clone(), Some(text)));
    }
    let tasks_text = tasks
        .as_ref()
        .map(serde_json::to_string_pretty)
        .transpose()?;
    changes.push((tasks_rel.clone(), tasks_text));
    if tasks.is_none() {
        let files = brief::developer_files_past(root, unit, 0)?;
        changes.extend(files.into_iter().map(|rel| (rel, None)));
    }
    if let Some(text) = tester_text {
        let rel = brief::rel_path(Some(unit), Role::Tester);
        out.tester_brief = Some(root.join(&rel));
        changes.push((rel, Some(text)));
        changes.push((format!(".stewards/briefs/{unit}-tester.answer.json"), None));
    }
    let brief_rel = brief::rel_path(Some(unit), Role::Maintainer);
    write_remove_and_retire(repo, before, &checked, &changes, &brief_rel)?;
    Ok(out)
}

/// Writes nothing but telemetry and retires the brief: the lane report is computed from git.
fn developer(
    repo: &Repo,
    unit: &str,
    header: &Header,
    text: &str,
    before: &Before,
) -> Result<Submitted> {
    let Answer::Developer(a) = parse_answer(Role::Developer, header.mode, text)? else {
        unreachable!("a developer answer parses to the developer variant")
    };
    let mut p = vec![];
    check_text(&mut p, "summary", &a.summary, PROSE_CAP, false);
    if let Some(b) = &a.blocked_on {
        check_text(&mut p, "blocked_on", b, PROSE_CAP, false);
    }
    if let Some(c) = &a.check {
        check_text(&mut p, "check.command", &c.command, LINE_CAP, true);
        check_text(&mut p, "check.output", &c.output, PROSE_CAP, false);
    }
    check_common(&mut p, &a.pulled_anchors, &a.notes);
    if !p.is_empty() {
        return Err(rejected(p));
    }

    let root = &repo.root;
    let loaded = repo.loaded()?;
    let router = Router::new(&loaded.units, &loaded.repo.docs_path)?;
    let lanes = read_tasks(repo, unit)?;
    let lane = header.lane.as_deref().unwrap_or_default();
    let changed: Vec<String> = git::diff_names(root, &header.snapshot, &git::snapshot(root)?)?
        .into_iter()
        .filter(|f| router.owner(f) == Some(unit))
        .collect();
    let k = changed.iter().filter(|f| lane.contains(f)).count();
    let out_of_lane: Vec<String> = changed
        .into_iter()
        .filter(|f| !lanes.iter().any(|t| t.files.contains(f)))
        .collect();
    let n = header.n.unwrap_or_default();
    let summary = match &a.blocked_on {
        Some(b) => {
            let b: String = one_line(b).chars().take(LINE_CAP).collect();
            format!("{unit} developer {n}: blocked: {b}")
        }
        None => {
            let check = match &a.check {
                None => "no check",
                Some(c) if c.pass => "check pass",
                Some(_) => "check fail",
            };
            let mut s = format!("{unit} developer {n}: {}; {check}", count(k, "file"));
            if !out_of_lane.is_empty() {
                s += &format!("; out of lane: {}", one_line(&out_of_lane.join(", ")));
            }
            s
        }
    };
    let brief_rel = format!(".stewards/briefs/{unit}-developer-{n}.md");
    write_and_retire(repo, before, &[], &brief_rel)?;
    Ok(Submitted {
        summary,
        developer: Some(DeveloperReport {
            out_of_lane,
            blocked_on: a.blocked_on,
        }),
        pulled_anchors: a.pulled_anchors,
        ..Default::default()
    })
}

fn tester(
    repo: &Repo,
    unit: &str,
    header: &Header,
    text: &str,
    before: &Before,
) -> Result<Submitted> {
    let Answer::Tester(a) = parse_answer(Role::Tester, header.mode, text)? else {
        unreachable!("a tester answer parses to the tester variant")
    };
    let loaded = repo.loaded()?;
    let commands = &loaded.repo.commands;
    let mut card = card_of(loaded, unit);
    let mut tests = loaded.tests.get(unit).cloned().unwrap_or_else(|| TestCard {
        unit: unit.to_string(),
        primary_paths: vec![],
        test_command: None,
        scenarios: vec![],
    });

    let mut p = vec![];
    let interface = |p: &mut Problems, field: String, id: &str| {
        if !card.interface.iter().any(|i| i.id == id) {
            let ids: Vec<&str> = card.interface.iter().map(|i| i.id.as_str()).collect();
            p.push((
                "answer_parse_failed",
                FieldError {
                    field,
                    problem: format!(
                        "`{id}` is not an interface id of unit `{unit}`; ids: {}",
                        ids.join(", ")
                    ),
                    example: format!("\"{}\"", ids.first().unwrap_or(&"create-item")),
                },
            ));
        }
    };
    check_list(&mut p, "scenario_patch", a.scenario_patch.len());
    for (i, s) in a.scenario_patch.iter().enumerate() {
        let f = format!("scenario_patch[{i}]");
        check_text(&mut p, &format!("{f}.id"), &s.id, ID_CAP, true);
        check_text(
            &mut p,
            &format!("{f}.interface"),
            &s.interface,
            ID_CAP,
            true,
        );
        check_text(&mut p, &format!("{f}.given"), &s.given, LINE_CAP, true);
        check_text(&mut p, &format!("{f}.expect"), &s.expect, LINE_CAP, true);
        check_text(&mut p, &format!("{f}.level"), &s.level, ID_CAP, true);
        if let Some(c) = &s.command {
            check_text(&mut p, &format!("{f}.command"), c, LINE_CAP, true);
            check_command(&mut p, &format!("{f}.command"), c, commands);
        }
        interface(&mut p, format!("{f}.interface"), &s.interface);
    }
    check_list(&mut p, "results", a.results.len());
    for (i, r) in a.results.iter().enumerate() {
        let f = format!("results[{i}]");
        check_text(&mut p, &format!("{f}.scenario"), &r.scenario, ID_CAP, true);
        check_text(&mut p, &format!("{f}.output"), &r.output, PROSE_CAP, false);
    }
    check_list(&mut p, "bugs", a.bugs.len());
    for (i, b) in a.bugs.iter().enumerate() {
        let f = format!("bugs[{i}]");
        check_text(&mut p, &format!("{f}.title"), &b.title, ID_CAP, true);
        check_text(&mut p, &format!("{f}.repro"), &b.repro, LINE_CAP, true);
        check_command(&mut p, &format!("{f}.repro"), &b.repro, commands);
        check_text(
            &mut p,
            &format!("{f}.evidence"),
            &b.evidence,
            PROSE_CAP,
            false,
        );
        if let Some(id) = &b.interface {
            check_text(&mut p, &format!("{f}.interface"), id, ID_CAP, true);
            interface(&mut p, format!("{f}.interface"), id);
        }
        // a bug upserts by title, so it may not take the title of a gap of another kind
        if card
            .gaps
            .iter()
            .any(|g| g.title == b.title && g.kind != GapKind::Bug)
        {
            p.push((
                "answer_parse_failed",
                FieldError {
                    field: format!("{f}.title"),
                    problem: "a gap that is not a bug already has this title".into(),
                    example: "\"count off by one\"".into(),
                },
            ));
        }
    }
    check_list(&mut p, "tests_written", a.tests_written.len());
    for (i, t) in a.tests_written.iter().enumerate() {
        check_text(&mut p, &format!("tests_written[{i}]"), t, LINE_CAP, true);
    }
    check_common(&mut p, &a.pulled_anchors, &a.notes);
    if !p.is_empty() {
        return Err(rejected(p));
    }

    for s in &a.scenario_patch {
        let new = model::Scenario {
            id: s.id.clone(),
            interface: s.interface.clone(),
            given: s.given.clone(),
            expect: s.expect.clone(),
            level: s.level.clone(),
            command: s.command.clone(),
        };
        match tests.scenarios.iter_mut().find(|o| o.id == s.id) {
            Some(old) => *old = new,
            None => tests.scenarios.push(new),
        }
    }
    for b in &a.bugs {
        match card.gaps.iter_mut().find(|g| g.title == b.title) {
            Some(g) => {
                g.repro = Some(b.repro.clone());
                g.note = b.evidence.clone();
            }
            None => card.gaps.push(model::Gap {
                kind: GapKind::Bug,
                title: b.title.clone(),
                anchor: None,
                note: b.evidence.clone(),
                repro: Some(b.repro.clone()),
                severity: None,
                issue: None,
            }),
        }
    }
    // At submit time, so the tester's own new test files do not call the tester back.
    if let Some(r) = card.reviewed.as_mut() {
        r.tester = Some(git::snapshot(&repo.root)?);
    }
    let mut writes = vec![];
    if !a.scenario_patch.is_empty() {
        writes.push((
            format!(".stewards/units/{unit}.test.toml"),
            toml::to_string(&tests)?,
        ));
    }
    if !a.bugs.is_empty() || card.reviewed.is_some() {
        writes.push((
            format!(".stewards/units/{unit}.toml"),
            toml::to_string(&card)?,
        ));
    }
    write_and_retire(
        repo,
        before,
        &writes,
        &brief::rel_path(Some(unit), Role::Tester),
    )?;

    let ids = a.scenario_patch.iter().map(|s| s.id.as_str());
    let titles = a.bugs.iter().map(|b| b.title.as_str());
    let passed = a.results.iter().filter(|r| r.pass).count();
    Ok(Submitted {
        summary: format!(
            "{unit} tester: {}; {passed}/{} pass; {}; {} written",
            named(count(ids.len(), "scenario") + " upserted", ids),
            a.results.len(),
            named(count(titles.len(), "bug"), titles),
            count(a.tests_written.len(), "test"),
        ),
        pulled_anchors: a.pulled_anchors,
        ..Default::default()
    })
}

fn documenter(
    repo: &Repo,
    unit: &str,
    header: &Header,
    text: &str,
    before: &Before,
) -> Result<Submitted> {
    let Answer::Documenter(a) = parse_answer(Role::Documenter, header.mode, text)? else {
        unreachable!("a documenter answer parses to the documenter variant")
    };
    let mut p = vec![];
    for (field, value) in [
        ("doc.overview", &a.doc.overview),
        ("doc.how_it_works", &a.doc.how_it_works),
        ("doc.how_to_change", &a.doc.how_to_change),
    ] {
        check_text(&mut p, field, value, PROSE_CAP, false);
        check_doc_markdown(&mut p, field, value);
    }
    check_common(&mut p, &a.pulled_anchors, &a.notes);
    if !p.is_empty() {
        return Err(rejected(p));
    }

    let loaded = repo.loaded()?;
    let mut card = card_of(loaded, unit);
    card.doc = Some(model::Doc {
        overview: a.doc.overview,
        how_it_works: a.doc.how_it_works,
        how_to_change: a.doc.how_to_change,
    });
    if let Some(r) = card.reviewed.as_mut() {
        r.documenter = Some(git::snapshot(&repo.root)?);
    }
    let card_text = toml::to_string(&card)?;
    // The page renders the card as it is about to be written.
    let mut fresh = loaded.clone();
    fresh.cards.insert(unit.to_string(), card);
    let mut writes = vec![(format!(".stewards/units/{unit}.toml"), card_text)];
    writes.extend(docs::pages(&fresh, Some(unit))?);
    write_and_retire(
        repo,
        before,
        &writes,
        &brief::rel_path(Some(unit), Role::Documenter),
    )?;
    Ok(Submitted {
        summary: format!("{unit} documenter: doc updated; {}", writes[1].0),
        pulled_anchors: a.pulled_anchors,
        ..Default::default()
    })
}

fn validator(
    repo: &Repo,
    unit: &str,
    header: &Header,
    text: &str,
    before: &Before,
) -> Result<Submitted> {
    let loaded = repo.loaded()?;
    let mut card = card_of(loaded, unit);
    let title = header.bug.as_deref().unwrap_or_default();
    let Some(at) = card
        .gaps
        .iter()
        .position(|g| g.kind == GapKind::Bug && g.title == title)
    else {
        bail!(CliError::new(
            "no_brief",
            format!(
                "the bug `{}` of the validator brief is no longer on {unit}'s card",
                one_line(title)
            ),
            "write a fresh brief with `stewards next`",
        ));
    };
    let Answer::Validator(a) = parse_answer(Role::Validator, header.mode, text)? else {
        unreachable!("a validator answer parses to the validator variant")
    };
    let mut p = vec![];
    check_text(&mut p, "reason", &a.reason, PROSE_CAP, false);
    check_list(&mut p, "evidence", a.evidence.len());
    for (i, e) in a.evidence.iter().enumerate() {
        check_text(&mut p, &format!("evidence[{i}]"), e, PROSE_CAP, false);
    }
    check_common(&mut p, &a.pulled_anchors, &a.notes);
    if !p.is_empty() {
        return Err(rejected(p));
    }

    let verdict = if a.real {
        let severity = serde_json::to_value(&a.severity)?;
        let severity = severity
            .as_str()
            .expect("a severity serializes to a string");
        card.gaps[at].severity = Some(severity.to_string());
        severity.to_string()
    } else {
        card.gaps.remove(at);
        "not real, removed".to_string()
    };
    let writes = [(
        format!(".stewards/units/{unit}.toml"),
        toml::to_string(&card)?,
    )];
    write_and_retire(
        repo,
        before,
        &writes,
        &brief::rel_path(Some(unit), Role::Validator),
    )?;
    Ok(Submitted {
        summary: format!("{unit} validator: {}: {verdict}", one_line(title)),
        pulled_anchors: a.pulled_anchors,
        ..Default::default()
    })
}

fn architect(
    repo: &Repo,
    _unit: &str,
    header: &Header,
    text: &str,
    before: &Before,
) -> Result<Submitted> {
    let Answer::Architect(a) = parse_answer(Role::Architect, header.mode, text)? else {
        unreachable!("an architect answer parses to the architect variant")
    };
    let root = &repo.root;
    let loaded = repo.loaded()?;
    let entries = git::ls_tree(root, &git::snapshot(root)?)?;
    let mut p = vec![];
    if let Some(plan) = &a.plan {
        check_text(&mut p, "plan.title", &plan.title, ID_CAP, true);
        check_list(&mut p, "plan.steps", plan.steps.len());
        for (i, s) in plan.steps.iter().enumerate() {
            let f = format!("plan.steps[{i}]");
            check_text(&mut p, &format!("{f}.unit"), &s.unit, ID_CAP, true);
            check_text(&mut p, &format!("{f}.change"), &s.change, PROSE_CAP, false);
            check_list(&mut p, &format!("{f}.interfaces"), s.interfaces.len());
            for (j, id) in s.interfaces.iter().enumerate() {
                check_text(&mut p, &format!("{f}.interfaces[{j}]"), id, ID_CAP, true);
            }
            check_text(&mut p, &format!("{f}.check"), &s.check, LINE_CAP, true);
        }
    }
    check_list(&mut p, "flows", a.flows.len());
    for (i, flow) in a.flows.iter().enumerate() {
        let f = format!("flows[{i}]");
        check_text(&mut p, &format!("{f}.name"), &flow.name, ID_CAP, true);
        check_list(&mut p, &format!("{f}.units"), flow.units.len());
        for (j, u) in flow.units.iter().enumerate() {
            let field = format!("{f}.units[{j}]");
            check_text(&mut p, &field, u, ID_CAP, true);
            if !loaded.units.iter().any(|d| &d.name == u) {
                let names: Vec<&str> = loaded.units.iter().map(|d| d.name.as_str()).collect();
                p.push((
                    "answer_parse_failed",
                    FieldError {
                        field,
                        problem: format!(
                            "`{u}` is not a unit in units.toml; units: {}",
                            names.join(", ")
                        ),
                        example: format!("\"{}\"", names.first().unwrap_or(&"api")),
                    },
                ));
            }
        }
        check_text(&mut p, &format!("{f}.entry"), &flow.entry, LINE_CAP, true);
        let alive = model::Anchor::parse(&flow.entry).is_ok_and(|e| {
            in_snapshot(&entries, &e.path) && matches!(resolve(root, &e), Resolution::Found { .. })
        });
        if !alive {
            p.push((
                "answer_dead_anchor",
                FieldError {
                    field: format!("{f}.entry"),
                    problem: "the anchor does not resolve to a whole-word name in a file of \
                              the snapshot"
                        .into(),
                    example: "src/api.rs::create_item".into(),
                },
            ));
        }
        if let Some(c) = &flow.check {
            check_text(&mut p, &format!("{f}.check"), c, LINE_CAP, true);
            check_command(&mut p, &format!("{f}.check"), c, &loaded.repo.commands);
        }
    }
    check_list(&mut p, "decisions", a.decisions.len());
    for (i, d) in a.decisions.iter().enumerate() {
        check_text(
            &mut p,
            &format!("decisions[{i}].title"),
            &d.title,
            ID_CAP,
            true,
        );
        check_text(
            &mut p,
            &format!("decisions[{i}].why"),
            &d.why,
            PROSE_CAP,
            false,
        );
    }
    check_list(&mut p, "unit_proposals", a.unit_proposals.len());
    for (i, u) in a.unit_proposals.iter().enumerate() {
        let f = format!("unit_proposals[{i}]");
        check_text(&mut p, &format!("{f}.name"), &u.name, ID_CAP, true);
        check_text(&mut p, &format!("{f}.purpose"), &u.purpose, LINE_CAP, true);
        check_list(&mut p, &format!("{f}.paths"), u.paths.len());
        for (j, path) in u.paths.iter().enumerate() {
            check_text(&mut p, &format!("{f}.paths[{j}]"), path, LINE_CAP, true);
        }
    }
    check_text(
        &mut p,
        "explanation",
        &a.explanation,
        EXPLANATION_CAP,
        false,
    );
    check_common(&mut p, &a.pulled_anchors, &a.notes);
    if !p.is_empty() {
        return Err(rejected(p));
    }

    let mut writes = vec![];
    if !a.flows.is_empty() || !a.decisions.is_empty() {
        // toml_edit, not a re-serialization: the user's comments and layout in repo.toml stay
        let rel = ".stewards/repo.toml";
        let mut doc: toml_edit::DocumentMut =
            std::fs::read_to_string(safe_path(root, rel, false)?)?.parse()?;
        for f in &a.flows {
            let t = upsert(&mut doc, "flow", "name", &f.name)?;
            t["units"] = toml_edit::value(
                f.units
                    .iter()
                    .map(String::as_str)
                    .collect::<toml_edit::Array>(),
            );
            t["entry"] = toml_edit::value(f.entry.as_str());
            match &f.check {
                Some(c) => t["check"] = toml_edit::value(c.as_str()),
                None => drop(t.remove("check")),
            }
        }
        for d in &a.decisions {
            upsert(&mut doc, "decision", "title", &d.title)?["why"] =
                toml_edit::value(d.why.as_str());
        }
        writes.push((rel.to_string(), doc.to_string()));
    }
    // The one proposal applied: a purpose for an unnamed unit exactly as units.toml has it, which
    // is how `init --draft` names the units; every other proposal is for the user.
    let text = before.iter().find(|(rel, _)| rel == UNITS_TOML);
    let text = text
        .and_then(|(_, bytes)| bytes.as_deref())
        .unwrap_or_default();
    let mut units: toml_edit::DocumentMut = std::str::from_utf8(text)?.parse()?;
    let mut left = vec![];
    let mut set = 0;
    for u in a.unit_proposals {
        let same = loaded
            .units
            .iter()
            .any(|d| d.name == u.name && d.paths == u.paths);
        let table = units
            .get_mut("unit")
            .and_then(|i| i.as_array_of_tables_mut())
            .and_then(|l| {
                l.iter_mut()
                    .find(|t| t.get("name").and_then(|v| v.as_str()) == Some(&u.name))
            })
            .filter(|t| {
                t.get("purpose")
                    .and_then(|v| v.as_str())
                    .is_none_or(str::is_empty)
            });
        match table {
            Some(t) if same => {
                // the old value's spacing and trailing comment stay
                let decor = t
                    .get("purpose")
                    .and_then(|i| i.as_value())
                    .map(|v| v.decor().clone());
                let mut purpose = toml_edit::Value::from(u.purpose.as_str());
                if let Some(decor) = decor {
                    *purpose.decor_mut() = decor;
                }
                t["purpose"] = toml_edit::Item::Value(purpose);
                set += 1;
            }
            _ => left.push(u),
        }
    }
    if set > 0 {
        writes.push((UNITS_TOML.to_string(), units.to_string()));
    }
    write_and_retire(
        repo,
        before,
        &writes,
        &brief::rel_path(None, Role::Architect),
    )?;

    let names = a.flows.iter().map(|f| f.name.as_str());
    let mut summary = format!(
        "architect: {} set; {} for you to review; {}; {}",
        count(set, "purpose"),
        count(left.len(), "unit proposal"),
        named(count(names.len(), "flow") + " upserted", names),
        count(a.decisions.len(), "decision"),
    );
    // simplified: a plan is validated but not stored; the plans stage stores and tracks it
    if a.plan.is_some() {
        summary += "; plan not stored yet";
    }
    Ok(Submitted {
        summary,
        explanation: Some(a.explanation.chars().take(EXPLANATION_SHOWN).collect()),
        unit_proposals: left,
        pulled_anchors: a.pulled_anchors,
        ..Default::default()
    })
}

/// The `[[array]]` table of `doc` whose `key` is `id`, appended when there is none.
fn upsert<'a>(
    doc: &'a mut toml_edit::DocumentMut,
    array: &str,
    key: &str,
    id: &str,
) -> Result<&'a mut toml_edit::Table> {
    let list = doc
        .entry(array)
        .or_insert(toml_edit::Item::ArrayOfTables(Default::default()))
        .as_array_of_tables_mut()
        .ok_or_else(|| {
            CliError::new(
                "invalid_toml",
                format!(
                    ".stewards/repo.toml: `{array}` is not a list of [[{array}]] tables; flows \
                     and decisions must be written as [[flow]] and [[decision]] tables"
                ),
                format!("rewrite `{array}` in .stewards/repo.toml as [[{array}]] tables"),
            )
        })?;
    let at = list
        .iter()
        .position(|t| t.get(key).and_then(|v| v.as_str()) == Some(id));
    let at = at.unwrap_or_else(|| {
        let mut t = toml_edit::Table::new();
        t[key] = toml_edit::value(id);
        list.push(t);
        list.len() - 1
    });
    Ok(list.get_mut(at).expect("the index is in the list"))
}

/// The bytes of each of `rels` under `root` now, `None` for an absent file.
pub fn record(root: &Path, rels: &[String]) -> Result<Before> {
    rels.iter()
        .map(|rel| Ok((rel.clone(), read_if_present(root, rel)?)))
        .collect()
}

fn read_if_present(root: &Path, rel: &str) -> Result<Option<Vec<u8>>> {
    match std::fs::read(safe_path(root, rel, false)?) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        r => Ok(Some(r.with_context(|| format!("read {rel}"))?)),
    }
}

/// `card_changed` unless each of `rels` still has the bytes `before` recorded for it: another
/// submit wrote it since, and writing now would erase that submit's change.
fn check_unchanged(root: &Path, before: &Before, rels: &[&str]) -> Result<()> {
    for rel in rels {
        let then = before
            .iter()
            .find(|(r, _)| r == rel)
            .map(|(_, bytes)| bytes);
        if then != Some(&read_if_present(root, rel)?) {
            bail!(CliError::new(
                "card_changed",
                format!("{rel} changed since this submit read it; nothing was written"),
                "run the same submit again",
            ));
        }
    }
    Ok(())
}

/// Writes each `(rel, text)` in order, then removes the job's brief, through
/// `write_remove_and_retire`.
pub fn write_and_retire(
    repo: &Repo,
    before: &Before,
    writes: &[(String, String)],
    brief_rel: &str,
) -> Result<()> {
    let changes: Vec<(String, Option<String>)> = writes
        .iter()
        .map(|(rel, text)| (rel.clone(), Some(text.clone())))
        .collect();
    let checked: Vec<&str> = writes.iter().map(|(rel, _)| rel.as_str()).collect();
    write_remove_and_retire(repo, before, &checked, &changes, brief_rel)
}

/// Writes each `(rel, Some(text))` and removes each `(rel, None)` in order, then removes the job's
/// brief: the one checked write step of every submit. A link at any of the paths is refused, and
/// a `checked` file changed since `before` was recorded is `card_changed`, before the first
/// change; job files (briefs and their answers) are changed unchecked.
pub fn write_remove_and_retire(
    repo: &Repo,
    before: &Before,
    checked: &[&str],
    changes: &[(String, Option<String>)],
    brief_rel: &str,
) -> Result<()> {
    for rel in changes
        .iter()
        .map(|(rel, _)| rel.as_str())
        .chain([brief_rel])
    {
        safe_path(&repo.root, rel, false)?;
    }
    // simplified: the check and the writes are not atomic, so a write landing between them is
    // still lost; a lock file would close that window if it shows up in practice.
    check_unchanged(&repo.root, before, checked)?;
    let mut written: Vec<&str> = vec![];
    for (rel, text) in changes {
        match text {
            Some(text) => repo.write(rel, text.as_bytes()),
            None => repo.remove(rel),
        }
        .map_err(|e| after_writes(e, &written))?;
        written.push(rel);
    }
    // a submitted job is done: a replay of the same answer finds no brief
    repo.remove(brief_rel)
        .map_err(|e| after_writes(e, &written))
}

/// `e` with the files submit had already written named in its message.
fn after_writes(e: anyhow::Error, written: &[&str]) -> anyhow::Error {
    if written.is_empty() {
        return e;
    }
    let done = format!("submit had already written {}", written.join(", "));
    match e.downcast::<CliError>() {
        Ok(mut err) => {
            err.message = format!("{}; {done}", err.message);
            err.into()
        }
        Err(e) => e.context(done),
    }
}

/// The failed-submit error; its code names the problem when there is only one kind.
fn rejected(problems: Problems) -> anyhow::Error {
    let first = problems.first().map_or("answer_parse_failed", |p| p.0);
    let code = if problems.iter().all(|p| p.0 == first) {
        first
    } else {
        "answer_parse_failed"
    };
    let fix = match code {
        "answer_too_long" => "shorten each field listed to its cap and rerun submit",
        "answer_dead_anchor" => "correct each anchor listed or drop it, then rerun submit",
        "command_not_allowed" => {
            "start each command listed with one of `commands.test` in .stewards/repo.toml, or add it there, and write it as plain argv with no shell characters"
        }
        _ => "fix the fields listed in `errors` and rerun submit",
    };
    let mut err = CliError::new(
        code,
        format!("the answer has {}", count(problems.len(), "problem")),
        fix,
    );
    // field and problem may quote the answer: one line, no invisible characters, capped
    let shown = |s: &str| {
        one_line(s)
            .replace(invisible, " ")
            .chars()
            .take(ERROR_SHOWN)
            .collect::<String>()
    };
    let more = problems.len().saturating_sub(ERRORS_SHOWN);
    err.errors = problems
        .into_iter()
        .take(ERRORS_SHOWN)
        .map(|(_, e)| FieldError {
            field: shown(&e.field),
            problem: shown(&e.problem),
            example: e.example.chars().take(EXAMPLE_SHOWN).collect(),
        })
        .collect();
    if more > 0 {
        err.errors.push(FieldError {
            field: String::new(),
            problem: format!("and {more} more"),
            example: String::new(),
        });
    }
    err.into()
}

/// `answer_too_long` when the list `field` has more than `LIST_CAP` entries.
fn check_list(out: &mut Problems, field: &str, len: usize) {
    if len > LIST_CAP {
        out.push((
            "answer_too_long",
            FieldError {
                field: field.to_string(),
                problem: format!("{len} entries; the cap is {LIST_CAP}"),
                example: format!("at most {LIST_CAP} entries"),
            },
        ));
    }
}

/// Checks the `pulled_anchors` and `notes` every answer carries.
fn check_common(out: &mut Problems, pulled: &[String], notes: &str) {
    check_list(out, "pulled_anchors", pulled.len());
    for (i, a) in pulled.iter().enumerate() {
        check_text(out, &format!("pulled_anchors[{i}]"), a, LINE_CAP, true);
    }
    check_text(out, "notes", notes, PROSE_CAP, false);
}

/// The tool-owned task list a maintainer submit writes for the unit's developers.
fn tasks_rel(unit: &str) -> String {
    format!(".stewards/briefs/{unit}-tasks.json")
}

/// The unit's task list, strictly parsed and checked again against the repo as it is now: the
/// file sits where subagents can write, so what the maintainer submit validated is not trusted.
pub(crate) fn read_tasks(repo: &Repo, unit: &str) -> Result<Vec<types::Task>> {
    let rel = tasks_rel(unit);
    let path = safe_path(&repo.root, &rel, false)?;
    let no_tasks = |message: String| -> anyhow::Error {
        CliError::new(
            "no_tasks",
            message,
            format!(
                "brief the maintainer again with `stewards brief {unit} --role maintainer --task <the change>` and submit its answer with tasks"
            ),
        )
        .into()
    };
    let unusable = |why: &str| no_tasks(format!("{rel} is not a usable task list: {why}"));
    match std::fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(no_tasks(format!("unit `{unit}` has no task list ({rel})")));
        }
        Ok(m) if m.is_file() => {}
        _ => return Err(unusable("not a regular file")),
    }
    let text = std::fs::read_to_string(&path).map_err(|_| unusable("cannot read it"))?;
    // strict: a re-serialization that differs from the file means an unknown or missing field
    let tasks = serde_json::from_str::<Vec<types::Task>>(&text)
        .ok()
        .filter(|t| {
            !t.is_empty()
                && serde_json::from_str::<serde_json::Value>(&text).ok()
                    == serde_json::to_value(t).ok()
        })
        .ok_or_else(|| unusable("not a non-empty JSON list of tasks"))?;
    let loaded = repo.loaded()?;
    let router = Router::new(&loaded.units, &loaded.repo.docs_path)?;
    let mut p = vec![];
    check_tasks(
        &mut p,
        &tasks,
        &repo.root,
        unit,
        &router,
        &loaded.repo.commands,
    );
    match p.first() {
        Some((_, e)) => Err(unusable(&format!("{} fails its check", e.field))),
        None => Ok(tasks),
    }
}

/// Checks a task list for `unit`: caps and characters, at least one file per task, each file
/// plain and owned by `unit`, no file in two lanes, and each check an allowed command.
fn check_tasks(
    out: &mut Problems,
    tasks: &[types::Task],
    root: &Path,
    unit: &str,
    router: &Router,
    commands: &Commands,
) {
    check_list(out, "tasks", tasks.len());
    let mut lanes = std::collections::HashMap::new();
    for (i, t) in tasks.iter().enumerate() {
        let f = format!("tasks[{i}]");
        check_text(out, &format!("{f}.title"), &t.title, ID_CAP, true);
        check_text(out, &format!("{f}.change"), &t.change, PROSE_CAP, false);
        if t.files.is_empty() {
            out.push((
                "answer_parse_failed",
                FieldError {
                    field: format!("{f}.files"),
                    problem: "no files; a task names at least one file it may write".into(),
                    example: "[\"src/api.rs\"]".into(),
                },
            ));
        }
        check_list(out, &format!("{f}.files"), t.files.len());
        for (j, file) in t.files.iter().enumerate() {
            let field = format!("{f}.files[{j}]");
            check_text(out, &field, file, LINE_CAP, true);
            check_owned(out, &field, file, root, unit, router);
            let first = *lanes.entry(file.as_str()).or_insert(i);
            if first != i {
                out.push((
                    "answer_parse_failed",
                    FieldError {
                        field,
                        problem: format!(
                            "`{file}` is also in the lane of tasks[{first}]; no two lanes share a file"
                        ),
                        example: "\"src/api.rs\" in one task only".into(),
                    },
                ));
            }
        }
        if let Some(c) = &t.check {
            check_text(out, &format!("{f}.check"), c, LINE_CAP, true);
            check_command(out, &format!("{f}.check"), c, commands);
        }
    }
}

/// Rejects a lane file that is not a plain repo-relative path or that `unit` does not own; a
/// file that does not exist yet is fine, since a task may create it.
fn check_owned(
    out: &mut Problems,
    field: &str,
    file: &str,
    root: &Path,
    unit: &str,
    router: &Router,
) {
    let problem = if file.contains('\\') || safe_path(root, file, false).is_err() {
        "not a plain repo-relative path: no `..`, `.`, empty part, leading `/`, backslash, or \
         symlink"
            .to_string()
    } else {
        match router.owner(file) {
            Some(owner) if owner == unit => return,
            Some(owner) => format!("`{file}` is owned by unit `{owner}`, not `{unit}`"),
            None => format!("no unit owns `{file}`"),
        }
    };
    out.push((
        "answer_parse_failed",
        FieldError {
            field: field.to_string(),
            problem,
            example: "\"src/api.rs\"".into(),
        },
    ));
}

/// `command_not_allowed` unless `cmd` has no `SHELL_CHARS` and its leading words are all the
/// words of one `commands.test` entry, so `pytest` allows `pytest tests/x` but not `pytest-x`.
fn check_command(out: &mut Problems, field: &str, cmd: &str, commands: &Commands) {
    if let Some(c) = cmd.chars().find(|c| SHELL_CHARS.contains(c)) {
        out.push((
            "command_not_allowed",
            FieldError {
                field: field.to_string(),
                problem: format!(
                    "contains {c:?}; a command is plain argv with no shell syntax: no \
                     separators, pipes, redirects, substitutions, quotes, or globs"
                ),
                example: "\"pytest tests/api/test_items.py::test_x -k count\"".into(),
            },
        ));
        return;
    }
    let words: Vec<&str> = cmd.split_whitespace().collect();
    let entries: Vec<&str> = commands
        .test
        .iter()
        .map(|e| match e {
            CommandEntry::Plain(c) | CommandEntry::Timed { cmd: c, .. } => c.as_str(),
        })
        .collect();
    let allowed = entries.iter().any(|e| {
        let entry: Vec<&str> = e.split_whitespace().collect();
        !entry.is_empty() && words.starts_with(&entry)
    });
    if !allowed {
        out.push((
            "command_not_allowed",
            FieldError {
                field: field.to_string(),
                problem: format!(
                    "`{cmd}` does not start with a commands.test entry ({})",
                    entries.join(", ")
                ),
                example: format!("\"{} tests\"", entries.first().unwrap_or(&"pytest")),
            },
        ));
    }
}

/// Caps `value` at `cap` characters and rejects data markers, invisible and direction
/// characters, and control characters: any in a `one_line` field, any but newline and tab in prose.
fn check_text(out: &mut Problems, field: &str, value: &str, cap: usize, one_line: bool) {
    let mut add = |code, problem: String, example: &str| {
        out.push((
            code,
            FieldError {
                field: field.to_string(),
                problem,
                example: example.to_string(),
            },
        ))
    };
    let n = value.chars().count();
    if n > cap {
        let example = format!("at most {cap} characters");
        add(
            "answer_too_long",
            format!("{n} characters; the cap is {cap}"),
            &example,
        );
    }
    let bad = value
        .chars()
        .find(|&c| invisible(c) || c.is_control() && (one_line || !matches!(c, '\n' | '\t')));
    if let Some(c) = bad {
        let (problem, example) = if one_line {
            ("; this field is one line", "one line of plain text")
        } else {
            ("", "plain text; newline and tab are allowed")
        };
        let kind = if invisible(c) { "invisible" } else { "control" };
        add(
            "answer_parse_failed",
            format!("{kind} character U+{:04X}{problem}", c as u32),
            example,
        );
    }
    if value.contains("<<<DATA") || value.contains("DATA>>>") {
        add(
            "answer_parse_failed",
            "contains a data marker (`<<<DATA` or `DATA>>>`)".into(),
            "the text without the marker",
        );
    }
}

/// Rejects a heading, a setext underline or thematic break, and an unclosed code fence in the
/// documenter's prose: the page puts it under the unit's own headings, so any of these could fake
/// a section or swallow the sections after it.
fn check_doc_markdown(out: &mut Problems, field: &str, value: &str) {
    let mut add = |problem: String| {
        out.push((
            "answer_parse_failed",
            FieldError {
                field: field.to_string(),
                problem,
                example: "paragraphs, lists, and closed ``` blocks; no headings or rules".into(),
            },
        ))
    };
    // the open fence's character and length, and every fence line, since an odd count is
    // refused even where CommonMark would see the fences balanced
    let mut fence: Option<(char, usize)> = None;
    let mut fences = 0;
    for (n, line) in value.lines().enumerate().map(|(i, l)| (i + 1, l)) {
        let t = line.trim_start_matches([' ', '\t']);
        if t.starts_with('#') {
            add(format!("line {n} starts with `#`, a heading"));
        }
        // setext underlines are any run of `=` or `-`; thematic breaks three or more marks
        let marks: Vec<char> = t.chars().filter(|c| !matches!(c, ' ' | '\t')).collect();
        let only = |m: char| !marks.is_empty() && marks.iter().all(|&c| c == m);
        if only('=') || only('-') || marks.len() >= 3 && (only('*') || only('_')) {
            add(format!("line {n} is a heading underline or a rule"));
        }
        // CommonMark: a fence line is indented at most three spaces; an opening backtick fence
        // has no backtick after it; a closing fence is the open mark, at least as long, alone
        let indent = line.len() - line.trim_start_matches(' ').len();
        let t = &line[indent..];
        let Some(c) = t.chars().next().filter(|c| matches!(c, '`' | '~')) else {
            continue;
        };
        let run = t.len() - t.trim_start_matches(c).len();
        let rest = &t[run..];
        fences += usize::from(run >= 3);
        match fence {
            _ if run < 3 || indent > 3 => {}
            None if c == '`' && rest.contains('`') => {}
            None => fence = Some((c, run)),
            Some((open, len))
                if c == open && run >= len && rest.trim_matches([' ', '\t']).is_empty() =>
            {
                fence = None
            }
            Some(_) => {}
        }
    }
    if fence.is_some() || fences % 2 == 1 {
        add("a code fence (``` or ~~~) is not closed".into());
    }
}

/// Checks the patch's fields and lists; an anchor `alive` rejects is `answer_dead_anchor`.
fn check_patch(
    out: &mut Problems,
    patch: Option<&CardPatch>,
    alive: impl Fn(&model::Anchor) -> bool,
) {
    let Some(p) = patch else {
        return;
    };
    let dead = |out: &mut Problems, field: String, anchor: Option<model::Anchor>| {
        if !anchor.is_some_and(|a| alive(&a)) {
            out.push((
                "answer_dead_anchor",
                FieldError {
                    field,
                    problem: "the anchor does not resolve to a whole-word name in a file \
                              this unit owns in the snapshot"
                        .into(),
                    example: "src/api.rs::create_item".into(),
                },
            ));
        }
    };
    check_list(
        out,
        "card_patch.interface",
        p.interface.iter().flatten().count(),
    );
    check_list(
        out,
        "card_patch.anchors",
        p.anchors.iter().flatten().count(),
    );
    check_list(out, "card_patch.gaps", p.gaps.iter().flatten().count());
    for (i, x) in p.interface.iter().flatten().enumerate() {
        let f = format!("card_patch.interface[{i}]");
        check_text(out, &format!("{f}.id"), &x.id, ID_CAP, true);
        check_text(out, &format!("{f}.anchor"), &x.anchor, LINE_CAP, true);
        check_text(out, &format!("{f}.contract"), &x.contract, LINE_CAP, true);
        dead(
            out,
            format!("{f}.anchor"),
            model::Anchor::parse(&x.anchor).ok(),
        );
    }
    for (i, a) in p.anchors.iter().flatten().enumerate() {
        let f = format!("card_patch.anchors[{i}]");
        check_text(out, &format!("{f}.path"), &a.path, LINE_CAP, true);
        check_text(out, &format!("{f}.name"), &a.name, LINE_CAP, true);
        let anchor = model::Anchor {
            path: a.path.clone(),
            name: a.name.clone(),
        };
        dead(out, f, Some(anchor));
    }
    for (i, g) in p.gaps.iter().flatten().enumerate() {
        let f = format!("card_patch.gaps[{i}]");
        check_text(out, &format!("{f}.title"), &g.title, ID_CAP, true);
        check_text(out, &format!("{f}.note"), &g.note, PROSE_CAP, false);
        if let Some(r) = &g.repro {
            check_text(out, &format!("{f}.repro"), r, LINE_CAP, true);
        }
        if let Some(a) = &g.anchor {
            check_text(out, &format!("{f}.anchor"), a, LINE_CAP, true);
            dead(out, format!("{f}.anchor"), model::Anchor::parse(a).ok());
        }
    }
}

/// Appends `+noun key` for each entry added, `~noun key` for each edited, and `-noun key` for
/// each removed, keyed by `key`.
fn changes<T: Serialize>(
    out: &mut Vec<String>,
    noun: &str,
    old: &[T],
    new: &[T],
    key: impl Fn(&T) -> String,
) {
    let find = |list: &'_ [T], k: &str| list.iter().position(|x| key(x) == k);
    for n in new {
        let k = key(n);
        match find(old, &k) {
            None => out.push(format!("+{noun} {}", one_line(&k))),
            Some(i) if serde_json::to_value(&old[i]).ok() != serde_json::to_value(n).ok() => {
                out.push(format!("~{noun} {}", one_line(&k)))
            }
            Some(_) => {}
        }
    }
    for o in old.iter().filter(|o| find(new, &key(o)).is_none()) {
        out.push(format!("-{noun} {}", one_line(&key(o))));
    }
}

/// `head`, then the `names` in parentheses when there are any.
fn named<'a>(head: String, names: impl Iterator<Item = &'a str>) -> String {
    let names: Vec<&str> = names.collect();
    if names.is_empty() {
        head
    } else {
        format!("{head} ({})", one_line(&names.join(", ")))
    }
}

fn count(n: usize, noun: &str) -> String {
    format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
}

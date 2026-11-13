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
use crate::model::{self, CommandEntry, Commands, GapKind, Role, TestCard, UnitCard};
use crate::next;
use crate::output::{CliError, Global, Human, emit, with_note};
use crate::route::Router;
use crate::telemetry;
use crate::workspace::{Repo, here, safe_path};

/// Ids and titles.
pub const ID_CAP: usize = 120;
/// Contracts, anchors, paths, and commands.
pub const LINE_CAP: usize = 300;
/// Notes, cut, replace_with, gap notes, and every other prose field but explanations and task
/// changes.
pub const PROSE_CAP: usize = 2000;
/// A task's change: larger than the prose cap, since the fix task of a `changes` review joins
/// every finding.
pub const TASK_CHANGE_CAP: usize = 20_000;
pub const EXPLANATION_CAP: usize = 4000;
/// How much of an explanation submit prints.
const EXPLANATION_SHOWN: usize = 2000;
/// How much of a reviewer finding's `cut` submit prints.
const CUT_SHOWN: usize = 300;
/// The largest answer file read.
const ANSWER_BYTES: u64 = 256 * 1024;
/// Entries in any one list of an answer.
pub const LIST_CAP: usize = 200;
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
pub struct Submitted {
    pub(crate) summary: String,
    explanation: Option<String>,
    tester_brief: Option<PathBuf>,
    /// The architect's that set no purpose; printed for the user to put in units.toml.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) unit_proposals: Vec<types::UnitProposal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    warning: Option<String>,
    /// A review's verdict per unit (the reviewer's, or a maintainer review's one unit).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    units: Vec<UnitVerdict>,
    /// A review's findings, for the agent that made the change to fix.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    findings: Vec<ShownFinding>,
    #[serde(flatten)]
    developer: Option<DeveloperReport>,
    #[serde(skip)]
    pulled_anchors: Vec<String>,
}

/// A review's finding as submit prints it: where, what to cut, and the smaller replacement, each
/// text capped.
#[derive(Serialize)]
struct ShownFinding {
    unit: String,
    file: String,
    line: i64,
    cut: String,
    replace_with: String,
}

impl ShownFinding {
    fn new(unit: &str, f: &types::Finding) -> ShownFinding {
        let shown = |s: &str| one_line(s).chars().take(CUT_SHOWN).collect();
        ShownFinding {
            unit: unit.to_string(),
            file: f.file.clone(),
            line: f.line,
            cut: shown(&f.cut),
            replace_with: shown(&f.replace_with),
        }
    }
}

/// One unit's verdict in a review's output.
#[derive(Serialize)]
struct UnitVerdict {
    unit: String,
    verdict: &'static str,
    stamped: bool,
    /// The pass value of the `check` the answer reported; `None` with no check.
    check_reported: Option<bool>,
}

/// What a developer submit computed from git, beside the summary.
#[derive(Serialize)]
struct DeveloperReport {
    out_of_lane: Vec<String>,
    blocked_on: Option<String>,
}

/// A unit proposal on one line.
pub(crate) fn proposal_line(u: &types::UnitProposal) -> String {
    one_line(&format!(
        "unit proposal: {}: {}; paths {}",
        u.name,
        u.purpose,
        u.paths.join(", ")
    ))
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
        out.extend(self.unit_proposals.iter().map(proposal_line));
        out.extend(self.findings.iter().map(|f| {
            one_line(&format!(
                "- {} {}:{}: cut {}",
                f.unit, f.file, f.line, f.cut
            ))
        }));
        out.extend(self.warning.as_ref().map(|w| format!("warning: {w}")));
        out.join("\n")
    }
}

pub fn run(g: &Global, unit: Option<&str>, role: Role, file: &Path) -> Result<ExitCode> {
    let (cwd, ws) = here(g)?;
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
    // the architect and the reviewer take no unit and get ""
    let submit: fn(&Repo, &str, &Header, &str, &Before) -> Result<Submitted> = match role {
        Role::Maintainer => maintainer,
        Role::Tester => tester,
        Role::Documenter => documenter,
        Role::Validator => validator,
        Role::Architect => architect,
        Role::Developer => developer,
        Role::Reviewer => reviewer,
    };
    let loaded = repo.loaded()?;
    let no_unit = matches!(role, Role::Architect | Role::Reviewer);
    if unit.is_some() == no_unit {
        let wants = if no_unit {
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
    if let Some(unit) = unit {
        loaded.unit(unit)?;
    }
    // a developer's task number is in its answer file's name
    let n = match unit.filter(|_| role == Role::Developer) {
        Some(u) => Some(task_number(u, file)?),
        None => None,
    };
    let brief_rel = brief::rel_path(unit, role, n);
    let header = brief::read_header(&repo.root.join(&brief_rel))?;
    check_header(&repo.root, unit, role, n, &header, &brief_rel)?;
    // A reviewer's units name the files it writes, as a unit does, and only a stamped unit is
    // reviewed this way: a hand-edited header must not skip a first review.
    let covered = header.units.clone().unwrap_or_default();
    for (i, u) in covered.iter().enumerate() {
        let stamped = loaded.cards.get(u).is_some_and(|c| c.reviewed.is_some());
        if loaded.unit(u).is_err() || !stamped || covered[..i].contains(u) {
            bail!(CliError::new(
                "no_brief",
                format!(
                    "the header of {brief_rel} lists `{}`, which is unknown, never reviewed, or listed twice",
                    one_line(u)
                ),
                "write a fresh brief with `stewards next`",
            ));
        }
    }
    let text = read_answer(file)?;
    // The job's own answer, the path the brief names: removed with the brief once the submit
    // succeeds. An answer given from anywhere else, or through a link, is the caller's.
    let answer_rel = brief::answer_rel(&brief_rel);
    let own_answer = [file, &repo.root.join(&answer_rel)]
        .iter()
        .all(|p| std::fs::symlink_metadata(p).is_ok_and(|m| m.is_file()))
        && file.canonicalize().ok() == repo.root.join(&answer_rel).canonicalize().ok();
    // Every file the job may write, read before the repo is loaded again for the job, so a
    // write by another submit after this read is caught before this one writes.
    let mut files = vec![".stewards/repo.toml".to_string()];
    if role == Role::Architect {
        files.push(UNITS_TOML.into());
    }
    for u in unit
        .iter()
        .copied()
        .chain(covered.iter().map(String::as_str))
    {
        files.push(format!(".stewards/units/{u}.toml"));
        files.push(format!(".stewards/units/{u}.test.toml"));
        if matches!(role, Role::Maintainer | Role::Reviewer) {
            files.push(tasks_rel(u));
        }
        // a bad docs_path is the documenter's own error, reported after the answer's
        if matches!(role, Role::Documenter | Role::Reviewer)
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
    if own_answer {
        repo.remove(&answer_rel)?;
    }
    let record = telemetry::Record {
        unit: unit.map(String::from),
        role: role.as_str(),
        mode: header.mode,
        task: header.task.clone(),
        bytes: header.bytes,
        anchors: header.anchors,
        pulled_anchors: std::mem::take(&mut out.pulled_anchors),
        harness: harness::detect().map(|h| h.name),
        out_of_lane: out.developer.as_ref().map(|d| d.out_of_lane.len()),
        units: header.units.as_ref().map(Vec::len),
        timestamp: telemetry::rfc3339(SystemTime::now()),
    };
    if let Err(e) = telemetry::append(repo, &record) {
        out.warning = Some(format!("telemetry not written: {e:#}"));
    }
    // what it prints, for `review` and `--run`, which read the outcome from the repository
    let result = serde_json::to_string(&out)?;
    if let Err(e) = repo.write(&brief::result_rel(&brief_rel), result.as_bytes()) {
        let note = format!("result file not written: {e:#}");
        out.warning = Some(match out.warning.take() {
            Some(w) => format!("{w}; {note}"),
            None => note,
        });
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
    // a reviewer's header names the units it covers, and only a reviewer's does
    let units_ok = match &h.units {
        None => role != Role::Reviewer,
        Some(u) => role == Role::Reviewer && !u.is_empty(),
    };
    if h.unit.as_deref() != unit
        || h.role != role
        || h.n != n
        || !units_ok
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
    let mut problems = vec![];
    // `tasks` is the developers' task list to write; None removes it.
    let (patch, pulled, notes, tester_needed, tasks) = match &answer {
        Answer::MaintainerReview(a) => {
            let fix = check_review(&mut problems, &a.verdict, &a.findings, root, unit, &router);
            (
                &a.card_patch,
                &a.pulled_anchors,
                &a.notes,
                a.tester_needed,
                fix.map(|t| vec![t]),
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
    let old = loaded.card(unit);
    let alive = |a: &model::Anchor| anchor_alive(root, &entries, &router, unit, a);
    check_patch(&mut problems, patch.as_ref(), &old, alive);
    check_common(&mut problems, pulled, notes);
    if !problems.is_empty() {
        return Err(rejected(problems));
    }

    let (mut card, patched) = apply_patch(&old, patch.as_ref());
    let card_line = card_line(&old, &card);

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
    if let Answer::MaintainerReview(a) = &answer {
        out.units.push(UnitVerdict {
            unit: unit.to_string(),
            verdict: if approve { "approve" } else { "changes" },
            stamped: approve,
            check_reported: None,
        });
        out.findings = a
            .findings
            .iter()
            .map(|f| ShownFinding::new(unit, f))
            .collect();
    }
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
    let mut edits = vec![];
    if let Some(text) = card_text {
        checked.push(&card_rel);
        edits.push((card_rel.clone(), Some(text)));
    }
    edits.extend(task_edits(root, unit, tasks.as_ref())?);
    if let Some(text) = tester_text {
        let rel = brief::rel_path(Some(unit), Role::Tester, None);
        out.tester_brief = Some(root.join(&rel));
        let answer = brief::answer_rel(&rel);
        edits.push((rel, Some(text)));
        edits.push((answer, None));
    }
    let brief_rel = brief::rel_path(Some(unit), Role::Maintainer, None);
    write_remove_and_retire(repo, before, &checked, &edits, &brief_rel)?;
    Ok(out)
}

/// The name of a finding's rung, as the answer spells it.
fn rung(f: &types::Finding) -> String {
    let rung = serde_json::to_value(&f.rung).expect("a rung serializes");
    rung.as_str().unwrap_or_default().to_string()
}

/// An anchor a patch may name: in a file `unit` owns in the snapshot `entries` (never `.git/`,
/// an ignored file, or another unit's file), resolving there.
fn anchor_alive(
    root: &Path,
    entries: &[(String, String)],
    router: &Router,
    unit: &str,
    a: &model::Anchor,
) -> bool {
    in_snapshot(entries, &a.path)
        && router.owner(&a.path) == Some(unit)
        && matches!(resolve(root, a), Resolution::Found { .. })
}

/// Checks a review's verdict against its findings (`approve` has none, `changes` at least one)
/// and each finding of `unit`; returns a `changes` review's fix task: the findings as one task
/// whose lane is the files they name.
fn check_review(
    problems: &mut Problems,
    verdict: &Verdict,
    findings: &[types::Finding],
    root: &Path,
    unit: &str,
    router: &Router,
) -> Option<types::Task> {
    let empty = findings.is_empty();
    if (*verdict == Verdict::Approve) != empty {
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
                    count(findings.len(), "finding")
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
    check_list(problems, "findings", findings.len());
    for (i, f) in findings.iter().enumerate() {
        let file = format!("findings[{i}].file");
        check_text(problems, &file, &f.file, LINE_CAP, true);
        check_owned(problems, &file, &f.file, root, unit, router);
        check_text(
            problems,
            &format!("findings[{i}].cut"),
            &f.cut,
            PROSE_CAP,
            false,
        );
        let field = format!("findings[{i}].replace_with");
        check_text(problems, &field, &f.replace_with, PROSE_CAP, false);
    }
    let fix = (*verdict == Verdict::Changes).then(|| {
        let mut files: Vec<String> = vec![];
        for f in findings {
            if !files.contains(&f.file) {
                files.push(f.file.clone());
            }
        }
        let lines: Vec<String> = findings
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
        check_text(problems, "findings", &t.change, TASK_CHANGE_CAP, false);
    }
    fix
}

/// `old` with the patch applied, and whether the patch named anything. Each list upserts by key
/// and keeps the rest of the card; only the remove lists remove.
fn apply_patch(old: &UnitCard, patch: Option<&CardPatch>) -> (UnitCard, bool) {
    let mut card = old.clone();
    let patch = patch.cloned().unwrap_or_default();
    let patched = patch.interface.is_some()
        || patch.anchors.is_some()
        || patch.gaps.is_some()
        || patch.remove_interface.is_some()
        || patch.remove_anchors.is_some()
        || patch.remove_gaps.is_some();
    for i in patch.interface.into_iter().flatten() {
        let i = model::Interface {
            id: i.id,
            anchor: i.anchor,
            contract: i.contract,
        };
        put(&mut card.interface, i, |o, n| o.id == n.id);
    }
    for a in patch.anchors.into_iter().flatten() {
        let a = model::Anchor {
            path: a.path,
            name: a.name,
        };
        put(&mut card.anchors, a, |o, n| o == n);
    }
    for g in patch.gaps.into_iter().flatten() {
        // severity and issue are the tool's: a gap keeps them across a patch, by title
        let kept = old.gaps.iter().find(|o| o.title == g.title);
        let g = model::Gap {
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
        };
        put(&mut card.gaps, g, |o, n| o.title == n.title);
    }
    let ids = patch.remove_interface.unwrap_or_default();
    card.interface.retain(|i| !ids.contains(&i.id));
    let anchors = patch.remove_anchors.unwrap_or_default();
    card.anchors
        .retain(|a| !anchors.contains(&format!("{}::{}", a.path, a.name)));
    let titles = patch.remove_gaps.unwrap_or_default();
    card.gaps.retain(|g| !titles.contains(&g.title));
    (card, patched)
}

/// What changed from `old` to `card`, for a summary: `card unchanged`, or `card: ` and each
/// interface, anchor, and gap added, edited, or removed.
fn card_line(old: &UnitCard, card: &UnitCard) -> String {
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
    if changed.is_empty() {
        "card unchanged".to_string()
    } else {
        format!("card: {}", changed.join(", "))
    }
}

/// The task list's edits: `tasks` written to the unit's task file, or, with none, the file and
/// every developer job file of the unit removed.
fn task_edits(
    root: &Path,
    unit: &str,
    tasks: Option<&Vec<types::Task>>,
) -> Result<Vec<(String, Option<String>)>> {
    let text = tasks.map(serde_json::to_string_pretty).transpose()?;
    let mut edits = vec![(tasks_rel(unit), text)];
    if tasks.is_none() {
        let files = brief::developer_files_past(root, unit, 0)?;
        edits.extend(files.into_iter().map(|rel| (rel, None)));
    }
    Ok(edits)
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
    let brief_rel = brief::rel_path(Some(unit), Role::Developer, Some(n));
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
    let mut card = loaded.card(unit);
    let mut tests = test_card(loaded, unit);

    let mut p = vec![];
    check_scenarios(&mut p, &a.scenario_patch, &card, unit, commands);
    check_list(&mut p, "results", a.results.len());
    for (i, r) in a.results.iter().enumerate() {
        let f = format!("results[{i}]");
        check_text(&mut p, &format!("{f}.scenario"), &r.scenario, ID_CAP, true);
        check_text(&mut p, &format!("{f}.output"), &r.output, PROSE_CAP, false);
    }
    check_bugs(&mut p, &a.bugs, &card, unit, commands);
    check_list(&mut p, "tests_written", a.tests_written.len());
    for (i, t) in a.tests_written.iter().enumerate() {
        check_text(&mut p, &format!("tests_written[{i}]"), t, LINE_CAP, true);
    }
    check_common(&mut p, &a.pulled_anchors, &a.notes);
    if !p.is_empty() {
        return Err(rejected(p));
    }

    apply_scenarios(&mut tests, &a.scenario_patch);
    apply_bugs(&mut card, &a.bugs);
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
        &brief::rel_path(Some(unit), Role::Tester, None),
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

/// The unit's test card, or an empty one.
fn test_card(loaded: &model::Loaded, unit: &str) -> TestCard {
    loaded.tests.get(unit).cloned().unwrap_or_else(|| TestCard {
        unit: unit.to_string(),
        primary_paths: vec![],
        test_command: None,
        scenarios: vec![],
    })
}

/// `answer_parse_failed` for `field` unless `id` is an interface id of `card`.
fn check_interface_id(p: &mut Problems, field: String, id: &str, card: &UnitCard, unit: &str) {
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
}

/// Checks a `scenario_patch` for `card`: caps, an allowed command, a known interface id.
fn check_scenarios(
    p: &mut Problems,
    scenarios: &[types::Scenario],
    card: &UnitCard,
    unit: &str,
    commands: &Commands,
) {
    check_list(p, "scenario_patch", scenarios.len());
    for (i, s) in scenarios.iter().enumerate() {
        let f = format!("scenario_patch[{i}]");
        check_text(p, &format!("{f}.id"), &s.id, ID_CAP, true);
        check_text(p, &format!("{f}.interface"), &s.interface, ID_CAP, true);
        check_text(p, &format!("{f}.given"), &s.given, LINE_CAP, true);
        check_text(p, &format!("{f}.expect"), &s.expect, LINE_CAP, true);
        check_text(p, &format!("{f}.level"), &s.level, ID_CAP, true);
        if let Some(c) = &s.command {
            check_text(p, &format!("{f}.command"), c, LINE_CAP, true);
            check_command(p, &format!("{f}.command"), c, commands);
        }
        check_interface_id(p, format!("{f}.interface"), &s.interface, card, unit);
    }
}

/// Checks the `bugs` reported for `card`: caps, an allowed repro, a known interface id, and a
/// title no gap of another kind has.
fn check_bugs(
    p: &mut Problems,
    bugs: &[types::Bug],
    card: &UnitCard,
    unit: &str,
    commands: &Commands,
) {
    check_list(p, "bugs", bugs.len());
    for (i, b) in bugs.iter().enumerate() {
        let f = format!("bugs[{i}]");
        check_text(p, &format!("{f}.title"), &b.title, ID_CAP, true);
        check_text(p, &format!("{f}.repro"), &b.repro, LINE_CAP, true);
        check_command(p, &format!("{f}.repro"), &b.repro, commands);
        check_text(p, &format!("{f}.evidence"), &b.evidence, PROSE_CAP, false);
        if let Some(id) = &b.interface {
            check_text(p, &format!("{f}.interface"), id, ID_CAP, true);
            check_interface_id(p, format!("{f}.interface"), id, card, unit);
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
}

/// Upserts each scenario into the test card by id.
fn apply_scenarios(tests: &mut TestCard, scenarios: &[types::Scenario]) {
    for s in scenarios {
        let new = model::Scenario {
            id: s.id.clone(),
            interface: s.interface.clone(),
            given: s.given.clone(),
            expect: s.expect.clone(),
            level: s.level.clone(),
            command: s.command.clone(),
        };
        put(&mut tests.scenarios, new, |o, n| o.id == n.id);
    }
}

/// Upserts each bug onto the card as a bug gap, by title.
fn apply_bugs(card: &mut UnitCard, bugs: &[types::Bug]) {
    for b in bugs {
        // the named interface's anchor shows the validator where the bug is; a known bug keeps
        // its severity and issue, and its anchor when no interface is named
        let named = b
            .interface
            .as_ref()
            .and_then(|id| card.interface.iter().find(|i| &i.id == id));
        let kept = card.gaps.iter().find(|g| g.title == b.title);
        let g = model::Gap {
            kind: GapKind::Bug,
            title: b.title.clone(),
            anchor: named
                .map(|i| i.anchor.clone())
                .or_else(|| kept.and_then(|o| o.anchor.clone())),
            note: b.evidence.clone(),
            repro: Some(b.repro.clone()),
            severity: kept.and_then(|o| o.severity.clone()),
            issue: kept.and_then(|o| o.issue.clone()),
        };
        put(&mut card.gaps, g, |o, n| o.title == n.title);
    }
}

/// The documenter's submit after its brief and `before` are read; public for the lost-update tests.
pub fn documenter(
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
    check_doc(&mut p, &a.doc);
    check_common(&mut p, &a.pulled_anchors, &a.notes);
    if !p.is_empty() {
        return Err(rejected(p));
    }

    let loaded = repo.loaded()?;
    let mut card = loaded.card(unit);
    card.doc = Some(model_doc(&a.doc));
    if let Some(r) = card.reviewed.as_mut() {
        r.documenter = Some(git::snapshot(&repo.root)?);
    }
    let card_text = toml::to_string(&card)?;
    // The page renders the card as it is about to be written.
    let mut fresh = loaded.clone();
    fresh.cards.insert(unit.to_string(), card);
    let mut writes = vec![(format!(".stewards/units/{unit}.toml"), card_text)];
    writes.extend(docs::pages(&fresh, Some(unit))?);
    // The index is derived from every card, so documenters of two units both rewrite it: not
    // a lost update, and not checked.
    let checked: Vec<&str> = writes
        .iter()
        .map(|(rel, _)| rel.as_str())
        .filter(|rel| !rel.ends_with("/README.md"))
        .collect();
    let edits: Vec<(String, Option<String>)> = writes
        .iter()
        .map(|(rel, text)| (rel.clone(), Some(text.clone())))
        .collect();
    write_remove_and_retire(
        repo,
        before,
        &checked,
        &edits,
        &brief::rel_path(Some(unit), Role::Documenter, None),
    )?;
    Ok(Submitted {
        summary: format!("{unit} documenter: doc updated; {}", writes[1].0),
        pulled_anchors: a.pulled_anchors,
        ..Default::default()
    })
}

/// Checks the documenter's three prose sections: caps, markdown that could fake a section, and
/// not all three empty.
fn check_doc(p: &mut Problems, doc: &types::Doc) {
    for (field, value) in [
        ("doc.overview", &doc.overview),
        ("doc.how_it_works", &doc.how_it_works),
        ("doc.how_to_change", &doc.how_to_change),
    ] {
        check_text(p, field, value, PROSE_CAP, false);
        check_doc_markdown(p, field, value);
    }
    // an empty doc leaves `next` listing the job again, forever
    if [&doc.overview, &doc.how_it_works, &doc.how_to_change]
        .iter()
        .all(|v| v.trim().is_empty())
    {
        p.push((
            "answer_parse_failed",
            FieldError {
                field: "doc".into(),
                problem: "all three prose fields are empty".into(),
                example: "{\"overview\": \"Serves items over HTTP.\", ...}".into(),
            },
        ));
    }
}

/// The reviewer's submit after its brief and `before` are read; public for the lost-update tests.
/// Every unit of the brief once, each checked as its maintainer's review, tester's scenarios and
/// bugs, and documenter's doc would be, and nothing written on any error. Per unit: the card
/// patch, bugs, and doc; an approval stamps at the brief's snapshot and sets the tester and
/// documenter marks there; `changes` writes the fix task as a maintainer's does.
pub fn reviewer(
    repo: &Repo,
    _unit: &str,
    header: &Header,
    text: &str,
    before: &Before,
) -> Result<Submitted> {
    let Answer::Review(a) = parse_answer(Role::Reviewer, header.mode, text)? else {
        unreachable!("a reviewer answer parses to the review variant")
    };
    // `next` rewrites review.md: an answer to an older brief must not stamp the newer one
    if a.snapshot != header.snapshot {
        let brief_rel = brief::rel_path(None, Role::Reviewer, None);
        bail!(CliError::new(
            "no_brief",
            format!(
                "the answer's `snapshot` is not the one in {brief_rel}: it answers an older review brief"
            ),
            format!("read {brief_rel} again and answer the units it lists now, with its snapshot"),
        ));
    }
    let root = &repo.root;
    let loaded = repo.loaded()?;
    let commands = &loaded.repo.commands;
    let snapshot = git::snapshot(root)?;
    let entries = git::ls_tree(root, &snapshot)?;
    let router = Router::new(&loaded.units, &loaded.repo.docs_path)?;
    let covered = header.units.clone().unwrap_or_default();

    let mut p = vec![];
    check_list(&mut p, "units", a.units.len());
    // each covered unit's review, with its card patched and its fix task
    let mut reviews = vec![];
    for (k, u) in a.units.iter().enumerate() {
        let field = format!("units[{k}].unit");
        check_text(&mut p, &field, &u.unit, ID_CAP, true);
        let problem = if !covered.contains(&u.unit) {
            format!(
                "`{}` is not under review; the units are: {}",
                u.unit,
                covered.join(", ")
            )
        } else if a.units[..k].iter().any(|o| o.unit == u.unit) {
            format!("`{}` appears twice; give each unit once", u.unit)
        } else {
            let unit = u.unit.as_str();
            let old = loaded.card(unit);
            let (card, _) = apply_patch(&old, u.card_patch.as_ref());
            let mut fix = None;
            prefixed(&mut p, &format!("units[{k}]."), |p| {
                fix = check_review(p, &u.verdict, &u.findings, root, unit, &router);
                let alive = |a: &model::Anchor| anchor_alive(root, &entries, &router, unit, a);
                check_patch(p, u.card_patch.as_ref(), &old, alive);
                // the brief shows the unit's and the repo's test commands: with one, run it
                let shown = !commands.test.is_empty()
                    || loaded
                        .tests
                        .get(unit)
                        .is_some_and(|t| t.test_command.is_some());
                match &u.check {
                    Some(c) if shown => {
                        check_text(p, "check.command", &c.command, LINE_CAP, true);
                        check_text(p, "check.output", &c.output, PROSE_CAP, false);
                    }
                    None if !shown => {}
                    _ => p.push((
                        "answer_parse_failed",
                        FieldError {
                            field: "check".into(),
                            problem: if shown {
                                "missing; run the test command the brief shows and report it"
                            } else {
                                "the brief shows no test command, so there is none to report"
                            }
                            .into(),
                            example: if shown {
                                "{\"command\": \"pytest tests\", \"pass\": true, \"output\": \"2 passed\"}"
                            } else {
                                "null"
                            }
                            .into(),
                        },
                    )),
                }
                if u.verdict == Verdict::Approve && u.check.as_ref().is_some_and(|c| !c.pass) {
                    p.push((
                        "answer_parse_failed",
                        FieldError {
                            field: "verdict".into(),
                            problem: "approve with a failing check; a unit whose tests fail is \
                                      not approved"
                                .into(),
                            example: "\"changes\", with a finding for the failure".into(),
                        },
                    ));
                }
                // the scenarios and bugs may name an interface the patch adds
                let scenarios = u.scenario_patch.as_deref().unwrap_or_default();
                check_scenarios(p, scenarios, &card, unit, commands);
                check_bugs(
                    p,
                    u.bugs.as_deref().unwrap_or_default(),
                    &card,
                    unit,
                    commands,
                );
                match &u.doc {
                    Some(doc) => check_doc(p, doc),
                    // `next` lists the unit until it has docs
                    None if next::needs_docs(&card) => p.push((
                        "answer_parse_failed",
                        FieldError {
                            field: "doc".into(),
                            problem: format!(
                                "unit `{unit}` has an interface and no docs; write all three sections"
                            ),
                            example: "{\"overview\": \"Serves items over HTTP.\", ...}".into(),
                        },
                    )),
                    None => {}
                }
            });
            reviews.push((u, old, card, fix));
            continue;
        };
        p.push((
            "answer_parse_failed",
            FieldError {
                field,
                problem,
                example: format!("\"{}\"", covered[0]),
            },
        ));
    }
    for unit in covered
        .iter()
        .filter(|c| !a.units.iter().any(|u| &u.unit == *c))
    {
        p.push((
            "answer_parse_failed",
            FieldError {
                field: "units".into(),
                problem: format!("`{unit}` is missing; every unit under review appears once"),
                example: format!("{{\"unit\": \"{unit}\", ...}}"),
            },
        ));
    }
    check_common(&mut p, &a.pulled_anchors, &a.notes);
    if !p.is_empty() {
        return Err(rejected(p));
    }

    let mut edits: Vec<(String, Option<String>)> = vec![];
    let mut checked = vec![];
    let mut fresh = loaded.clone();
    let mut paged = vec![];
    let mut parts = vec![];
    let mut findings = vec![];
    let mut verdicts = vec![];
    let short: String = header.snapshot.chars().take(7).collect();
    for (u, old, mut card, fix) in reviews {
        let unit = u.unit.as_str();
        apply_bugs(&mut card, u.bugs.as_deref().unwrap_or_default());
        let mut part = vec![card_line(&old, &card)];
        if let Some(doc) = &u.doc {
            card.doc = Some(model_doc(doc));
            part.push("doc updated".into());
        }
        let scenarios = u.scenario_patch.as_deref().unwrap_or_default();
        let mut written = !scenarios.is_empty();
        if written {
            let mut tests = test_card(loaded, unit);
            apply_scenarios(&mut tests, scenarios);
            let rel = format!(".stewards/units/{unit}.test.toml");
            edits.push((rel.clone(), Some(toml::to_string(&tests)?)));
            checked.push(rel);
            part.push(count(scenarios.len(), "scenario") + " upserted");
            fresh.tests.insert(unit.to_string(), tests);
        }
        if u.verdict == Verdict::Approve {
            // at the brief's snapshot, as a maintainer's; the reviewer answered for the tester
            // and the documenter too
            let mut r = check::reviewed_at(repo, unit, &header.snapshot, old.reviewed.as_ref())?;
            r.tester = Some(header.snapshot.clone());
            r.documenter = Some(header.snapshot.clone());
            card.reviewed = Some(r);
            part.push(format!("stamped {short}"));
        } else {
            part.insert(0, count(u.findings.len(), "finding"));
            part.push("not stamped".into());
            findings.extend(u.findings.iter().map(|f| ShownFinding::new(unit, f)));
        }
        let card_text = toml::to_string(&card)?;
        if card_text != toml::to_string(&old)? {
            let rel = format!(".stewards/units/{unit}.toml");
            edits.push((rel.clone(), Some(card_text)));
            checked.push(rel);
            written = true;
        }
        // its page shows the card and the scenarios, so it is rendered again with them
        if written && card.doc.is_some() {
            paged.push(unit);
        }
        edits.extend(task_edits(root, unit, fix.map(|t| vec![t]).as_ref())?);
        checked.push(tasks_rel(unit));
        let verdict = if u.verdict == Verdict::Approve {
            "approve"
        } else {
            "changes"
        };
        verdicts.push(UnitVerdict {
            unit: unit.to_string(),
            verdict,
            stamped: u.verdict == Verdict::Approve,
            check_reported: u.check.as_ref().map(|c| c.pass),
        });
        parts.push(format!("{unit} {verdict} ({})", part.join("; ")));
        fresh.cards.insert(unit.to_string(), card);
    }
    // each page renders its card as it is about to be written; the index once, unchecked, as the
    // documenter's
    for unit in paged {
        for (rel, text) in docs::pages(&fresh, Some(unit))? {
            if !rel.ends_with("/README.md") {
                checked.push(rel.clone());
            }
            edits.retain(|(r, _)| *r != rel);
            edits.push((rel, Some(text)));
        }
    }
    let checked: Vec<&str> = checked.iter().map(String::as_str).collect();
    let brief_rel = brief::rel_path(None, Role::Reviewer, None);
    write_remove_and_retire(repo, before, &checked, &edits, &brief_rel)?;
    Ok(Submitted {
        summary: one_line(&format!("reviewer: {}", parts.join(", "))),
        units: verdicts,
        findings,
        pulled_anchors: a.pulled_anchors,
        ..Default::default()
    })
}

/// Runs `check` on a list of its own and adds its problems to `out`, `prefix` before each field.
fn prefixed(out: &mut Problems, prefix: &str, check: impl FnOnce(&mut Problems)) {
    let mut p = vec![];
    check(&mut p);
    out.extend(p.into_iter().map(|(code, mut e)| {
        e.field = format!("{prefix}{}", e.field);
        (code, e)
    }));
}

fn model_doc(doc: &types::Doc) -> model::Doc {
    model::Doc {
        overview: doc.overview.clone(),
        how_it_works: doc.how_it_works.clone(),
        how_to_change: doc.how_to_change.clone(),
    }
}

fn validator(
    repo: &Repo,
    unit: &str,
    header: &Header,
    text: &str,
    before: &Before,
) -> Result<Submitted> {
    let loaded = repo.loaded()?;
    let mut card = loaded.card(unit);
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
        &brief::rel_path(Some(unit), Role::Validator, None),
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
        let entry = model::Anchor::parse(&flow.entry).ok();
        let spaced = entry
            .as_ref()
            .is_some_and(|e| spaced_name(&mut p, &format!("{f}.entry"), &e.name));
        let alive = entry.is_some_and(|e| {
            in_snapshot(&entries, &e.path) && matches!(resolve(root, &e), Resolution::Found { .. })
        });
        if !spaced && !alive {
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
        // a blank purpose sets nothing: it would block a later architect from naming the unit
        let same = !u.purpose.trim().is_empty()
            && loaded
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
                let mut purpose = toml_edit::Value::from(u.purpose.trim());
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
        &brief::rel_path(None, Role::Architect, None),
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
    let edits: Vec<(String, Option<String>)> = writes
        .iter()
        .map(|(rel, text)| (rel.clone(), Some(text.clone())))
        .collect();
    let checked: Vec<&str> = writes.iter().map(|(rel, _)| rel.as_str()).collect();
    write_remove_and_retire(repo, before, &checked, &edits, brief_rel)
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
    with_note(
        e,
        format!("submit had already written {}", written.join(", ")),
    )
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
        check_text(
            out,
            &format!("{f}.change"),
            &t.change,
            TASK_CHANGE_CAP,
            false,
        );
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

/// `answer_parse_failed` for `field` when the anchor `name` holds whitespace (`fn dispatch`,
/// `pub struct Pack`): a name is the bare identifier, which survives a change of keyword or
/// visibility. True when it does.
fn spaced_name(out: &mut Problems, field: &str, name: &str) -> bool {
    let spaced = name.contains(char::is_whitespace);
    if spaced {
        out.push((
            "answer_parse_failed",
            FieldError {
                field: field.to_string(),
                problem: "an anchor name is the bare identifier, without keywords such as fn \
                          or struct"
                    .into(),
                example: "src/api.rs::create_item, not src/api.rs::fn create_item".into(),
            },
        ));
    }
    spaced
}

/// Replaces the first entry of `list` that is `same` as `item`, or appends `item`.
fn put<T>(list: &mut Vec<T>, item: T, same: impl Fn(&T, &T) -> bool) {
    match list.iter_mut().find(|o| same(o, &item)) {
        Some(o) => *o = item,
        None => list.push(item),
    }
}

/// Checks the patch's fields and lists against `card`, the card it patches; an anchor `alive`
/// rejects is `answer_dead_anchor`.
fn check_patch(
    out: &mut Problems,
    patch: Option<&CardPatch>,
    card: &UnitCard,
    alive: impl Fn(&model::Anchor) -> bool,
) {
    let Some(p) = patch else {
        return;
    };
    // A removal names a key the card has, so a typo cannot pass, and one the patch does not
    // also add or update. `keys` are the card's, `upserted` the patch list's.
    let removals = |out: &mut Problems,
                    list: &str,
                    cap: usize,
                    remove: &Option<Vec<String>>,
                    keys: Vec<String>,
                    upserted: Vec<String>| {
        let remove = remove.iter().flatten();
        check_list(out, &format!("card_patch.{list}"), remove.clone().count());
        for (i, key) in remove.enumerate() {
            let field = format!("card_patch.{list}[{i}]");
            check_text(out, &field, key, cap, true);
            let problem = if list == "remove_anchors" && model::Anchor::parse(key).is_err() {
                format!("`{key}` is not `path::name`")
            } else if !keys.contains(key) {
                format!("`{key}` is not on the card; a removal names an existing entry")
            } else if upserted.contains(key) {
                format!("`{key}` is also added or updated by this patch; give it in one list")
            } else {
                continue;
            };
            out.push((
                "answer_parse_failed",
                FieldError {
                    field,
                    problem,
                    example: if keys.is_empty() {
                        "the card has none".into()
                    } else {
                        format!("one of: {}", keys.join(", "))
                    },
                },
            ));
        }
    };
    let anchor_key = |path: &str, name: &str| format!("{path}::{name}");
    removals(
        out,
        "remove_interface",
        ID_CAP,
        &p.remove_interface,
        card.interface.iter().map(|i| i.id.clone()).collect(),
        p.interface.iter().flatten().map(|i| i.id.clone()).collect(),
    );
    removals(
        out,
        "remove_anchors",
        LINE_CAP,
        &p.remove_anchors,
        card.anchors
            .iter()
            .map(|a| anchor_key(&a.path, &a.name))
            .collect(),
        p.anchors
            .iter()
            .flatten()
            .map(|a| anchor_key(&a.path, &a.name))
            .collect(),
    );
    removals(
        out,
        "remove_gaps",
        ID_CAP,
        &p.remove_gaps,
        card.gaps.iter().map(|g| g.title.clone()).collect(),
        p.gaps.iter().flatten().map(|g| g.title.clone()).collect(),
    );
    let dead = |out: &mut Problems, field: String, anchor: Option<model::Anchor>| {
        if anchor
            .as_ref()
            .is_some_and(|a| spaced_name(out, &field, &a.name))
        {
            return;
        }
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
        if spaced_name(out, &format!("{f}.name"), &a.name) {
            continue;
        }
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

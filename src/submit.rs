// The `submit` command: validate a role's typed answer against its brief header and write it.
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::SystemTime;

use anyhow::{Result, bail};
use serde::Serialize;

use crate::anchors::{Resolution, resolve};
use crate::brief::{self, Header};
use crate::check::{self, in_snapshot, one_line};
use crate::git;
use crate::harness;
use crate::llm::types::{self, CardPatch, Verdict};
use crate::llm::{self, Answer, FieldError, Mode};
use crate::model::{self, GapKind, Role, UnitCard};
use crate::output::{CliError, Global, Human, emit, not_implemented};
use crate::route::Router;
use crate::telemetry;
use crate::workspace::{Repo, discover, safe_path};

/// Ids and titles.
const ID_CAP: usize = 120;
/// Contracts, anchors, paths, and commands.
const LINE_CAP: usize = 300;
/// Notes, cut, replace_with, gap notes, and task changes.
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

/// Each field error with the error code its kind of problem maps to.
type Problems = Vec<(&'static str, FieldError)>;

#[derive(Serialize)]
struct Submitted {
    summary: String,
    explanation: Option<String>,
    tester_brief: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    warning: Option<String>,
    #[serde(skip)]
    pulled_anchors: Vec<String>,
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
        out.extend(self.warning.as_ref().map(|w| format!("warning: {w}")));
        out.join("\n")
    }
}

pub fn run(g: &Global, unit: Option<&str>, role: Role, file: &Path) -> Result<ExitCode> {
    let submit: fn(&Repo, &str, &Header, &str) -> Result<Submitted> = match role {
        Role::Maintainer => maintainer,
        _ => return not_implemented(&format!("submit --role {}", role.as_str())),
    };
    let cwd = std::env::current_dir()?.canonicalize()?;
    let ws = discover(&cwd, g.repo.as_deref())?;
    let repo = ws.current_repo(&cwd)?;
    let loaded = repo.loaded()?;
    let Some(unit) = unit else {
        bail!(CliError::new(
            "usage",
            format!("submit --role {} needs a unit", role.as_str()),
            "run `stewards submit --help`",
        ));
    };
    // The unit names the files submit writes, so only a units.toml name (checked on load) passes.
    if !loaded.units.iter().any(|u| u.name == unit) {
        let names: Vec<_> = loaded.units.iter().map(|u| u.name.as_str()).collect();
        bail!(CliError::new(
            "unknown_unit",
            format!("no unit named `{unit}`; units: {}", names.join(", ")),
            "one of the listed units",
        ));
    }
    let brief_rel = brief::rel_path(Some(unit), role);
    let header = brief::read_header(&repo.root.join(&brief_rel))?;
    check_header(&repo.root, unit, role, &header, &brief_rel)?;
    let text = read_answer(file)?;
    let mut out = submit(repo, unit, &header, &text)?;
    let record = telemetry::Record {
        unit: Some(unit.to_string()),
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
        timestamp: telemetry::rfc3339(SystemTime::now()),
    };
    if let Err(e) = telemetry::append(repo, &record) {
        out.warning = Some(format!("telemetry not written: {e:#}"));
    }
    emit(&out, g.human);
    Ok(ExitCode::SUCCESS)
}

/// `no_brief` unless the header names this unit and role and a full snapshot tree id: a
/// hand-edited header must not stamp another unit, or stamp a commit or a ref.
fn check_header(root: &Path, unit: &str, role: Role, h: &Header, rel: &str) -> Result<()> {
    let hex = matches!(h.snapshot.len(), 40 | 64)
        && h.snapshot
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    if h.unit.as_deref() != Some(unit) || h.role != role || !hex || !git::is_tree(root, &h.snapshot)
    {
        bail!(CliError::new(
            "no_brief",
            format!(
                "the header of {rel} is not usable for {unit} {}",
                role.as_str()
            ),
            format!(
                "write a fresh brief with `stewards next` or `stewards brief {unit} --role {}`",
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

fn maintainer(repo: &Repo, unit: &str, header: &Header, text: &str) -> Result<Submitted> {
    let answer = llm::parse(Role::Maintainer, header.mode, text).map_err(|errors| {
        rejected(
            errors
                .into_iter()
                .map(|e| ("answer_parse_failed", e))
                .collect(),
        )
    })?;
    let root = &repo.root;
    let loaded = repo.loaded()?;
    // Taken before any write: patch anchors are checked in it and the tester brief packs it.
    let snapshot = git::snapshot(root)?;
    let entries = git::ls_tree(root, &snapshot)?;
    let router = Router::new(&loaded.units, &loaded.repo.docs_path)?;
    let mut problems = vec![];
    let (patch, pulled, notes, tester_needed) = match &answer {
        Answer::MaintainerReview(a) => {
            if a.verdict == Verdict::Approve && !a.findings.is_empty() {
                problems.push((
                    "answer_parse_failed",
                    FieldError {
                        field: "verdict".into(),
                        problem: format!(
                            "approve with {}; an approval has no findings",
                            count(a.findings.len(), "finding")
                        ),
                        example: "\"changes\"".into(),
                    },
                ));
            }
            check_list(&mut problems, "findings", a.findings.len());
            for (i, f) in a.findings.iter().enumerate() {
                let p = &mut problems;
                check_text(p, &format!("findings[{i}].file"), &f.file, LINE_CAP, true);
                check_text(p, &format!("findings[{i}].cut"), &f.cut, PROSE_CAP, false);
                let field = format!("findings[{i}].replace_with");
                check_text(p, &field, &f.replace_with, PROSE_CAP, false);
            }
            (&a.card_patch, &a.pulled_anchors, &a.notes, a.tester_needed)
        }
        Answer::MaintainerExplain(a) => {
            let p = &mut problems;
            check_text(p, "explanation", &a.explanation, EXPLANATION_CAP, false);
            check_list(p, "tasks", a.tasks.len());
            for (i, t) in a.tasks.iter().enumerate() {
                check_text(p, &format!("tasks[{i}].title"), &t.title, ID_CAP, true);
                check_text(
                    p,
                    &format!("tasks[{i}].change"),
                    &t.change,
                    PROSE_CAP,
                    false,
                );
                check_list(p, &format!("tasks[{i}].files"), t.files.len());
                for (j, f) in t.files.iter().enumerate() {
                    check_text(p, &format!("tasks[{i}].files[{j}]"), f, LINE_CAP, true);
                }
                if let Some(c) = &t.check {
                    check_text(p, &format!("tasks[{i}].check"), c, LINE_CAP, true);
                }
            }
            (&a.card_patch, &a.pulled_anchors, &a.notes, false)
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
    check_list(&mut problems, "pulled_anchors", pulled.len());
    for (i, a) in pulled.iter().enumerate() {
        check_text(
            &mut problems,
            &format!("pulled_anchors[{i}]"),
            a,
            LINE_CAP,
            true,
        );
    }
    check_text(&mut problems, "notes", notes, PROSE_CAP, false);
    if !problems.is_empty() {
        return Err(rejected(problems));
    }

    let old = loaded.cards.get(unit).cloned();
    let old = old.unwrap_or_else(|| UnitCard {
        unit: unit.to_string(),
        reviewed: None,
        interface: vec![],
        anchors: vec![],
        gaps: vec![],
        doc: None,
    });
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
        summary: String::new(),
        explanation: None,
        tester_brief: None,
        warning: None,
        pulled_anchors: pulled.clone(),
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
                .map(|f| {
                    let rung = serde_json::to_value(&f.rung).expect("a rung serializes");
                    format!(
                        "{} {}:{}",
                        rung.as_str().unwrap_or_default(),
                        f.file,
                        f.line
                    )
                })
                .collect();
            let mut findings = count(list.len(), "finding");
            if !list.is_empty() {
                findings += &format!(" ({})", list.join(", "));
            }
            format!(
                "{unit} maintainer: changes; {findings}; {card_line}; next: stewards -C {} brief {unit} --role developer",
                root.display()
            )
        }
        Answer::MaintainerExplain(a) => {
            out.explanation = Some(a.explanation.chars().take(EXPLANATION_SHOWN).collect());
            let tasks = count(a.tasks.len(), "task");
            format!("{unit} maintainer: explained; {card_line}; {tasks}")
        }
        _ => unreachable!("a maintainer answer parses to a maintainer variant"),
    };

    // The writes, last: the card, the tester brief, then the retired job's brief. A link at any
    // of their paths is refused here, before the first of them.
    let card_rel = format!(".stewards/units/{unit}.toml");
    let tester_rel = brief::rel_path(Some(unit), Role::Tester);
    let tester_answer = format!(".stewards/briefs/{unit}-tester.answer.json");
    let brief_rel = brief::rel_path(Some(unit), Role::Maintainer);
    let planned = [
        (card_text.is_some(), &card_rel),
        (tester_needed, &tester_rel),
        (tester_needed, &tester_answer),
        (true, &brief_rel),
    ];
    for (_, rel) in planned.iter().filter(|(writes, _)| *writes) {
        safe_path(root, rel, false)?;
    }
    let mut written: Vec<&str> = vec![];
    if let Some(text) = &card_text {
        repo.write(&card_rel, text.as_bytes())
            .map_err(|e| after_writes(e, &written))?;
        written.push(&card_rel);
    }
    if let Some(text) = &tester_text {
        let path = brief::write_text(repo, Some(unit), Role::Tester, text)
            .map_err(|e| after_writes(e, &written))?;
        out.tester_brief = Some(path);
        written.push(&tester_rel);
    }
    // a submitted job is done: a replay of the same answer finds no brief
    repo.remove(&brief_rel)
        .map_err(|e| after_writes(e, &written))?;
    Ok(out)
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
        _ => "fix the fields listed in `errors` and rerun submit",
    };
    let mut err = CliError::new(
        code,
        format!("the answer has {}", count(problems.len(), "problem")),
        fix,
    );
    // field and problem may quote the answer: one line, capped
    let shown = |s: &str| one_line(s).chars().take(ERROR_SHOWN).collect::<String>();
    err.errors = problems
        .into_iter()
        .map(|(_, e)| FieldError {
            field: shown(&e.field),
            problem: shown(&e.problem),
            example: e.example,
        })
        .collect();
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

/// Characters that hide text or reorder it on screen.
fn invisible(c: char) -> bool {
    matches!(c, '\u{2028}' | '\u{2029}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}'
        | '\u{2060}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
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

fn count(n: usize, noun: &str) -> String {
    format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
}

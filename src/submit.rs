// The `submit` command: validate a role's typed answer against its brief header and write it.
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::SystemTime;

use anyhow::{Result, bail};
use serde::Serialize;

use crate::anchors::{Resolution, resolve};
use crate::brief::{self, Header};
use crate::check;
use crate::git;
use crate::harness;
use crate::llm::types::{self, CardPatch, Verdict};
use crate::llm::{self, Answer, FieldError, Mode};
use crate::model::{self, GapKind, Role, UnitCard, load_repo};
use crate::output::{CliError, Global, Human, emit, not_implemented};
use crate::telemetry;
use crate::workspace::{Repo, discover};

/// Ids and titles.
const ID_CAP: usize = 120;
/// Contracts, anchors, paths, and commands.
const LINE_CAP: usize = 300;
/// Notes, cut, replace_with, gap notes, and task changes.
const PROSE_CAP: usize = 2000;
const EXPLANATION_CAP: usize = 4000;
/// How much of an explanation submit prints.
const EXPLANATION_SHOWN: usize = 2000;

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
    let header = brief::read_header(
        &repo
            .root
            .join(format!(".stewards/briefs/{unit}-{}.md", role.as_str())),
    )?;
    let text = std::fs::read_to_string(file).map_err(|e| {
        CliError::new(
            "usage",
            format!("cannot read {}: {e}", file.display()),
            "pass the answer file the brief names",
        )
    })?;
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
    if let Err(e) = telemetry::append(&repo.root, &record) {
        out.warning = Some(format!("telemetry not written: {e:#}"));
    }
    emit(&out, g.human);
    Ok(ExitCode::SUCCESS)
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
            for (i, t) in a.tasks.iter().enumerate() {
                check_text(p, &format!("tasks[{i}].title"), &t.title, ID_CAP, true);
                check_text(
                    p,
                    &format!("tasks[{i}].change"),
                    &t.change,
                    PROSE_CAP,
                    false,
                );
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
    check_patch(&mut problems, root, patch.as_ref());
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

    // Checked before any write: check::stamp and brief::write also write these paths.
    let card_rel = format!(".stewards/units/{unit}.toml");
    refuse_symlinks(root, &card_rel)?;
    if tester_needed {
        refuse_symlinks(root, &format!(".stewards/briefs/{unit}-tester.md"))?;
        refuse_symlinks(root, &format!(".stewards/briefs/{unit}-tester.answer.json"))?;
    }

    let old = repo.loaded()?.cards.get(unit).cloned();
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
        // At the brief's snapshot, so a file edited since stays stale. stamp writes the card as
        // loaded, so the patched card is written after it.
        card.reviewed = Some(check::stamp(repo, unit, &header.snapshot)?);
    }
    if patched {
        write_under_stewards(root, &card_rel, toml::to_string(&card)?.as_bytes())?;
    }

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
                "{unit} maintainer: changes; {findings}; {card_line}; next: stewards brief {unit} --role developer"
            )
        }
        Answer::MaintainerExplain(a) => {
            out.explanation = Some(a.explanation.chars().take(EXPLANATION_SHOWN).collect());
            let tasks = count(a.tasks.len(), "task");
            format!("{unit} maintainer: explained; {card_line}; {tasks}")
        }
        _ => unreachable!("a maintainer answer parses to a maintainer variant"),
    };
    if tester_needed {
        // The tester's pack must see the card as just written, not as loaded before it.
        let fresh = Repo {
            name: repo.name.clone(),
            root: root.clone(),
            loaded: Some(load_repo(root)?),
        };
        let snapshot = git::snapshot(root)?;
        let (path, _) = brief::write(
            &fresh,
            Some(unit),
            Role::Tester,
            Mode::Review,
            &header.task,
            &snapshot,
        )?;
        out.tester_brief = Some(path);
    }
    Ok(out)
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
    err.errors = problems.into_iter().map(|p| p.1).collect();
    err.into()
}

/// Caps `value` at `cap` characters and rejects data markers and control characters: any in a
/// `one_line` field, any but newline and tab in prose.
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
        .find(|&c| c.is_control() && (one_line || !matches!(c, '\n' | '\t')));
    if let Some(c) = bad {
        let (problem, example) = if one_line {
            ("; this field is one line", "one line of plain text")
        } else {
            ("", "plain text; newline and tab are allowed")
        };
        add(
            "answer_parse_failed",
            format!("control character U+{:04X}{problem}", c as u32),
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

fn check_patch(out: &mut Problems, root: &Path, patch: Option<&CardPatch>) {
    let Some(p) = patch else {
        return;
    };
    let dead = |out: &mut Problems, field: String, anchor: Option<model::Anchor>| {
        if !anchor.is_some_and(|a| matches!(resolve(root, &a), Resolution::Found { .. })) {
            out.push((
                "answer_dead_anchor",
                FieldError {
                    field,
                    problem: "the anchor does not resolve to a whole-word name in a repo file"
                        .into(),
                    example: "src/api.rs::create_item".into(),
                },
            ));
        }
    };
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

/// Appends `+n noun`, `-n noun`, and `n noun edited` for the list's change, keyed by `key`.
fn changes<T: Serialize>(
    out: &mut Vec<String>,
    noun: &str,
    old: &[T],
    new: &[T],
    key: impl Fn(&T) -> String,
) {
    let find = |list: &'_ [T], k: &str| list.iter().position(|x| key(x) == k);
    let added = new.iter().filter(|n| find(old, &key(n)).is_none()).count();
    let removed = old.iter().filter(|o| find(new, &key(o)).is_none()).count();
    let edited = new
        .iter()
        .filter(|n| {
            find(old, &key(n))
                .is_some_and(|i| serde_json::to_value(&old[i]).ok() != serde_json::to_value(n).ok())
        })
        .count();
    if added > 0 {
        out.push(format!("+{}", count(added, noun)));
    }
    if removed > 0 {
        out.push(format!("-{}", count(removed, noun)));
    }
    if edited > 0 {
        out.push(format!("{} edited", count(edited, noun)));
    }
}

fn count(n: usize, noun: &str) -> String {
    format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
}

/// `root/rel`, or `unsafe_path` when any component below `root` is a symlink: a hostile repo could
/// commit one to point a write outside `.stewards/`.
fn refuse_symlinks(root: &Path, rel: &str) -> Result<PathBuf> {
    let mut path = root.to_path_buf();
    for c in Path::new(rel).components() {
        path.push(c);
        if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            bail!(CliError::new(
                "unsafe_path",
                format!("{rel} passes through the symlink {}", path.display()),
                "replace the symlink with a plain file or directory",
            ));
        }
    }
    Ok(path)
}

/// Every write submit makes itself goes through here.
fn write_under_stewards(root: &Path, rel: &str, bytes: &[u8]) -> Result<()> {
    let path = refuse_symlinks(root, rel)?;
    std::fs::create_dir_all(path.parent().expect("a file has a parent"))?;
    std::fs::write(&path, bytes)?;
    Ok(())
}

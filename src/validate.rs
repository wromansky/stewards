// `validate --report`: record a bug report on its unit's card, run its repro, brief a validator.
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::brief;
use crate::check::one_line;
use crate::git;
use crate::llm::{FieldError, types};
use crate::model::{self, Role};
use crate::output::{CliError, Global, Human, emit, not_implemented};
use crate::run;
use crate::submit::{self, Problems};
use crate::workspace::{Repo, here};

const SHAPE: &str = r#"{"unit": "api", "title": "count off by one", "repro": "<a commands.test prefix> tests/api -k count", "evidence": "expected 2, got 3", "interface": null}"#;

/// The report file: a tester's bug and its unit.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Report {
    unit: String,
    title: String,
    repro: String,
    evidence: String,
    #[serde(default)]
    interface: Option<String>,
}

#[derive(Serialize)]
struct Recorded {
    unit: String,
    title: String,
}

#[derive(Serialize)]
struct Validated {
    #[serde(skip_serializing_if = "Option::is_none")]
    recorded: Option<Recorded>,
    #[serde(skip_serializing_if = "Option::is_none")]
    duplicate: Option<String>,
    repro_result: Option<serde_json::Value>,
    reproduced: Option<bool>,
    brief_path: Option<PathBuf>,
    handoff: Option<String>,
    then: String,
}

impl Human for Validated {
    fn render(&self) -> String {
        let mut out = vec![];
        if let Some(r) = &self.recorded {
            out.push(format!("Recorded `{}` on {}.", one_line(&r.title), r.unit));
        }
        out.extend(self.duplicate.clone());
        if let Some(r) = &self.reproduced {
            out.push(format!("Reproduced: {r}"));
        }
        out.extend(
            self.brief_path
                .as_ref()
                .map(|p| format!("Wrote `{}`.", p.display())),
        );
        out.extend(
            self.handoff
                .as_ref()
                .map(|h| format!("Handoff: {}", one_line(h))),
        );
        out.push(self.then.clone());
        out.join("\n")
    }
}

/// A report error: `code` with the fields listed and the shape of a report as the fix.
fn bad(code: &'static str, message: String, errors: Vec<FieldError>) -> anyhow::Error {
    let mut e = CliError::new(
        code,
        message,
        format!(
            "write the report file as one JSON object, for example {SHAPE}, and rerun validate"
        ),
    );
    e.errors = errors;
    e.into()
}

fn parse(text: &str) -> Result<Report> {
    serde_json::from_str(text).map_err(|e| {
        bad(
            "answer_parse_failed",
            "the report is not one JSON object of the right shape".into(),
            vec![FieldError {
                field: "(document)".into(),
                problem: e.to_string(),
                example: SHAPE.into(),
            }],
        )
    })
}

pub fn run(g: &Global, report: Option<&Path>, export: bool) -> Result<ExitCode> {
    if export {
        return not_implemented("validate --export");
    }
    let Some(file) = report else {
        return Err(CliError::new(
            "usage",
            "pass --report <file> to record a bug, or --export to file judged bugs as issues",
            format!("stewards validate --report <file>; the file is {SHAPE}"),
        )
        .into());
    };
    let (cwd, ws) = here(g)?;
    let out = report_file(ws.current_repo(&cwd)?, file)?;
    emit(&out, g.human);
    Ok(ExitCode::SUCCESS)
}

/// `repo` with its files read again.
fn reloaded(repo: &Repo) -> Result<Repo> {
    Ok(Repo {
        name: repo.name.clone(),
        root: repo.root.clone(),
        loaded: Some(model::load_repo(&repo.root)?),
    })
}

fn report_file(repo: &Repo, file: &Path) -> Result<Validated> {
    let r = parse(&submit::read_answer(file)?)?;
    let unit = r.unit.as_str();
    repo.loaded()?.unit(unit)?;
    let card_rel = format!(".stewards/units/{unit}.toml");
    let before = submit::record(&repo.root, std::slice::from_ref(&card_rel))?;
    let repo = &reloaded(repo)?;
    let loaded = repo.loaded()?;
    let mut card = loaded.card(unit);
    let bug = types::Bug {
        title: r.title.clone(),
        repro: r.repro.clone(),
        evidence: r.evidence.clone(),
        interface: r.interface.clone(),
    };
    let mut p: Problems = vec![];
    submit::check_bugs(
        &mut p,
        std::slice::from_ref(&bug),
        &card,
        unit,
        &run::allowed(loaded, Some(unit)),
    );
    if !p.is_empty() {
        let e = submit::rejected(p).downcast::<CliError>()?;
        return Err(bad(e.code, e.message, e.errors));
    }
    if let Some(d) = submit::apply_bugs(&mut card, std::slice::from_ref(&bug)).pop() {
        return Ok(Validated {
            recorded: None,
            duplicate: Some(d),
            repro_result: None,
            reproduced: None,
            brief_path: None,
            handoff: None,
            then: "No brief was written: reuse the existing title to update that bug, or give another interface anchor.".into(),
        });
    }
    submit::check_unchanged(&repo.root, &before, &[&card_rel])?;
    repo.write(&card_rel, toml::to_string(&card)?.as_bytes())?;

    let result = brief::repro_result(repo, unit, &bug.repro)?;
    let snapshot = git::snapshot(&repo.root)?;
    let task = format!("validate the bug `{}`", r.title);
    let repo = &reloaded(repo)?;
    let text = brief::build_validator(
        repo,
        unit,
        Some(&r.title),
        &task,
        &snapshot,
        Some(result.clone()),
    )?;
    let path = brief::write_text(repo, Some(unit), Role::Validator, None, &snapshot, &text)?;
    Ok(Validated {
        recorded: Some(Recorded {
            unit: r.unit,
            title: r.title,
        }),
        duplicate: None,
        reproduced: result["reproduced"].as_bool(),
        repro_result: Some(result),
        handoff: Some(brief::handoff(&path)?),
        brief_path: Some(path),
        then: "Start one subagent with the handoff below as its whole task; when it has finished, run stewards review to read its judgment.".into(),
    })
}

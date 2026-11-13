// `stewards review`: the jobs `next` lists, each run in the harness's own headless session, one
// after another, and what `next` lists after them.
use std::process::ExitCode;

use anyhow::Context;
use serde::Serialize;

use crate::check::one_line;
use crate::launch::{self, Launcher, Outcome, Status};
use crate::next::{self, NextReport};
use crate::output::{Global, Human, emit, not_implemented};
use crate::workspace::here;

const TASK_HINT: &str = "pass `--task \"<what you changed and why>\"`: the reviewer checks that every changed line traces to the request";

#[derive(Serialize)]
struct Review {
    /// The request the reviewer checks against.
    task: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<&'static str>,
    /// The harness launched; `None` when no job was due.
    harness: Option<&'static str>,
    jobs: Vec<Outcome>,
    /// What `next` lists after this run: units deferred to the next round, findings to fix,
    /// never-reviewed units.
    remaining: NextReport,
    then: String,
}

impl Human for Review {
    fn render(&self) -> String {
        let mut out = vec![format!("Task: {}", one_line(&self.task))];
        out.extend(self.hint.map(|h| format!("Hint: {h}")));
        out.push("\n## Review jobs\n".into());
        if self.jobs.is_empty() {
            out.push("No jobs were due.".into());
        }
        out.extend(self.jobs.iter().map(Outcome::render));
        out.push(format!("\n## Remaining\n\n{}", self.remaining.render()));
        out.push(format!("\nThen: {}.", self.then));
        out.join("\n")
    }
}

pub fn run(
    g: &Global,
    task: Option<&str>,
    roles: bool,
    options: &launch::Options,
    pr: Option<u64>,
) -> anyhow::Result<ExitCode> {
    if pr.is_some() {
        return not_implemented("review --pr");
    }
    let command = if roles { "review --roles" } else { "review" };
    if let Some(code) = launch::guard(g, command) {
        return Ok(code);
    }
    let (_, ws) = here(g)?;
    let due = next::next(&ws, task, true, roles)?;
    let mut jobs = vec![];
    let mut harness = None;
    if !due.jobs.is_empty() {
        let l = Launcher::new(options, command)?;
        harness = Some(l.harness.name);
        for j in &due.jobs {
            let brief = j.brief_path.as_ref().context("next wrote no brief")?;
            let repo = ws
                .repo_containing(brief)
                .with_context(|| format!("no repo holds {}", brief.display()))?;
            jobs.push(l.run_job(repo, brief)?);
        }
    }
    // loaded again: the launched submits wrote cards
    let (_, ws) = here(g)?;
    let remaining = next::next(&ws, task, false, roles)?;
    let clean = remaining.jobs.is_empty() && remaining.check.failures.is_empty();
    let then = if clean {
        "done".to_string()
    } else if jobs.iter().any(|j| j.status == Status::Changes) {
        format!("fix the findings and run stewards {command} again")
    } else {
        format!("run stewards {command} again for the next round")
    };
    let (task, hint) = match task {
        Some(t) => (t.to_string(), None),
        None => {
            let used = due
                .jobs
                .first()
                .map_or("the branch and last commit", |j| &j.task);
            (
                format!("(none given; used {})", one_line(used)),
                Some(TASK_HINT),
            )
        }
    };
    emit(
        &Review {
            task,
            hint,
            harness,
            jobs,
            remaining,
            then,
        },
        g.human,
    );
    Ok(if clean {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

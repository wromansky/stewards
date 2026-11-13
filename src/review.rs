// `stewards review`: collects what the subagents of the last call submitted, then lists what
// `next` lists now, each job with its handoff. It never starts a session.
use std::process::ExitCode;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::check::one_line;
use crate::model::Role;
use crate::next::{self, Briefs, NextReport};
use crate::output::{Global, Human, emit, not_implemented};
use crate::submit::{ShownFinding, UnitVerdict};
use crate::workspace::{Repo, here, safe_path};

const TASK_HINT: &str = "pass `--task \"<what you changed and why>\"`: the reviewer checks that every changed line traces to the request";
const FIX: &str = "Fix the findings, then run stewards review again.";
/// The largest result file read.
const RESULT_BYTES: u64 = 256 * 1024;

/// What one job's `submit` printed, from its result file.
#[derive(Serialize, Deserialize)]
struct Collected {
    #[serde(skip_deserializing)]
    role: Option<Role>,
    #[serde(skip_deserializing)]
    unit: Option<String>,
    #[serde(default)]
    summary: String,
    /// A review's verdict per unit.
    #[serde(default)]
    units: Vec<UnitVerdict>,
    #[serde(default)]
    findings: Vec<ShownFinding>,
}

#[derive(Serialize)]
struct Review {
    /// The request the reviewer checks against.
    task: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<&'static str>,
    /// Each result file present when the command started, read and removed.
    collected: Vec<Collected>,
    /// What to do now; before `next`, whose handoffs it refers to as below.
    then: String,
    /// What `next` lists now, each job with its handoff; `None` while collected findings are to
    /// be fixed first.
    next: Option<NextReport>,
}

impl Human for Review {
    fn render(&self) -> String {
        let mut out = vec![format!("Task: {}", one_line(&self.task))];
        out.extend(self.hint.map(|h| format!("Hint: {h}")));
        out.push("\n## Collected\n".into());
        if self.collected.is_empty() {
            out.push("No results.".into());
        }
        for c in &self.collected {
            let who = format!(
                "{} {}",
                c.unit.as_deref().unwrap_or_default(),
                c.role.map_or("unknown role", |r| r.as_str())
            );
            out.push(format!("- `{}`: {}", who.trim(), one_line(&c.summary)));
            out.extend(c.findings.iter().map(|f| {
                one_line(&format!(
                    "  - {} {}:{}: cut {}; replace with {}",
                    f.unit, f.file, f.line, f.cut, f.replace_with
                ))
            }));
        }
        out.push(format!("\nThen: {}", self.then));
        match &self.next {
            Some(n) => out.push(format!("\n{}", n.render())),
            None => out.push("\n## Jobs\n\nNone until the findings are fixed.".into()),
        }
        out.join("\n")
    }
}

/// The unit and role of the brief whose result file has stem `stem` (`brief::rel_path`'s names):
/// `review`, `architect`, `<unit>-<role>`, or `<unit>-developer-<n>`.
fn job_of(stem: &str) -> (Option<String>, Option<Role>) {
    if stem == "review" {
        return (None, Some(Role::Reviewer));
    }
    let stem = match stem.rsplit_once('-') {
        Some((s, n)) if n.parse::<usize>().is_ok() => s,
        _ => stem,
    };
    match stem.rsplit_once('-') {
        Some((unit, role)) => (Some(unit.to_string()), role.parse().ok()),
        None => (None, stem.parse().ok()),
    }
}

/// Reads and removes every result file in `repo`'s `.stewards/briefs/`, in name order.
fn collect(repo: &Repo) -> Result<Vec<Collected>> {
    let dir = match std::fs::read_dir(repo.root.join(".stewards/briefs")) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        r => r?,
    };
    let mut names: Vec<String> = dir
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.ends_with(".result.json"))
        .collect();
    names.sort();
    let mut out = vec![];
    for name in names {
        let rel = format!(".stewards/briefs/{name}");
        let path = safe_path(&repo.root, &rel, false)?;
        let read = std::fs::symlink_metadata(&path)
            .ok()
            .filter(|m| m.is_file() && m.len() <= RESULT_BYTES)
            .and_then(|_| std::fs::read(&path).ok())
            .and_then(|b| serde_json::from_slice::<Collected>(&b).ok());
        let (unit, role) = job_of(name.trim_end_matches(".result.json"));
        let mut c = read.unwrap_or_else(|| Collected {
            role: None,
            unit: None,
            summary: format!("{rel} is not a result submit wrote"),
            units: vec![],
            findings: vec![],
        });
        c.findings = c.findings.into_iter().map(ShownFinding::capped).collect();
        (c.unit, c.role) = (unit, role);
        repo.remove(&rel)?;
        out.push(c);
    }
    Ok(out)
}

pub fn run(g: &Global, task: Option<&str>, roles: bool, pr: Option<u64>) -> Result<ExitCode> {
    if pr.is_some() {
        return not_implemented("review --pr");
    }
    let command = if roles { "review --roles" } else { "review" };
    let (_, ws) = here(g)?;
    let mut collected = vec![];
    for (repo, _) in ws.loaded_repos()? {
        collected.extend(collect(repo)?);
    }
    let findings = collected
        .iter()
        .any(|c| !c.findings.is_empty() || c.units.iter().any(|u| u.verdict == "changes"));
    let report = if findings {
        None
    } else {
        Some(next::next(&ws, task, Briefs::KeepWaiting, roles)?)
    };
    let (then, clean) = match &report {
        None => (FIX.to_string(), false),
        Some(r) if !r.jobs.is_empty() => {
            let mut then = format!(
                "Start one subagent per handoff below, giving it exactly that sentence as its whole task; when they have finished, run stewards {command} again."
            );
            if r.jobs.iter().any(|j| j.waiting) {
                then += " A waiting job's brief has no submit yet: if its subagent already replied, it did not finish; start a new one with the same handoff.";
            }
            (then, false)
        }
        Some(r) if !r.check.failures.is_empty() => (FIX.to_string(), false),
        Some(_) => ("done".to_string(), true),
    };
    let (task, hint) = match task {
        Some(t) => (t.to_string(), None),
        None => {
            let used = report
                .as_ref()
                .and_then(|r| r.jobs.first())
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
            collected,
            then,
            next: report,
        },
        g.human,
    );
    Ok(if clean {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brief;

    #[test]
    fn job_of_reads_every_brief_name() {
        for (stem, unit, role) in [
            ("review", None, Role::Reviewer),
            ("architect", None, Role::Architect),
            ("api-tester", Some("api"), Role::Tester),
            ("my-api-maintainer", Some("my-api"), Role::Maintainer),
            ("my-api-developer-2", Some("my-api"), Role::Developer),
        ] {
            assert_eq!(job_of(stem), (unit.map(String::from), Some(role)), "{stem}");
            let rel = brief::rel_path(unit, role, (role == Role::Developer).then_some(2));
            assert_eq!(
                brief::result_rel(&rel),
                format!(".stewards/briefs/{stem}.result.json")
            );
        }
    }
}

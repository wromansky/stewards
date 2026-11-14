// `stewards review`: reports what the subagents of the last call submitted, then lists what
// `next` lists now, each job with its handoff. It never starts a session.
use std::process::ExitCode;

use anyhow::Result;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::check::{escape_invisible, one_line};
use crate::model::{Loaded, Role};
use crate::next::{self, Briefs, NextReport};
use crate::output::{Global, Human, emit, not_implemented};
use crate::route::Router;
use crate::submit::{ShownFinding, TASK_CHANGE_CAP, UnitVerdict};
use crate::workspace::{Repo, here, safe_path};
use crate::{brief, git};

const TASK_HINT: &str = "pass `--task \"<what you changed and why>\"`: the reviewer checks that every changed line traces to the request";
/// Where `review --task` keeps the request for the calls after it, until `review` is done.
const TASK_REL: &str = ".stewards/briefs/review-task.txt";
/// The largest result file read.
const RESULT_BYTES: u64 = 256 * 1024;
/// Result files read per call; the rest are read by the next call.
const RESULTS_PER_CALL: usize = 20;
/// Entries of a result's findings, units, or any other list shown.
const LIST_SHOWN: usize = 50;
/// Characters of a summary, a file name, or any other text shown.
const TEXT_SHOWN: usize = 2000;

/// `text` as printed from a result or a file name: on one line, each invisible character shown
/// as `\u{XXXX}`, at most `TEXT_SHOWN` characters.
fn shown(text: &str) -> String {
    escape_invisible(&one_line(text))
        .chars()
        .take(TEXT_SHOWN)
        .collect()
}

/// `v` with every text `shown`, every list and object cut to `LIST_SHOWN` entries, and no nulls
/// in an object.
fn shown_value(v: Value) -> Value {
    match v {
        Value::String(s) => Value::String(shown(&s)),
        Value::Array(a) => a.into_iter().take(LIST_SHOWN).map(shown_value).collect(),
        Value::Object(o) => Value::Object(shown_map(o)),
        v => v,
    }
}

fn shown_map(o: Map<String, Value>) -> Map<String, Value> {
    o.into_iter()
        .filter(|(_, v)| !v.is_null())
        .take(LIST_SHOWN)
        .map(|(k, v)| (shown(&k), shown_value(v)))
        .collect()
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// What one subagent reported through `submit`, from its result file. stewards did not verify
/// it: it is shown as the file says, after submit's own checks on each finding and unit.
#[derive(Serialize)]
struct Reported {
    /// The result file's name; `role` and `unit` come from it.
    file: String,
    role: Option<Role>,
    /// `None` unless the name gives a unit of the repo.
    unit: Option<String>,
    /// Why the file is not shown as a result.
    #[serde(skip_serializing_if = "Option::is_none")]
    not_a_result: Option<String>,
    summary: String,
    /// A review's verdict per unit.
    units: Vec<UnitVerdict>,
    findings: Vec<ShownFinding>,
    /// Every other field of the result (explanation, unit proposals, tester brief, out-of-lane
    /// files, blocked_on, warning, ...), as `shown_value` prints it.
    other: Map<String, Value>,
    /// Findings and units dropped: they fail submit's checks or name no unit of the repo.
    #[serde(skip_serializing_if = "is_zero")]
    invalid: usize,
    /// Findings and units past the `LIST_SHOWN` shown of each.
    #[serde(skip_serializing_if = "is_zero")]
    not_shown: usize,
    /// It reports a valid finding or a `changes` verdict.
    #[serde(skip)]
    changes: bool,
}

impl Reported {
    fn new(file: &str, role: Option<Role>, unit: Option<String>) -> Reported {
        Reported {
            file: shown(file),
            role,
            unit,
            not_a_result: None,
            summary: String::new(),
            units: vec![],
            findings: vec![],
            other: Map::new(),
            invalid: 0,
            not_shown: 0,
            changes: false,
        }
    }

    /// Fills the result from `bytes`, read in `repo`; false when they are not a JSON object.
    fn read(&mut self, bytes: &[u8], repo: &Repo, loaded: &Loaded, router: &Router) -> bool {
        let Ok(Value::Object(mut map)) = serde_json::from_slice(bytes) else {
            return false;
        };
        let is_unit = |u: &str| loaded.units.iter().any(|d| d.name == u);
        let mut list = |key: &str| match map.remove(key) {
            Some(Value::Array(a)) => a,
            _ => vec![],
        };
        let (units, findings) = (list("units"), list("findings"));
        let total = units.len() + findings.len();
        self.units = units
            .into_iter()
            .filter_map(|u| serde_json::from_value::<UnitVerdict>(u).ok())
            .filter(|u| is_unit(&u.unit) && ["approve", "changes"].contains(&u.verdict.as_str()))
            .collect();
        self.findings = findings
            .into_iter()
            .filter_map(|f| serde_json::from_value::<ShownFinding>(f).ok())
            .filter(|f| f.passes_submit_checks(&repo.root, router))
            .map(ShownFinding::capped)
            .collect();
        self.invalid = total - self.units.len() - self.findings.len();
        self.changes =
            !self.findings.is_empty() || self.units.iter().any(|u| u.verdict == "changes");
        self.not_shown = self.units.len().saturating_sub(LIST_SHOWN)
            + self.findings.len().saturating_sub(LIST_SHOWN);
        self.units.truncate(LIST_SHOWN);
        self.findings.truncate(LIST_SHOWN);
        self.summary = map
            .remove("summary")
            .and_then(|s| s.as_str().map(shown))
            .unwrap_or_default();
        self.other = shown_map(map);
        true
    }
}

/// The result files of one call, across repos.
#[derive(Default)]
struct Collected<'a> {
    reported: Vec<Reported>,
    /// Each result file read, removed once the output is written.
    read: Vec<(&'a Repo, String)>,
    /// Result files not read this call: past `RESULTS_PER_CALL`.
    left: usize,
    /// Files named as results that are not, past the `RESULTS_PER_CALL` shown.
    not_results_unshown: usize,
}

#[derive(Serialize)]
struct Task {
    /// The request the reviewer checks against.
    text: String,
    /// `given` (`--task`), `stored` (the last `--task`, kept until `review` is done), or `none`
    /// (the branch and last commit stand in).
    from: &'static str,
}

#[derive(Serialize)]
struct Review {
    task: Task,
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<&'static str>,
    /// What each subagent's `submit` wrote to its result file since the last call: reported by
    /// the subagent, not verified by stewards.
    reported: Vec<Reported>,
    #[serde(skip_serializing_if = "is_zero")]
    left: usize,
    #[serde(skip_serializing_if = "is_zero")]
    not_results_unshown: usize,
    /// What to do now; before `next`, whose handoffs it refers to as below.
    then: String,
    /// What `next` lists now, each job with its handoff; `None` while reported findings are to be
    /// fixed first or result files are left to read.
    next: Option<NextReport>,
}

impl Human for Review {
    fn render(&self) -> String {
        let mut out = vec![format!(
            "Task ({}): {}",
            self.task.from,
            one_line(&self.task.text)
        )];
        out.extend(self.hint.map(|h| format!("Hint: {h}")));
        out.push("\n## Reported by subagents (not verified by stewards)\n".into());
        if self.reported.is_empty() {
            out.push("No results.".into());
        }
        for r in &self.reported {
            let who = format!(
                "{} {}",
                r.unit.as_deref().unwrap_or_default(),
                r.role.map_or("unknown role", |r| r.as_str())
            );
            if let Some(why) = &r.not_a_result {
                out.push(format!("- `{}`: not a result: {why}", r.file));
                continue;
            }
            out.push(format!("- `{}` ({}): {}", who.trim(), r.file, r.summary));
            out.extend(r.units.iter().map(|u| {
                let stamped = if u.stamped { "stamped" } else { "not stamped" };
                format!("  - unit {}: {}, {stamped}", u.unit, u.verdict)
            }));
            out.extend(r.findings.iter().map(|f| {
                format!(
                    "  - finding {} {}:{}: cut {}; replace with {}",
                    f.unit, f.file, f.line, f.cut, f.replace_with
                )
            }));
            out.extend(r.other.iter().map(|(k, v)| match v {
                Value::String(s) => format!("  - {k}: {s}"),
                v => format!("  - {k}: {v}"),
            }));
            if r.invalid > 0 {
                out.push(format!(
                    "  - {} entries were not valid and are not shown.",
                    r.invalid
                ));
            }
            if r.not_shown > 0 {
                out.push(format!("  - {} more entries are not shown.", r.not_shown));
            }
        }
        if self.left > 0 {
            out.push(format!(
                "{} more result files are left for the next call.",
                self.left
            ));
        }
        if self.not_results_unshown > 0 {
            out.push(format!(
                "{} more files named as results are not results and are not shown.",
                self.not_results_unshown
            ));
        }
        out.push(format!("\nThen: {}", self.then));
        match &self.next {
            Some(n) => out.push(format!("\n{}", n.render())),
            None => out.push("\n## Jobs\n\nNone until the above is done.".into()),
        }
        out.join("\n")
    }
}

/// Reads the result files in `repo`'s `.stewards/briefs/`, in name order, into `c`, at most
/// `RESULTS_PER_CALL` per call. A file git tracks or that is not a regular file is reported as
/// not a result and left in place; any other bad file is reported as such, never aborting.
// simplified: a result planted by a hostile subagent (it can write here) cannot be told apart
// from one submit wrote without a key kept outside the repository; until then results are shown
// as reported, never as verified, and cannot make `review` pass.
fn collect<'a>(repo: &'a Repo, loaded: &Loaded, c: &mut Collected<'a>) -> Result<()> {
    let dir = match std::fs::read_dir(repo.root.join(".stewards/briefs")) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        r => r?,
    };
    let mut names: Vec<String> = dir
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.ends_with(".result.json"))
        .collect();
    names.sort();
    let tracked = git::ls_files(&repo.root)?;
    let router = Router::new(&loaded.units, &loaded.repo.docs_path)?;
    for name in names {
        let rel = format!(".stewards/briefs/{name}");
        let job = brief::job_of(&name);
        let role = job.as_ref().map(|j| j.role);
        let unit = job
            .and_then(|j| j.unit)
            .filter(|u| loaded.units.iter().any(|d| &d.name == u));
        let mut r = Reported::new(&name, role, unit);
        let regular = safe_path(&repo.root, &rel, false)
            .and_then(|p| Ok(std::fs::symlink_metadata(p)?))
            .is_ok_and(|m| m.is_file());
        let kept = if tracked.contains(&rel) {
            Some("git tracks it; submit writes only untracked result files")
        } else if !regular {
            Some("not a regular file")
        } else {
            None
        };
        if let Some(why) = kept {
            let shown_kept = c.reported.len() - c.read.len();
            if shown_kept < RESULTS_PER_CALL {
                r.not_a_result = Some(why.into());
                c.reported.push(r);
            } else {
                c.not_results_unshown += 1;
            }
            continue;
        }
        if c.read.len() == RESULTS_PER_CALL {
            c.left += 1;
            continue;
        }
        c.read.push((repo, rel.clone()));
        match repo.read(&rel, RESULT_BYTES) {
            Err(e) => r.not_a_result = Some(shown(&format!("{e:#}"))),
            Ok(bytes) if r.read(&bytes, repo, loaded, &router) => {}
            Ok(_) => r.not_a_result = Some("not the JSON object submit writes".into()),
        }
        c.reported.push(r);
    }
    Ok(())
}

/// The task the last `review --task` stored in `repo`, if any.
fn stored_task(repo: &Repo) -> Option<String> {
    let bytes = repo.read(TASK_REL, 4 * TASK_CHANGE_CAP as u64).ok()?;
    String::from_utf8(bytes)
        .ok()
        .filter(|t| !t.trim().is_empty())
}

pub fn run(g: &Global, task: Option<&str>, roles: bool, pr: Option<u64>) -> Result<ExitCode> {
    if pr.is_some() {
        return not_implemented("review --pr");
    }
    let command = if roles { "review --roles" } else { "review" };
    let (_, ws) = here(g)?;
    let repos = ws.loaded_repos()?;
    let (task, from) = match task {
        Some(t) => {
            let t: String = t.chars().take(TASK_CHANGE_CAP).collect();
            for (repo, _) in &repos {
                repo.write(TASK_REL, t.as_bytes())?;
            }
            (Some(t), "given")
        }
        None => match repos.iter().find_map(|(r, _)| stored_task(r)) {
            Some(t) => (Some(t), "stored"),
            None => (None, "none"),
        },
    };
    let mut c = Collected::default();
    for (repo, loaded) in &repos {
        collect(repo, loaded, &mut c)?;
    }
    let changes = c.reported.iter().any(|r| r.changes);
    let report = if changes || c.left > 0 {
        None
    } else {
        Some(next::next(
            &ws,
            task.as_deref(),
            Briefs::KeepWaiting,
            roles,
        )?)
    };
    let (mut then, mut clean) = match &report {
        None if changes => (
            format!("Fix the findings, then run stewards {command} again."),
            false,
        ),
        None => (
            format!(
                "Run stewards {command} again to read the {} result files left.",
                c.left
            ),
            false,
        ),
        Some(r) if !r.jobs.is_empty() => {
            let mut then = format!(
                "Start one subagent per handoff below, giving it exactly that sentence as its whole task; when they have finished, run stewards {command} again."
            );
            if r.jobs.iter().any(|j| j.waiting) {
                then += " A waiting job's brief has no submit yet: if its subagent already replied, it did not finish; start a new one with the same handoff.";
            }
            (then, false)
        }
        Some(r) if !r.check.failures.is_empty() => (
            format!(
                "Fix the check failures listed under next.check, then run stewards {command} again."
            ),
            false,
        ),
        Some(_) => ("done".to_string(), true),
    };
    if let Some(line) = report.as_ref().and_then(NextReport::unowned_line) {
        then = if clean {
            line
        } else {
            format!("{then} {line}")
        };
        clean = false;
    }
    let text = match &task {
        Some(t) => t.clone(),
        None => match repos.first() {
            Some((repo, _)) => git::task_text(&repo.root)?,
            None => String::new(),
        },
    };
    let written = emit(
        &Review {
            task: Task { text, from },
            hint: task.is_none().then_some(TASK_HINT),
            reported: c.reported,
            left: c.left,
            not_results_unshown: c.not_results_unshown,
            then,
            next: report,
        },
        g.human,
    );
    // only once the output is out: a failed write loses nothing. A file that cannot be removed
    // is reported again by the next call.
    if written {
        for (repo, rel) in &c.read {
            let _ = repo.remove(rel);
        }
        if clean {
            for (repo, _) in &repos {
                let _ = repo.remove(TASK_REL);
            }
        }
    }
    Ok(if clean {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shown_flattens_escapes_and_caps_every_text() {
        let v = shown_value(serde_json::json!({
            "a\u{202E}": ["x\ny", null, "z".repeat(3000)],
            "gone": null,
        }));
        assert_eq!(
            v,
            serde_json::json!({"a\\u{202E}": ["x y", null, "z".repeat(TEXT_SHOWN)]})
        );
        let long = Value::Array(vec![Value::Bool(true); 60]);
        assert_eq!(shown_value(long).as_array().unwrap().len(), LIST_SHOWN);
    }
}

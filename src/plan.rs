//! Architect plans and flow checks in `next`: running each check through the allowlist, storing
//! step results, and closing a plan whose steps all pass on fresh units.
use std::path::Path;

use anyhow::Result;
use serde::Serialize;

use crate::check::{fingerprint, is_stale};
use crate::git;
use crate::model::{Loaded, PLANS_DIR, StepResult};
use crate::next::FlowCheck;
use crate::route::Router;
use crate::run;
use crate::workspace::{Repo, Workspace};

#[derive(Debug, Clone, Serialize)]
pub struct PlanReport {
    pub id: String,
    pub title: String,
    /// All steps passed on fresh units: the plan file was deleted.
    pub closed: bool,
    pub steps: Vec<StepReport>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StepReport {
    pub unit: String,
    pub change: String,
    pub check: String,
    pub result: Option<StepResult>,
    /// The unit is stamped and unchanged since.
    pub fresh: bool,
    /// The check's output, when it ran in this call: the command's text, not stewards'.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// What to do for a step that has not passed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub todo: Option<String>,
}

/// One repo's plans and flow checks at `snapshot`. With `run_checks`, a step runs its check when
/// its unit is stale or never stamped, or its stored result is `fail`, and the result is written
/// back when it changed; a plan whose steps all pass on fresh units is deleted and reported
/// closed; a flow with a `check` and a stale unit runs it. Without, nothing runs or is written.
pub fn plans_and_flows(
    ws: &Workspace,
    repo: &Repo,
    loaded: &Loaded,
    snapshot: &str,
    run_checks: bool,
) -> Result<(Vec<PlanReport>, Vec<FlowCheck>)> {
    let flows: Vec<_> = loaded
        .repo
        .flows
        .iter()
        .filter_map(|f| Some((f, f.check.as_ref()?)))
        .collect();
    if loaded.plans.is_empty() && flows.is_empty() {
        return Ok((vec![], vec![]));
    }
    let router = Router::new(&loaded.units, &loaded.repo.docs_path)?;
    let entries = git::ls_tree(&repo.root, snapshot)?;
    let fresh = |unit: &str| {
        let reviewed = loaded.cards.get(unit).and_then(|c| c.reviewed.as_ref());
        reviewed.is_some() && !is_stale(reviewed, &fingerprint(&entries, unit, &router))
    };

    let mut plans = vec![];
    for (id, plan) in &loaded.plans {
        let mut plan = plan.clone();
        let mut changed = false;
        let mut steps = vec![];
        for step in &mut plan.steps {
            let fresh = fresh(&step.unit);
            let mut output = None;
            if run_checks && (!fresh || step.result == Some(StepResult::Fail)) {
                let allowed = run::allowed(loaded, Some(&step.unit));
                let (pass, out) = run_check(&repo.root, &allowed, &step.check)?;
                let result = Some(if pass {
                    StepResult::Pass
                } else {
                    StepResult::Fail
                });
                changed |= step.result != result;
                step.result = result;
                output = Some(out);
            }
            let brief = if ws.is_workspace {
                format!("stewards --repo {} brief {}", repo.name, step.unit)
            } else {
                format!("stewards brief {}", step.unit)
            };
            steps.push(StepReport {
                unit: ws.display_name(repo, &step.unit),
                change: step.change.clone(),
                check: step.check.clone(),
                result: step.result,
                fresh,
                output,
                // the change stays out of the command line: it is an answer's text
                todo: (step.result != Some(StepResult::Pass)).then(|| {
                    format!(
                        "make the change yourself, or run {brief} --role maintainer --task \
                         \"<change>\" to split it into developer tasks"
                    )
                }),
            });
        }
        let closed = run_checks
            && steps
                .iter()
                .all(|s| s.fresh && s.result == Some(StepResult::Pass));
        let rel = format!("{PLANS_DIR}/{id}.toml");
        if closed {
            repo.remove(&rel)?;
        } else if changed {
            repo.write(&rel, toml::to_string(&plan)?.as_bytes())?;
        }
        plans.push(PlanReport {
            id: ws.display_name(repo, id),
            title: plan.title,
            closed,
            steps,
        });
    }

    let mut flow_checks = vec![];
    if run_checks {
        let allowed = run::allowed(loaded, None);
        for (flow, check) in flows {
            let known_stale = |u: &String| loaded.units.iter().any(|d| &d.name == u) && !fresh(u);
            if flow.units.iter().any(known_stale) {
                let (pass, output) = run_check(&repo.root, &allowed, check)?;
                flow_checks.push(FlowCheck {
                    flow: ws.display_name(repo, &flow.name),
                    command: check.clone(),
                    pass,
                    output,
                });
            }
        }
    }
    Ok((plans, flow_checks))
}

/// Whether `cmd` passed and its output; a command the allowlist refuses (a plan or `repo.toml`
/// edited by hand) fails with the reason as its output.
fn run_check(root: &Path, allowed: &[run::Allowed], cmd: &str) -> Result<(bool, String)> {
    if let Err(problem) = run::check(cmd, allowed) {
        return Ok((false, format!("command_not_allowed: {problem}")));
    }
    let r = run::run(root, allowed, cmd)?;
    Ok((r.pass, r.output))
}

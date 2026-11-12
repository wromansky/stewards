// Graders: pure functions from run records to per-task verdicts.
use std::path::PathBuf;

use serde::Serialize;

use super::{
    Arm, Plan, RITUAL_NOTE, RITUAL_STEPS, RitualSteps, Run, RunRecord, RunRole, SetupNotes, Status,
};
use crate::output::Human;

/// Medians of `full` over `bare`: main context at most this share, total at most this share.
pub const MAIN_LIMIT: f64 = 0.8;
pub const TOTAL_LIMIT: f64 = 1.1;
/// `full` task 6 files opened over `bare` task 4 files opened.
pub const FILES_LIMIT: f64 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Pass,
    Warn,
    Fail,
    /// A compared run failed, timed out, has no transcript, or never started: no verdict on the numbers.
    Incomplete,
}

impl Verdict {
    fn as_str(self) -> &'static str {
        match self {
            Verdict::Pass => "pass",
            Verdict::Warn => "warn",
            Verdict::Fail => "fail",
            Verdict::Incomplete => "incomplete",
        }
    }
}

/// The median of `xs`; the mean of the middle two for an even count; `None` when empty.
pub fn median(xs: &[f64]) -> Option<f64> {
    let mut v = xs.to_vec();
    v.sort_by(f64::total_cmp);
    let n = v.len();
    match n {
        0 => None,
        _ if n % 2 == 1 => Some(v[n / 2]),
        _ => Some((v[n / 2 - 1] + v[n / 2]) / 2.0),
    }
}

/// The median of `f` over every run; `None` when any run has no parsed transcript.
fn median_of(runs: &[&RunRecord], f: impl Fn(&Run) -> Option<f64>) -> Option<f64> {
    let xs: Option<Vec<f64>> = runs
        .iter()
        .map(|r| r.result.as_ref().and_then(&f))
        .collect();
    median(&xs?)
}

/// Medians of one task in one arm.
#[derive(Debug, Clone, Serialize)]
pub struct ArmStats {
    pub arm: Arm,
    pub runs: usize,
    pub not_ok: usize,
    pub main_tokens: Option<f64>,
    pub total_tokens: Option<f64>,
    /// Main session only.
    pub files_opened: Option<f64>,
    /// The whole session tree.
    pub files_opened_tree: Option<f64>,
    pub peak_main_context: Option<f64>,
    pub cost_usd: Option<f64>,
    /// Each step present in a majority of runs; `None` outside the `full` arms.
    pub ritual: Option<RitualSteps>,
}

pub fn arm_stats(arm: Arm, runs: &[&RunRecord]) -> ArmStats {
    let ritual = arm.is_full().then(|| {
        RITUAL_STEPS
            .iter()
            .map(|&name| {
                let yes = runs
                    .iter()
                    .filter(|r| {
                        r.result
                            .as_ref()
                            .is_some_and(|x| x.ritual_steps.get(name) == Some(&true))
                    })
                    .count();
                (name, yes * 2 > runs.len())
            })
            .collect()
    });
    ArmStats {
        arm,
        runs: runs.len(),
        not_ok: runs.iter().filter(|r| r.status != Status::Ok).count(),
        main_tokens: median_of(runs, |r| Some(r.main_tokens.sum() as f64)),
        total_tokens: median_of(runs, |r| Some(r.total_tokens.sum() as f64)),
        files_opened: median_of(runs, |r| Some(r.files_opened.len() as f64)),
        files_opened_tree: median_of(runs, |r| Some(r.files_opened_tree.len() as f64)),
        peak_main_context: median_of(runs, |r| Some(r.peak_main_context as f64)),
        cost_usd: median_of(runs, |r| r.cost_usd),
        ritual,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Criterion {
    pub name: &'static str,
    /// `full` over `bare`, or for `ritual` the number of steps present.
    pub value: f64,
    pub limit: f64,
    pub ok: bool,
    /// Nothing to compare (`files_opened` 0 on both sides): not ok, and not a measured failure.
    pub unmeasured: bool,
}

/// A report-only ratio of `full` over `bare` medians, outside the verdict.
#[derive(Debug, Clone, Serialize)]
pub struct Ratio {
    pub name: &'static str,
    pub full: Option<f64>,
    pub bare: Option<f64>,
    /// `None` when either median is missing or `bare` is 0.
    pub ratio: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskGrade {
    pub task: u32,
    pub verdict: Verdict,
    /// Why the task is not a pass: failed criteria, or each run that makes it incomplete.
    pub reasons: Vec<String>,
    pub arms: Vec<ArmStats>,
    /// Empty when the verdict is `incomplete`.
    pub criteria: Vec<Criterion>,
    /// Cost and peak main context, `full` over `bare`; they do not change the verdict.
    pub report_only: Vec<Ratio>,
}

fn runs_of(records: &[RunRecord], task: u32, arm: Arm, role: RunRole) -> Vec<&RunRecord> {
    records
        .iter()
        .filter(|r| r.task == Some(task) && r.arm == arm && r.role == role)
        .collect()
}

/// Grades one task: `full` (or `full-no-developers`) against `bare` medians. Both token criteria
/// failing fails the task; any other failing criterion is a warning. A compared run (task 6 also
/// compares `bare` task 4, and counts its own task-5 setup runs) that is not ok, has no transcript,
/// or never started makes the task `incomplete`.
pub fn grade_task(task: u32, records: &[RunRecord], arms: &[Arm], runs: u32) -> TaskGrade {
    let stats: Vec<ArmStats> = arms
        .iter()
        .map(|&a| arm_stats(a, &runs_of(records, task, a, RunRole::Task)))
        .collect();
    let full = arms.iter().copied().find(|a| a.is_full());
    let mut reasons = vec![];
    let mut compared: Vec<(Arm, u32, RunRole)> = vec![];
    match full {
        Some(f) if arms.contains(&Arm::Bare) => {
            for a in [Arm::Bare, f] {
                compared.push((a, task, RunRole::Task));
                if task == 6 {
                    compared.push((a, 5, RunRole::Setup));
                }
            }
            if task == 6 {
                compared.push((Arm::Bare, 4, RunRole::Task));
            }
        }
        _ => reasons.push("needs both the bare and a full arm".to_string()),
    }
    for &(arm, t, role) in &compared {
        let rs = runs_of(records, t, arm, role);
        let what = match role {
            RunRole::Task => format!("{} task {t}", arm.as_str()),
            RunRole::Setup => format!("{} task {t} setup", arm.as_str()),
        };
        if (rs.len() as u32) < runs {
            reasons.push(format!("{what}: {} of {runs} runs started", rs.len()));
        }
        for r in rs {
            if r.status != Status::Ok {
                let why = r
                    .failure
                    .as_ref()
                    .map_or(String::new(), |f| format!(" ({f})"));
                reasons.push(format!("{what} run {}: {}{why}", r.run, r.status.as_str()));
            } else if r.result.is_none() {
                reasons.push(format!(
                    "{what} run {}: {}",
                    r.run,
                    r.error.as_deref().unwrap_or("no transcript")
                ));
            }
        }
    }
    let report_only = full
        .map(|f| {
            let stat = |a: Arm| stats.iter().find(|s| s.arm == a);
            let (b, f) = (stat(Arm::Bare), stat(f));
            let ratio = |name, m: fn(&ArmStats) -> Option<f64>| {
                let (full, bare) = (f.and_then(m), b.and_then(m));
                Ratio {
                    name,
                    full,
                    bare,
                    ratio: full.zip(bare.filter(|b| *b > 0.0)).map(|(f, b)| f / b),
                }
            };
            vec![
                ratio("cost_usd", |s| s.cost_usd),
                ratio("peak_main_context", |s| s.peak_main_context),
            ]
        })
        .unwrap_or_default();
    let incomplete = |reasons: Vec<String>, stats: Vec<ArmStats>| TaskGrade {
        task,
        verdict: Verdict::Incomplete,
        reasons,
        arms: stats,
        criteria: vec![],
        report_only: report_only.clone(),
    };
    let Some(full) = full.filter(|_| reasons.is_empty()) else {
        return incomplete(reasons, stats);
    };
    let stat = |a: Arm| stats.iter().find(|s| s.arm == a).expect("arm graded");
    let (b, f) = (stat(Arm::Bare), stat(full));
    let ratio = |x: Option<f64>, y: Option<f64>| match (x, y) {
        (Some(x), Some(y)) if y > 0.0 => x / y,
        (Some(0.0), Some(_)) => 0.0,
        _ => f64::INFINITY,
    };
    let mut criteria = vec![];
    let mut push = |name, value: f64, limit| {
        criteria.push(Criterion {
            name,
            value,
            limit,
            ok: value <= limit,
            unmeasured: false,
        })
    };
    push(
        "main_tokens",
        ratio(f.main_tokens, b.main_tokens),
        MAIN_LIMIT,
    );
    push(
        "total_tokens",
        ratio(f.total_tokens, b.total_tokens),
        TOTAL_LIMIT,
    );
    if task == 6 {
        let bare4 = arm_stats(Arm::Bare, &runs_of(records, 4, Arm::Bare, RunRole::Task));
        if f.files_opened == Some(0.0) && bare4.files_opened == Some(0.0) {
            criteria.push(Criterion {
                name: "files_opened",
                value: 0.0,
                limit: FILES_LIMIT,
                ok: false,
                unmeasured: true,
            });
            reasons.push("files_opened unmeasured: 0 files opened on both sides".into());
        } else {
            push(
                "files_opened",
                ratio(f.files_opened, bare4.files_opened),
                FILES_LIMIT,
            );
        }
    }
    if let Some(steps) = &f.ritual {
        let missing: Vec<&str> = RITUAL_STEPS
            .into_iter()
            .filter(|n| steps.get(n) != Some(&true))
            .collect();
        criteria.push(Criterion {
            name: "ritual",
            value: (RITUAL_STEPS.len() - missing.len()) as f64,
            limit: RITUAL_STEPS.len() as f64,
            ok: missing.is_empty(),
            unmeasured: false,
        });
        if !missing.is_empty() {
            reasons.push(format!("ritual steps missing: {}", missing.join(", ")));
        }
    }
    for c in criteria
        .iter()
        .filter(|c| !c.ok && !c.unmeasured && c.name != "ritual")
    {
        reasons.push(format!("{} {:.2} over {:.2}", c.name, c.value, c.limit));
    }
    let failed = |n: &str| criteria.iter().any(|c| c.name == n && !c.ok);
    let verdict = if failed("main_tokens") && failed("total_tokens") {
        Verdict::Fail
    } else if criteria.iter().any(|c| !c.ok) {
        Verdict::Warn
    } else {
        Verdict::Pass
    };
    TaskGrade {
        task,
        verdict,
        reasons,
        arms: stats,
        criteria,
        report_only,
    }
}

/// Fail when any task fails, else incomplete when any task is, else pass.
pub fn suite_verdict(grades: &[TaskGrade]) -> Verdict {
    let any = |v| grades.iter().any(|g| g.verdict == v);
    if any(Verdict::Fail) {
        Verdict::Fail
    } else if any(Verdict::Incomplete) || grades.is_empty() {
        Verdict::Incomplete
    } else {
        Verdict::Pass
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub results: PathBuf,
    pub harness: &'static str,
    /// The harness setup the sessions ran with (for Pi: the eval's agent directory).
    pub harness_setup: Option<String>,
    pub model: Option<String>,
    pub thinking: String,
    pub arms: Vec<Arm>,
    pub tasks: Vec<u32>,
    pub runs_per_arm: u32,
    pub sessions: usize,
    pub max_cost_usd: Option<f64>,
    /// Summed from the transcripts' own cost figures; sessions without one add nothing.
    pub spent_usd: f64,
    /// Sessions whose transcript could not be parsed: their cost is unknown, and after one
    /// `--max-cost` stops the eval, since it can no longer bound the spend.
    pub unknown_cost_sessions: Vec<String>,
    pub stopped_by_max_cost: bool,
    /// The `--setup-dir` copied for the `full` arms, when one was given.
    pub setup_dir: Option<PathBuf>,
    /// The adoption session's record, kept apart from the graded runs.
    pub adoption: Option<RunRecord>,
    pub setup: SetupNotes,
    /// How far to trust the ritual steps.
    pub ritual_note: &'static str,
    /// Runs whose session tree used a model other than the requested one.
    pub model_mismatches: Vec<String>,
    pub verdict: Verdict,
    pub grades: Vec<TaskGrade>,
}

pub fn summarize(
    plan: &Plan,
    records: &[RunRecord],
    stopped: bool,
    unknown_cost: &[String],
    setup: SetupNotes,
) -> Summary {
    let grades: Vec<TaskGrade> = plan
        .tasks
        .iter()
        .map(|&t| grade_task(t, records, &plan.arms, plan.runs))
        .collect();
    Summary {
        results: plan.results.clone(),
        harness: "pi",
        harness_setup: plan.harness_setup.clone(),
        model: plan
            .model
            .clone()
            .or_else(|| records.iter().find_map(|r| r.model.clone())),
        thinking: plan.thinking.clone(),
        arms: plan.arms.clone(),
        tasks: plan.tasks.clone(),
        runs_per_arm: plan.runs,
        sessions: records.len(),
        max_cost_usd: plan.max_cost,
        spent_usd: records
            .iter()
            .filter_map(|r| r.result.as_ref().and_then(|x| x.cost_usd))
            .sum(),
        unknown_cost_sessions: unknown_cost.to_vec(),
        stopped_by_max_cost: stopped,
        setup_dir: plan.setup_dir.clone(),
        adoption: records.iter().find(|r| r.task.is_none()).cloned(),
        setup,
        ritual_note: RITUAL_NOTE,
        model_mismatches: records
            .iter()
            .filter(|r| r.model_mismatch)
            .map(|r| {
                let models = r.result.as_ref().map_or(vec![], |x| x.models.clone());
                format!("{}: {}", r.label, models.join(", "))
            })
            .collect(),
        verdict: suite_verdict(&grades),
        grades,
    }
}

fn num(x: Option<f64>) -> String {
    x.map_or("null".into(), |x| format!("{x:.0}"))
}

fn usd(x: Option<f64>) -> String {
    x.map_or("null".into(), |c| format!("{c:.2}"))
}

/// The short human table written to `summary.md`.
pub fn render(s: &Summary) -> String {
    let mut out = vec![
        format!("# eval: {}", s.verdict.as_str()),
        String::new(),
        format!(
            "{} sessions, model {}, thinking {}, spent ${:.2}{}{}",
            s.sessions,
            s.model.as_deref().unwrap_or("(harness default)"),
            s.thinking,
            s.spent_usd,
            s.max_cost_usd
                .map_or(String::new(), |m| format!(" of ${m:.2}")),
            if s.stopped_by_max_cost {
                "; stopped by --max-cost"
            } else {
                ""
            }
        ),
        format!("results: {}", s.results.display()),
    ];
    if !s.unknown_cost_sessions.is_empty() {
        out.push(format!(
            "cost unknown (transcript not parsed): {}; --max-cost cannot bound the spend after one, so it stops the eval",
            s.unknown_cost_sessions.join(", ")
        ));
    }
    if let Some(h) = &s.harness_setup {
        out.push(h.clone());
    }
    if let Some(a) = &s.adoption {
        let r = a.result.as_ref();
        out.push(format!(
            "adoption ({}): {}, main {}, total {}, ${}",
            a.label,
            a.status.as_str(),
            num(r.map(|x| x.main_tokens.sum() as f64)),
            num(r.map(|x| x.total_tokens.sum() as f64)),
            usd(r.and_then(|x| x.cost_usd))
        ));
    }
    out.extend(s.setup.units.clone());
    if !s.setup.added_paths.is_empty() {
        out.push(format!(
            "adoption added: {}",
            s.setup.added_paths.join(", ")
        ));
    }
    if !s.setup.bug_gaps.is_empty() {
        out.push(format!(
            "bug gaps on the adopted cards (a bug task may start already known): {}",
            s.setup.bug_gaps.join("; ")
        ));
    }
    if !s.model_mismatches.is_empty() {
        out.push(format!(
            "other models used: {}",
            s.model_mismatches.join("; ")
        ));
    }
    out.push(s.ritual_note.to_string());
    out.push(String::new());
    out.push(
        "| task | arm | runs | not ok | main | total | peak main | files main | files tree | cost | ritual |"
            .into(),
    );
    out.push("|---|---|---|---|---|---|---|---|---|---|---|".into());
    let count = |x: Option<f64>| x.map_or("null".into(), |x| format!("{x}"));
    for g in &s.grades {
        for a in &g.arms {
            let ritual = a.ritual.as_ref().map_or("n/a".to_string(), |r| {
                let n = r.values().filter(|v| **v).count();
                format!("{n}/{}", RITUAL_STEPS.len())
            });
            out.push(format!(
                "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
                g.task,
                a.arm.as_str(),
                a.runs,
                a.not_ok,
                num(a.main_tokens),
                num(a.total_tokens),
                num(a.peak_main_context),
                count(a.files_opened),
                count(a.files_opened_tree),
                usd(a.cost_usd),
                ritual
            ));
        }
    }
    out.push(String::new());
    for g in &s.grades {
        let why = if g.reasons.is_empty() {
            String::new()
        } else {
            format!(": {}", g.reasons.join("; "))
        };
        let report: Vec<String> = g
            .report_only
            .iter()
            .map(|r| {
                format!(
                    "{} {}",
                    r.name,
                    r.ratio.map_or("null".into(), |x| format!("{x:.2}"))
                )
            })
            .collect();
        out.push(format!(
            "- task {}: {}{why} (report-only, full/bare: {})",
            g.task,
            g.verdict.as_str(),
            report.join(", ")
        ));
    }
    out.join("\n") + "\n"
}

impl Human for Summary {
    fn render(&self) -> String {
        render(self)
    }
}

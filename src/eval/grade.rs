// Graders: pure functions from run records to per-task verdicts.
use std::path::PathBuf;

use serde::Serialize;

use super::{Arm, Plan, RitualSteps, Run, RunRecord, RunRole, Status};
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
    pub files_opened: Option<f64>,
    pub cost_usd: Option<f64>,
    /// Each step present in a majority of runs; `None` outside the `full` arms.
    pub ritual: Option<RitualSteps>,
}

pub fn arm_stats(arm: Arm, runs: &[&RunRecord]) -> ArmStats {
    let ritual = arm.is_full().then(|| {
        let has = |name: &str| {
            let yes = runs
                .iter()
                .filter(|r| r.result.as_ref().is_some_and(|x| x.ritual_steps.get(name)))
                .count();
            yes * 2 > runs.len()
        };
        RitualSteps {
            status: has("status"),
            route: has("route"),
            brief: has("brief"),
            next: has("next"),
            submit: has("submit"),
            check_green: has("check_green"),
        }
    });
    ArmStats {
        arm,
        runs: runs.len(),
        not_ok: runs.iter().filter(|r| r.status != Status::Ok).count(),
        main_tokens: median_of(runs, |r| Some(r.main_tokens.sum() as f64)),
        total_tokens: median_of(runs, |r| Some(r.total_tokens.sum() as f64)),
        files_opened: median_of(runs, |r| Some(r.files_opened.len() as f64)),
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
                reasons.push(format!("{what} run {}: {}", r.run, r.status.as_str()));
            } else if r.result.is_none() {
                reasons.push(format!(
                    "{what} run {}: {}",
                    r.run,
                    r.error.as_deref().unwrap_or("no transcript")
                ));
            }
        }
    }
    let incomplete = |reasons: Vec<String>, stats: Vec<ArmStats>| TaskGrade {
        task,
        verdict: Verdict::Incomplete,
        reasons,
        arms: stats,
        criteria: vec![],
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
        push(
            "files_opened",
            ratio(f.files_opened, bare4.files_opened),
            FILES_LIMIT,
        );
    }
    if let Some(steps) = f.ritual {
        let present = RitualSteps::NAMES.iter().filter(|n| steps.get(n)).count();
        let missing: Vec<&str> = RitualSteps::NAMES
            .into_iter()
            .filter(|n| !steps.get(n))
            .collect();
        criteria.push(Criterion {
            name: "ritual",
            value: present as f64,
            limit: RitualSteps::NAMES.len() as f64,
            ok: missing.is_empty(),
        });
        if !missing.is_empty() {
            reasons.push(format!("ritual steps missing: {}", missing.join(", ")));
        }
    }
    for c in criteria.iter().filter(|c| !c.ok && c.name != "ritual") {
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

/// The adoption session's numbers, kept apart from the graded runs.
#[derive(Debug, Clone, Serialize)]
pub struct Adoption {
    pub label: String,
    pub status: Status,
    pub main_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub results: PathBuf,
    pub harness: &'static str,
    pub model: Option<String>,
    pub arms: Vec<Arm>,
    pub tasks: Vec<u32>,
    pub runs_per_arm: u32,
    pub sessions: usize,
    pub max_cost_usd: Option<f64>,
    /// Summed from the transcripts' own cost figures; sessions without one add nothing.
    pub spent_usd: f64,
    pub stopped_by_max_cost: bool,
    /// The `--setup-dir` copied for the `full` arms, when one was given.
    pub setup_dir: Option<PathBuf>,
    pub adoption: Option<Adoption>,
    /// How the `full` arm's units were set, in plain words.
    pub units_note: Option<String>,
    pub verdict: Verdict,
    /// One line per task with the verdict `warn`.
    pub warnings: Vec<String>,
    pub grades: Vec<TaskGrade>,
}

pub fn summarize(
    plan: &Plan,
    records: &[RunRecord],
    stopped: bool,
    units_note: Option<String>,
) -> Summary {
    let grades: Vec<TaskGrade> = plan
        .tasks
        .iter()
        .map(|&t| grade_task(t, records, &plan.arms, plan.runs))
        .collect();
    let adoption = records.iter().find(|r| r.task.is_none()).map(|r| Adoption {
        label: r.label.clone(),
        status: r.status,
        main_tokens: r.result.as_ref().map(|x| x.main_tokens.sum()),
        total_tokens: r.result.as_ref().map(|x| x.total_tokens.sum()),
        cost_usd: r.result.as_ref().and_then(|x| x.cost_usd),
    });
    Summary {
        results: plan.results.clone(),
        harness: "pi",
        model: plan
            .model
            .clone()
            .or_else(|| records.iter().find_map(|r| r.model.clone())),
        arms: plan.arms.clone(),
        tasks: plan.tasks.clone(),
        runs_per_arm: plan.runs,
        sessions: records.len(),
        max_cost_usd: plan.max_cost,
        spent_usd: records
            .iter()
            .filter_map(|r| r.result.as_ref().and_then(|x| x.cost_usd))
            .sum(),
        stopped_by_max_cost: stopped,
        setup_dir: plan.setup_dir.clone(),
        adoption,
        units_note,
        verdict: suite_verdict(&grades),
        warnings: grades
            .iter()
            .filter(|g| g.verdict == Verdict::Warn)
            .map(|g| format!("task {}: {}", g.task, g.reasons.join("; ")))
            .collect(),
        grades,
    }
}

fn num(x: Option<f64>) -> String {
    x.map_or("null".into(), |x| format!("{x:.0}"))
}

/// The short human table written to `summary.md`.
pub fn render(s: &Summary) -> String {
    let mut out = vec![
        format!("# eval: {}", s.verdict.as_str()),
        String::new(),
        format!(
            "{} sessions, model {}, spent ${:.2}{}{}",
            s.sessions,
            s.model.as_deref().unwrap_or("(harness default)"),
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
    if let Some(a) = &s.adoption {
        out.push(format!(
            "adoption ({}): {}, main {}, total {}, ${}",
            a.label,
            a.status.as_str(),
            num(a.main_tokens.map(|x| x as f64)),
            num(a.total_tokens.map(|x| x as f64)),
            a.cost_usd.map_or("null".into(), |c| format!("{c:.2}"))
        ));
    }
    if let Some(note) = &s.units_note {
        out.push(note.clone());
    }
    out.push(String::new());
    out.push("| task | arm | runs | not ok | main | total | files | cost | ritual |".into());
    out.push("|---|---|---|---|---|---|---|---|---|".into());
    for g in &s.grades {
        for a in &g.arms {
            let ritual = a.ritual.map_or("n/a".to_string(), |r| {
                let n = RitualSteps::NAMES.iter().filter(|n| r.get(n)).count();
                format!("{n}/6")
            });
            out.push(format!(
                "| {} | {} | {} | {} | {} | {} | {} | {} | {} |",
                g.task,
                a.arm.as_str(),
                a.runs,
                a.not_ok,
                num(a.main_tokens),
                num(a.total_tokens),
                a.files_opened.map_or("null".into(), |x| format!("{x}")),
                a.cost_usd.map_or("null".into(), |c| format!("{c:.2}")),
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
        out.push(format!("- task {}: {}{why}", g.task, g.verdict.as_str()));
    }
    out.join("\n") + "\n"
}

impl Human for Summary {
    fn render(&self) -> String {
        render(self)
    }
}

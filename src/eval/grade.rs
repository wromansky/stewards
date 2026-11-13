// Graders: pure functions from run records to per-task verdicts.
use std::path::PathBuf;

use serde::Serialize;

use super::{
    Arm, Plan, RITUAL_NOTE, Run, RunRecord, RunRole, SetupNotes, Status, Suite, UnitReview,
};
use crate::output::Human;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Pass,
    Fail,
    /// A compared run failed, timed out, has no transcript, or never started: no verdict.
    Incomplete,
    /// A read-only task: no gate applies.
    #[serde(rename = "n/a")]
    NotApplicable,
}

impl Verdict {
    fn as_str(self) -> &'static str {
        match self {
            Verdict::Pass => "pass",
            Verdict::Fail => "fail",
            Verdict::Incomplete => "incomplete",
            Verdict::NotApplicable => "n/a",
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
    /// Child sessions of each run with a parsed transcript, in run order.
    pub subagent_sessions: Vec<usize>,
}

/// Medians of `runs`, all of one task.
pub fn arm_stats(arm: Arm, runs: &[&RunRecord]) -> ArmStats {
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
        subagent_sessions: runs
            .iter()
            .filter_map(|r| r.result.as_ref().map(|x| x.subagent_sessions))
            .collect(),
    }
}

/// A report-only ratio of `full` over `bare` medians.
#[derive(Debug, Clone, Serialize)]
pub struct Ratio {
    pub name: &'static str,
    pub full: Option<f64>,
    pub bare: Option<f64>,
    /// `None` when either median is missing or `bare` is 0.
    pub ratio: Option<f64>,
}

/// The `full` runs of an edit task that passed the gate, of those recorded.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct GateCount {
    pub passed: usize,
    pub runs: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskGrade {
    pub task: u32,
    pub verdict: Verdict,
    /// Why the task is not a pass: each run that makes it incomplete, or each run that failed the
    /// gate and what it missed.
    pub reasons: Vec<String>,
    pub arms: Vec<ArmStats>,
    /// `None` (n/a) on a read-only task or without a `full` arm.
    pub gate: Option<GateCount>,
    /// Main tokens, total tokens, cost, peak main context, and (with `files_opened_vs_bare`) main
    /// files opened, `full` over `bare`: reported, not judged.
    pub overhead: Vec<Ratio>,
}

fn runs_of(records: &[RunRecord], task: u32, arm: Arm, role: RunRole) -> Vec<&RunRecord> {
    records
        .iter()
        .filter(|r| r.task == Some(task) && r.arm == arm && r.role == role)
        .collect()
}

/// What one `full` run of an edit task misses of the gate: the runner saw a change, `next` ran,
/// the review brief was dispatched, a child ran the reviewer `submit`, and the runner's own
/// `next` is clean after the session. Empty when it passes.
pub fn gate_misses(r: &RunRecord) -> Vec<&'static str> {
    let step = |s: &str| {
        r.result
            .as_ref()
            .is_some_and(|x| x.ritual_steps.get(s) == Some(&true))
    };
    let mut out = vec![];
    if !r.changed {
        out.push("changed nothing");
    }
    match (step("next"), step("review_dispatched")) {
        (false, false) => {
            out.extend(["`stewards next` never ran", "the review was not dispatched"])
        }
        (false, true) => out.push("`stewards next` never ran"),
        (true, false) => out.push("`next` ran but the review was not dispatched"),
        (true, true) => {}
    }
    if !step("reviewer_submit") {
        out.push("no child ran `stewards submit --role reviewer`");
    }
    if !step("check_green") {
        out.push("the runner's `stewards next --no-brief` did not exit 0");
    }
    out
}

/// Grades one task of `suite`. A compared run (`bare` and `full`, their `setup` runs, and the
/// `files_opened_vs_bare` task in `bare`) that is not ok, has no transcript, or never started
/// makes the task `incomplete`. Otherwise a read-only task is `n/a`, and an edit task passes when
/// every `full` run passes the gate (`gate_misses`), else fails. The overhead ratios never change
/// the verdict.
pub fn grade_task(
    suite: &Suite,
    task: u32,
    records: &[RunRecord],
    arms: &[Arm],
    runs: u32,
) -> TaskGrade {
    let facts = suite.task(task).copied().unwrap_or_default();
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
                if let Some(s) = facts.setup {
                    compared.push((a, s, RunRole::Setup));
                }
            }
            if let Some(v) = facts.files_opened_vs_bare {
                compared.push((Arm::Bare, v, RunRole::Task));
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
    let incomplete = !reasons.is_empty();
    let stat = |a: Arm| stats.iter().find(|s| s.arm == a);
    let overhead = full
        .map(|f| {
            let (b, f) = (stat(Arm::Bare), stat(f));
            let ratio = |name, full: Option<f64>, bare: Option<f64>| Ratio {
                name,
                full,
                bare,
                ratio: full.zip(bare.filter(|b| *b > 0.0)).map(|(f, b)| f / b),
            };
            let both =
                |name, m: fn(&ArmStats) -> Option<f64>| ratio(name, f.and_then(m), b.and_then(m));
            let mut out = vec![
                both("main_tokens", |s| s.main_tokens),
                both("total_tokens", |s| s.total_tokens),
                both("cost_usd", |s| s.cost_usd),
                both("peak_main_context", |s| s.peak_main_context),
            ];
            if let Some(v) = facts.files_opened_vs_bare {
                let bare_v = arm_stats(Arm::Bare, &runs_of(records, v, Arm::Bare, RunRole::Task));
                out.push(ratio(
                    "files_opened",
                    f.and_then(|s| s.files_opened),
                    bare_v.files_opened,
                ));
            }
            out
        })
        .unwrap_or_default();
    let gate = full.filter(|_| !facts.read_only).map(|f| {
        let rs = runs_of(records, task, f, RunRole::Task);
        let mut passed = 0;
        for r in &rs {
            let misses = gate_misses(r);
            if misses.is_empty() {
                passed += 1;
            } else if r.status == Status::Ok && r.result.is_some() {
                reasons.push(format!(
                    "{} run {}: {}",
                    f.as_str(),
                    r.run,
                    misses.join("; ")
                ));
            }
        }
        GateCount {
            passed,
            runs: rs.len(),
        }
    });
    let verdict = match gate {
        _ if incomplete => Verdict::Incomplete,
        None => Verdict::NotApplicable,
        Some(g) if g.passed == g.runs => Verdict::Pass,
        Some(_) => Verdict::Fail,
    };
    TaskGrade {
        task,
        verdict,
        reasons,
        arms: stats,
        gate,
        overhead,
    }
}

/// Fail when any task fails, else incomplete when any task is (or there are none), else pass:
/// every edit task's gate passed and no run is incomplete.
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

/// One overhead ratio's median across the tasks that are not incomplete.
#[derive(Debug, Clone, Serialize)]
pub struct OverheadMedian {
    pub name: &'static str,
    pub median: Option<f64>,
    /// Tasks with this ratio measured.
    pub tasks: usize,
}

/// Each overhead ratio's median across `grades` that are not incomplete, in first-seen order.
pub fn overhead_medians(grades: &[TaskGrade]) -> Vec<OverheadMedian> {
    let mut by_name: Vec<(&'static str, Vec<f64>)> = vec![];
    for g in grades.iter().filter(|g| g.verdict != Verdict::Incomplete) {
        for r in &g.overhead {
            let at = match by_name.iter().position(|(n, _)| *n == r.name) {
                Some(at) => at,
                None => {
                    by_name.push((r.name, vec![]));
                    by_name.len() - 1
                }
            };
            by_name[at].1.extend(r.ratio);
        }
    }
    by_name
        .into_iter()
        .map(|(name, xs)| OverheadMedian {
            name,
            median: median(&xs),
            tasks: xs.len(),
        })
        .collect()
}

/// The reviewer outcomes of one `full` task run.
#[derive(Debug, Clone, Serialize)]
pub struct RunReviews {
    pub label: String,
    pub reviews: Vec<UnitReview>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub results: PathBuf,
    pub suite: String,
    /// A `repo` suite's pinned commit.
    pub commit: Option<String>,
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
    /// Runs where a session of the tree ran at a thinking level other than the requested one.
    pub thinking_mismatches: Vec<String>,
    /// Runs with a session that recorded no thinking level, as `label: sessions`.
    pub thinking_unknown: Vec<String>,
    /// Shell commands that read git history (`reads_git_history`), as `label: command`
    /// (flagged, not failed).
    pub git_history: Vec<String>,
    /// Report-only: the adoption session's total tokens over the median total of the parsed
    /// `bare` task runs, so a reader can amortize adoption.
    pub adoption_in_bare_tasks: Option<f64>,
    pub verdict: Verdict,
    pub grades: Vec<TaskGrade>,
    /// Report-only: each overhead ratio's median across tasks.
    pub overhead: Vec<OverheadMedian>,
    /// Report-only: the reviewer outcomes of each `full` task run.
    pub reviews: Vec<RunReviews>,
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
        .map(|&t| grade_task(&plan.suite, t, records, &plan.arms, plan.runs))
        .collect();
    let adoption = records.iter().find(|r| r.task.is_none()).cloned();
    let bare_totals: Vec<f64> = records
        .iter()
        .filter(|r| r.arm == Arm::Bare && r.role == RunRole::Task)
        .filter_map(|r| r.result.as_ref())
        .map(|x| x.total_tokens.sum() as f64)
        .collect();
    let adoption_in_bare_tasks = adoption
        .as_ref()
        .and_then(|a| a.result.as_ref())
        .zip(median(&bare_totals).filter(|m| *m > 0.0))
        .map(|(a, m)| a.total_tokens.sum() as f64 / m);
    Summary {
        results: plan.results.clone(),
        suite: plan.suite.name.clone(),
        commit: plan.suite.commit().map(String::from),
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
        adoption,
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
        thinking_mismatches: records
            .iter()
            .filter(|r| r.thinking_mismatch)
            .map(|r| {
                let levels = r
                    .result
                    .as_ref()
                    .map_or(vec![], |x| x.thinking_levels.clone());
                format!("{}: {}", r.label, levels.join(", "))
            })
            .collect(),
        thinking_unknown: records
            .iter()
            .filter_map(|r| {
                let s = &r.result.as_ref()?.sessions_without_thinking;
                (!s.is_empty()).then(|| format!("{}: {}", r.label, s.join(", ")))
            })
            .collect(),
        git_history: records
            .iter()
            .flat_map(|r| {
                r.result.iter().flat_map(|x| {
                    x.git_history_commands
                        .iter()
                        .map(|c| format!("{}: {c}", r.label))
                })
            })
            .collect(),
        adoption_in_bare_tasks,
        verdict: suite_verdict(&grades),
        overhead: overhead_medians(&grades),
        reviews: records
            .iter()
            .filter(|r| r.arm.is_full() && r.role == RunRole::Task)
            .map(|r| RunReviews {
                label: r.label.clone(),
                reviews: r.reviews.clone(),
            })
            .collect(),
        grades,
    }
}

fn num(x: Option<f64>) -> String {
    x.map_or("null".into(), |x| format!("{x:.0}"))
}

fn ratio(x: Option<f64>) -> String {
    x.map_or("null".into(), |x| format!("{x:.2}"))
}

fn usd(x: Option<f64>) -> String {
    x.map_or("null".into(), |c| format!("{c:.4}"))
}

/// The short human table written to `summary.md`.
pub fn render(s: &Summary) -> String {
    let mut out = vec![
        format!("# eval: {}", s.verdict.as_str()),
        String::new(),
        format!(
            "{} sessions, model {}, thinking {}, spent ${:.4}{}{}",
            s.sessions,
            s.model.as_deref().unwrap_or("(harness default)"),
            s.thinking,
            s.spent_usd,
            s.max_cost_usd
                .map_or(String::new(), |m| format!(" of ${m:.4}")),
            if s.stopped_by_max_cost {
                "; stopped by --max-cost"
            } else {
                ""
            }
        ),
        format!(
            "suite: {}{}{}",
            s.suite,
            s.commit
                .as_ref()
                .map_or(String::new(), |c| format!(" at {c}")),
            s.setup
                .resolved_commit
                .as_ref()
                .map_or(String::new(), |c| format!(" ({c})"))
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
    if let Some(b) = s.setup.build_s {
        out.push(format!(
            "build cache: one build, {b:.0} s, copied into every run (setup, outside every ratio)"
        ));
    }
    out.extend(s.setup.units.clone());
    if !s.setup.added_paths.is_empty() {
        out.push(format!(
            "adoption added: {}",
            s.setup.added_paths.join(", ")
        ));
    }
    if let Some(x) = s.adoption_in_bare_tasks {
        out.push(format!(
            "adoption total tokens = {x:.1} median bare task totals (report-only)"
        ));
    }
    if !s.setup.gaps.is_empty() {
        out.push(format!(
            "gaps on the adopted cards (a task whose bug or answer is listed started known): {}",
            s.setup.gaps.join("; ")
        ));
    }
    if !s.setup.flows.is_empty() {
        out.push(format!(
            "flows adoption recorded: {}",
            s.setup.flows.join("; ")
        ));
    }
    if !s.model_mismatches.is_empty() {
        out.push(format!(
            "other models used: {}",
            s.model_mismatches.join("; ")
        ));
    }
    if !s.thinking_mismatches.is_empty() {
        out.push(format!(
            "other thinking levels used: {}",
            s.thinking_mismatches.join("; ")
        ));
    }
    if !s.thinking_unknown.is_empty() {
        out.push(format!(
            "no thinking level recorded: {}",
            s.thinking_unknown.join("; ")
        ));
    }
    if !s.git_history.is_empty() {
        out.push(format!("git history read: {}", s.git_history.join("; ")));
    }
    out.push(s.ritual_note.to_string());
    out.push(String::new());
    out.push(
        "| task | arm | runs | not ok | main | total | peak main | files main | files tree | cost | subagents |"
            .into(),
    );
    out.push("|---|---|---|---|---|---|---|---|---|---|---|".into());
    let count = |x: Option<f64>| x.map_or("null".into(), |x| format!("{x}"));
    for g in &s.grades {
        for a in &g.arms {
            let subagents: Vec<String> = a.subagent_sessions.iter().map(usize::to_string).collect();
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
                subagents.join(",")
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
        let gate = g.gate.map_or(String::new(), |c| {
            format!(" (gate {}/{})", c.passed, c.runs)
        });
        let overhead: Vec<String> = g
            .overhead
            .iter()
            .map(|r| format!("{} {}", r.name, ratio(r.ratio)))
            .collect();
        out.push(format!(
            "- task {}: {}{gate}{why} (overhead, full/bare: {})",
            g.task,
            g.verdict.as_str(),
            overhead.join(", ")
        ));
    }
    let medians: Vec<String> = s
        .overhead
        .iter()
        .map(|m| format!("{} {} ({} tasks)", m.name, ratio(m.median), m.tasks))
        .collect();
    out.push(format!(
        "- overhead medians across tasks, full/bare (reported, not judged): {}",
        medians.join(", ")
    ));
    if !s.reviews.is_empty() {
        out.push(String::new());
        out.push("review outcomes (report-only):".into());
    }
    for r in &s.reviews {
        let units: Vec<String> = r
            .reviews
            .iter()
            .map(|u| {
                format!(
                    "round {} {} {} ({} findings, bugs: {}, tests {})",
                    u.round,
                    u.unit,
                    u.verdict,
                    u.findings,
                    if u.bugs.is_empty() {
                        "none".to_string()
                    } else {
                        u.bugs.join(", ")
                    },
                    if u.tests_ran { "ran" } else { "not seen" }
                )
            })
            .collect();
        out.push(format!(
            "- {}: {}",
            r.label,
            if units.is_empty() {
                "no reviewer submit".to_string()
            } else {
                units.join("; ")
            }
        ));
    }
    out.join("\n") + "\n"
}

impl Human for Summary {
    fn render(&self) -> String {
        render(self)
    }
}

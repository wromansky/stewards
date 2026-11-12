#[allow(dead_code)] // each test crate uses a subset of the helpers
mod common;

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use common::copy_dir;
use stewards::eval::grade::{grade_task, median};
use stewards::eval::pi::{Pi, parse};
use stewards::eval::{
    Action, Arm, Driver, Plan, RitualSteps, Run, RunRecord, RunRole, Session, Status, Tokens,
    Verdict, execute, path_without_stewards, ritual_steps, stewards_subcommands,
    without_developers, would_exceed,
};
use stewards::output::CliError;
use tempfile::TempDir;

fn sample(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/transcripts/pi")
        .join(name)
}

/// The parent session file of a recorded sample.
fn main_file(dir: &Path) -> PathBuf {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension() == Some(OsStr::new("jsonl")))
        .unwrap()
}

#[test]
fn pi_sample_counts_child_tokens_in_total_and_not_in_main() {
    let run = parse(&main_file(&sample("subagent")), 1.5).unwrap();
    let main = Tokens {
        input: 12,
        output: 743,
        cache_read: 57992,
        cache_write: 1476,
    };
    let child = Tokens {
        input: 4,
        output: 416,
        cache_read: 0,
        cache_write: 5728,
    };
    assert_eq!(run.main_tokens, main);
    let mut total = main;
    total.add(child);
    assert_eq!(run.total_tokens, total);
    assert_eq!(run.total_tokens.sum(), 60223 + 6148);
    assert_eq!(run.tool_calls, ["bash", "read", "subagent"]);
    assert_eq!(run.files_opened, ["AGENTS.md"]);
    assert_eq!(run.shell_commands, ["echo probe"]);
    assert_eq!(run.model.as_deref(), Some("anthropic/claude-haiku-5-5"));
    assert!((run.cost_usd.unwrap() - (0.00113712 + 0.0009244)).abs() < 1e-6);
    assert_eq!(run.duration_s, 1.5);
    assert_eq!(run.ritual_steps, RitualSteps::default());
}

#[test]
fn a_fork_copy_of_the_parent_is_counted_once() {
    let tmp = TempDir::new().unwrap();
    copy_dir(&sample("subagent"), tmp.path());
    let parent = main_file(tmp.path());
    let before = parse(&parent, 0.0).unwrap();
    let forks = parent.with_extension("").join("forks/f1");
    std::fs::create_dir_all(&forks).unwrap();
    let extra = r#"{"type":"message","id":"fork0001","parentId":null,"timestamp":"2026-11-12T19:30:00.000Z","message":{"role":"assistant","content":[],"usage":{"input":1,"output":2,"cacheRead":3,"cacheWrite":4,"cost":{"total":0.5}}}}"#;
    let copy = std::fs::read_to_string(&parent).unwrap() + extra + "\n";
    std::fs::write(forks.join("fork.jsonl"), copy).unwrap();
    let after = parse(&parent, 0.0).unwrap();
    assert_eq!(after.main_tokens, before.main_tokens);
    assert_eq!(after.total_tokens.sum(), before.total_tokens.sum() + 10);
    assert!((after.cost_usd.unwrap() - before.cost_usd.unwrap() - 0.5).abs() < 1e-9);
}

#[test]
fn pi_ritual_sample_shows_every_step_across_parent_and_child() {
    let run = parse(&main_file(&sample("ritual")), 0.0).unwrap();
    assert_eq!(
        run.ritual_steps,
        RitualSteps {
            status: true,
            route: true,
            brief: true,
            next: true,
            submit: true,
            check_green: true,
        }
    );
    // the usage record counts; the child's two messages count in total only
    assert_eq!(
        run.main_tokens,
        Tokens {
            input: 81,
            output: 41,
            cache_read: 850,
            cache_write: 160
        }
    );
    assert_eq!(run.total_tokens.sum(), 81 + 41 + 850 + 160 + 2 * 45);
    assert_eq!(run.files_opened, ["api/items.py"]);
    assert_eq!(run.shell_commands.len(), 4);
}

fn shell(command: &str, ok: bool, at: &str) -> Action {
    Action::Shell {
        command: command.into(),
        ok,
        at: at.into(),
    }
}

fn dispatch(text: &str) -> Action {
    Action::Dispatch { text: text.into() }
}

fn edit(at: &str) -> Action {
    Action::Edit { at: at.into() }
}

#[test]
fn each_ritual_step_is_detected_alone_and_absent_otherwise() {
    let t = "2026-11-12T20:00:00.000Z";
    let cases: [(&str, Vec<Action>); 5] = [
        ("status", vec![shell("stewards status", false, t)]),
        (
            "route",
            vec![shell("cd /r && stewards route a.py", true, t)],
        ),
        (
            "brief",
            vec![dispatch(
                r#"{"task":"Read /r/.stewards/briefs/api-maintainer.md"}"#,
            )],
        ),
        // a failing next is still the step, but not a green check
        ("next", vec![shell("stewards next", false, t)]),
        (
            "submit",
            vec![shell(
                "stewards submit api --role tester --file a.json",
                true,
                t,
            )],
        ),
    ];
    for (name, actions) in &cases {
        let steps = ritual_steps(actions);
        for other in RitualSteps::NAMES {
            // submit implies brief; nothing else implies another step
            let expected = other == *name || (*name == "submit" && other == "brief");
            assert_eq!(steps.get(other), expected, "{name}: {other}");
        }
    }
    assert_eq!(ritual_steps(&[]), RitualSteps::default());
    let green = ritual_steps(&[shell("stewards next", true, t)]);
    assert!(green.check_green && green.next);
}

#[test]
fn check_green_needs_a_passing_next_after_the_last_edit() {
    let early = "2026-11-12T20:00:00.000Z";
    let late = "2026-11-12T20:05:00.000Z";
    let green_after = [edit(early), shell("stewards next", true, late)];
    assert!(ritual_steps(&green_after).check_green);
    let edit_after = [shell("stewards next", true, early), edit(late)];
    assert!(!ritual_steps(&edit_after).check_green);
    let red_after = [edit(early), shell("stewards next", false, late)];
    assert!(!ritual_steps(&red_after).check_green);
}

#[test]
fn stewards_subcommands_reads_argv_not_text() {
    assert_eq!(
        stewards_subcommands("echo 'run stewards next'"),
        Vec::<&str>::new()
    );
    assert_eq!(
        stewards_subcommands("grep stewards README.md"),
        Vec::<&str>::new()
    );
    assert_eq!(
        stewards_subcommands(
            "cd /r && A=1 stewards -C /r --human next | head; /x/bin/stewards status"
        ),
        ["next", "status"]
    );
    assert_eq!(stewards_subcommands("stewards --repo api check"), ["check"]);
}

#[test]
fn median_of_three_and_of_even_counts() {
    assert_eq!(median(&[300.0, 100.0, 200.0]), Some(200.0));
    assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), Some(2.5));
    assert_eq!(median(&[]), None);
}

fn run_with(main: u64, total: u64, files: usize, steps: RitualSteps) -> Run {
    let t = |n| Tokens {
        input: n,
        ..Tokens::default()
    };
    Run {
        main_tokens: t(main),
        total_tokens: t(total),
        tool_calls: vec![],
        files_opened: (0..files).map(|n| format!("f{n}")).collect(),
        shell_commands: vec![],
        ritual_steps: steps,
        cost_usd: Some(0.1),
        duration_s: 1.0,
        model: None,
    }
}

fn record(task: u32, arm: Arm, role: RunRole, run: u32, result: Run) -> RunRecord {
    RunRecord {
        label: format!("{}-t{task}-r{run}", arm.as_str()),
        task: Some(task),
        arm,
        role,
        run,
        model: None,
        status: Status::Ok,
        transcript: None,
        workdir: PathBuf::new(),
        result: Some(result),
        error: None,
    }
}

const ALL: RitualSteps = RitualSteps {
    status: true,
    route: true,
    brief: true,
    next: true,
    submit: true,
    check_green: true,
};

/// Three runs per arm of `task`: bare at 1000 main and 1000 total, full at the medians given.
fn records(task: u32, full_main: u64, full_total: u64, steps: RitualSteps) -> Vec<RunRecord> {
    let mut out = vec![];
    for run in 1..=3 {
        // the spread around the median must not matter
        let spread = [0, 1, 0][run as usize - 1] * 5000;
        out.push(record(
            task,
            Arm::Bare,
            RunRole::Task,
            run,
            run_with(1000 + spread, 1000 + spread, 10, RitualSteps::default()),
        ));
        out.push(record(
            task,
            Arm::Full,
            RunRole::Task,
            run,
            run_with(full_main + spread, full_total + spread, 4, steps),
        ));
    }
    out
}

const ARMS: [Arm; 2] = [Arm::Bare, Arm::Full];

#[test]
fn token_criteria_boundaries() {
    let grade = |main, total| grade_task(1, &records(1, main, total, ALL), &ARMS, 3);
    let at_limits = grade(800, 1100);
    assert_eq!(at_limits.verdict, Verdict::Pass, "{:?}", at_limits.reasons);
    assert_eq!(at_limits.criteria.len(), 3);
    let main_over = grade(801, 1100);
    assert_eq!(main_over.verdict, Verdict::Warn);
    assert!(main_over.reasons[0].starts_with("main_tokens"));
    assert_eq!(grade(800, 1101).verdict, Verdict::Warn);
    assert_eq!(grade(801, 1101).verdict, Verdict::Fail);
}

#[test]
fn a_missing_ritual_step_is_a_warning() {
    let steps = RitualSteps {
        submit: false,
        ..ALL
    };
    let g = grade_task(1, &records(1, 500, 900, steps), &ARMS, 3);
    assert_eq!(g.verdict, Verdict::Warn);
    assert_eq!(g.reasons, ["ritual steps missing: submit"]);
}

/// Task 6 records with the full arm's task-6 files opened at `files`, against bare task 4 at 10.
fn task6(files: usize) -> Vec<RunRecord> {
    let mut out = records(6, 500, 900, ALL);
    for r in out.iter_mut().filter(|r| r.arm == Arm::Full) {
        r.result.as_mut().unwrap().files_opened = (0..files).map(|n| format!("f{n}")).collect();
    }
    for run in 1..=3 {
        for arm in ARMS {
            out.push(record(5, arm, RunRole::Setup, run, run_with(1, 1, 0, ALL)));
        }
        out.push(record(
            4,
            Arm::Bare,
            RunRole::Task,
            run,
            run_with(1000, 1000, 10, RitualSteps::default()),
        ));
    }
    out
}

#[test]
fn task_6_files_opened_is_at_most_half_of_bare_task_4() {
    let half = grade_task(6, &task6(5), &ARMS, 3);
    assert_eq!(half.verdict, Verdict::Pass, "{:?}", half.reasons);
    let over = grade_task(6, &task6(6), &ARMS, 3);
    assert_eq!(over.verdict, Verdict::Warn);
    assert!(over.reasons[0].starts_with("files_opened 0.60"));
}

#[test]
fn a_timed_out_or_missing_run_makes_the_task_incomplete_with_medians_kept() {
    let mut recs = records(1, 500, 900, ALL);
    recs[1].status = Status::TimedOut; // full run 1
    let g = grade_task(1, &recs, &ARMS, 3);
    assert_eq!(g.verdict, Verdict::Incomplete);
    assert_eq!(g.reasons, ["full task 1 run 1: timed_out"]);
    assert!(g.criteria.is_empty());
    assert_eq!(g.arms[1].main_tokens, Some(500.0));
    assert_eq!(g.arms[1].not_ok, 1);

    let mut no_transcript = records(1, 500, 900, ALL);
    no_transcript[0].result = None;
    no_transcript[0].error = Some("no transcript".into());
    let g = grade_task(1, &no_transcript, &ARMS, 3);
    assert_eq!(g.verdict, Verdict::Incomplete);
    assert_eq!(g.arms[0].main_tokens, None);

    let short = records(1, 500, 900, ALL);
    let g = grade_task(1, &short[..4], &ARMS, 3);
    assert_eq!(g.verdict, Verdict::Incomplete);
    assert!(
        g.reasons
            .iter()
            .any(|r| r == "bare task 1: 2 of 3 runs started")
    );

    let g = grade_task(6, &task6(5)[..task6(5).len() - 1], &ARMS, 3);
    assert_eq!(
        g.verdict,
        Verdict::Incomplete,
        "bare task 4 is compared too"
    );
}

#[test]
fn max_cost_estimates_from_the_mean_and_always_runs_the_first() {
    assert!(!would_exceed(0.0, &[]));
    assert!(!would_exceed(3.0, &[1.0, 1.0]));
    assert!(would_exceed(2.9, &[1.0, 1.0]));
}

#[test]
fn a_missing_pi_binary_is_a_clear_error() {
    let empty = TempDir::new().unwrap();
    let err = Pi::find(empty.path().as_os_str(), Duration::from_secs(1))
        .err()
        .unwrap();
    let err = err.downcast_ref::<CliError>().unwrap();
    assert_eq!(err.code, "harness_missing");
    assert!(err.fix.contains("pi-subagents"), "{}", err.fix);
}

#[cfg(unix)]
#[test]
fn pi_driver_records_a_timed_out_session_with_its_transcript() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempDir::new().unwrap();
    let bin = tmp.path().join("pi");
    // writes a session file into --session-dir, then hangs
    std::fs::write(
        &bin,
        "#!/bin/sh\nwhile [ \"$1\" != --session-dir ]; do shift; done\necho '{\"type\":\"session\",\"id\":\"s\",\"timestamp\":\"t\",\"cwd\":\"/r\"}' > \"$2/s.jsonl\"\nexec sleep 30\n",
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let pi = Pi {
        bin,
        limit: Duration::from_secs(1),
    };
    let sessions = tmp.path().join("sessions");
    let path = std::env::var_os("PATH").unwrap();
    let s = pi.start(tmp.path(), "hi", None, &sessions, &path).unwrap();
    assert_eq!(s.status, Status::TimedOut);
    assert!(s.duration_s < 20.0, "{}", s.duration_s);
    assert_eq!(s.transcript, Some(sessions.join("s.jsonl")));
}

/// Copies a recorded sample as each session's transcript, with a fixed status.
struct Replay {
    sample: PathBuf,
    status: Status,
}

impl Driver for Replay {
    fn start(
        &self,
        dir: &Path,
        prompt: &str,
        _: Option<&str>,
        sessions: &Path,
        _: &OsStr,
    ) -> anyhow::Result<Session> {
        assert!(dir.join("api").is_dir() && !prompt.is_empty());
        std::fs::create_dir_all(sessions)?;
        copy_dir(&self.sample, sessions);
        Ok(Session {
            status: self.status,
            transcript: Some(main_file(sessions)),
            duration_s: 0.1,
        })
    }

    fn parse(&self, transcript: &Path, duration_s: f64) -> anyhow::Result<Run> {
        parse(transcript, duration_s)
    }
}

fn plan(tmp: &Path, runs: u32, max_cost: Option<f64>) -> Plan {
    Plan {
        tasks: vec![1],
        arms: vec![Arm::Bare],
        runs,
        model: None,
        max_cost,
        setup_dir: None,
        evals: Path::new(env!("CARGO_MANIFEST_DIR")).join("evals"),
        results: tmp.join("results"),
        work: tmp.join("work"),
        stewards: PathBuf::from(env!("CARGO_BIN_EXE_stewards")),
        path: std::env::var_os("PATH").unwrap(),
    }
}

fn run_files(results: &Path) -> Vec<serde_json::Value> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(results.join("runs"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    files
        .iter()
        .map(|f| serde_json::from_str(&std::fs::read_to_string(f).unwrap()).unwrap())
        .collect()
}

#[test]
fn max_cost_stops_before_the_run_that_would_exceed_it() {
    let tmp = TempDir::new().unwrap();
    let driver = Replay {
        sample: sample("subagent"),
        status: Status::Ok,
    };
    // each replayed session costs about $0.00206: two fit under $0.005, a third would not
    let summary = execute(&driver, &plan(tmp.path(), 3, Some(0.005))).unwrap();
    assert!(summary.stopped_by_max_cost);
    assert_eq!(summary.sessions, 2);
    assert_eq!(run_files(&tmp.path().join("results")).len(), 2);
    assert_eq!(summary.verdict, Verdict::Incomplete);
    assert!(tmp.path().join("results/summary.json").is_file());
    assert!(tmp.path().join("results/summary.md").is_file());
}

#[test]
fn a_timed_out_run_is_recorded_not_dropped() {
    let tmp = TempDir::new().unwrap();
    let driver = Replay {
        sample: sample("subagent"),
        status: Status::TimedOut,
    };
    let summary = execute(&driver, &plan(tmp.path(), 1, None)).unwrap();
    let runs = run_files(&tmp.path().join("results"));
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["status"], "timed_out");
    assert_eq!(runs[0]["arm"], "bare");
    assert_eq!(runs[0]["task"], 1);
    assert_eq!(runs[0]["role"], "task");
    assert_eq!(runs[0]["result"]["main_tokens"]["output"], 743);
    assert!(runs[0]["transcript"].as_str().unwrap().ends_with(".jsonl"));
    assert_eq!(summary.grades[0].arms[0].not_ok, 1);
}

#[test]
fn no_developers_replaces_both_developer_lines_of_the_block() {
    let block = stewards::guide::block();
    let off = without_developers(block).unwrap();
    assert!(off.contains("- Changing: make the edits yourself.\n"));
    assert!(off.contains("A review that returns findings: fix them yourself. Done means no jobs."));
    assert!(!off.contains("--role developer"));
    assert_eq!(off.lines().count(), block.lines().count());
    assert!(without_developers("no block").is_err());
}

#[test]
fn arms_sh_writes_the_block_init_writes() {
    let arms = include_str!("../evals/arms.sh");
    let start = arms.find("<!-- stewards:start -->").unwrap();
    let end = arms.find("<!-- stewards:end -->").unwrap() + "<!-- stewards:end -->".len();
    assert_eq!(&arms[start..end], stewards::guide::block());
}

#[cfg(unix)]
#[test]
fn path_without_stewards_keeps_the_other_tools_of_that_directory() {
    let tmp = TempDir::new().unwrap();
    let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
    for (dir, file) in [(&a, "stewards"), (&a, "cargo"), (&b, "git")] {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(file), "").unwrap();
    }
    let path = std::env::join_paths([&a, &b]).unwrap();
    let off = path_without_stewards(&path, &tmp.path().join("mirrors")).unwrap();
    let dirs: Vec<PathBuf> = std::env::split_paths(&off).collect();
    assert_eq!(dirs.len(), 2);
    assert_ne!(dirs[0], a);
    assert!(dirs[0].join("cargo").exists() && !dirs[0].join("stewards").exists());
    assert_eq!(dirs[1], b);
}

#[test]
fn full_arm_sets_the_hand_drawn_units_and_stops_when_adoption_is_not_clean() {
    let tmp = TempDir::new().unwrap();
    let driver = Replay {
        sample: sample("subagent"),
        status: Status::Ok,
    };
    let mut p = plan(tmp.path(), 1, None);
    p.arms = vec![Arm::Bare, Arm::Full];
    // the replayed adoption session reviews nothing, so `stewards next` still lists jobs
    let err = execute(&driver, &p).err().unwrap();
    assert_eq!(
        err.downcast_ref::<CliError>().unwrap().code,
        "eval_setup_incomplete"
    );
    let adopted = tmp.path().join("work/adopted");
    assert_eq!(
        std::fs::read_to_string(adopted.join(".stewards/units.toml")).unwrap(),
        std::fs::read_to_string(p.evals.join("full-units.toml")).unwrap()
    );
    assert_eq!(
        stewards::eval::unowned_files(&adopted).unwrap(),
        [".gitignore"]
    );
    let summary: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(tmp.path().join("results/summary.json")).unwrap(),
    )
    .unwrap();
    let note = summary["units_note"].as_str().unwrap();
    assert!(
        note.contains("set by hand")
            && note.contains("proposed cli and left 5 tracked file(s) unowned"),
        "{note}"
    );
    assert_eq!(summary["adoption"]["label"], "001-adopt");
    // no full or bare task run started after the failed setup
    assert_eq!(run_files(&tmp.path().join("results")).len(), 1);
}

#[allow(dead_code)] // each test crate uses a subset of the helpers
mod common;

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use common::copy_dir;
use stewards::eval::grade::{grade_task, median, render, summarize};
use stewards::eval::pi::{Pi, agent_dir, parse};
use stewards::eval::{
    ADOPT_PROMPT, Action, Arm, Driver, Plan, RITUAL_STEPS, RitualSteps, Run, RunRecord, RunRole,
    Session, SetupNotes, Status, Tokens, Verdict, adopted_knowledge, execute, kill_leftovers,
    neutral_work_dir, next_check, path_without_stewards, ritual_steps, shell_reads,
    stewards_subcommands, without_developers, would_exceed,
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

/// The ritual steps with exactly `on` present.
fn steps(on: &[&str]) -> RitualSteps {
    RITUAL_STEPS.iter().map(|&s| (s, on.contains(&s))).collect()
}

fn all() -> RitualSteps {
    steps(&RITUAL_STEPS)
}

#[test]
fn pi_sample_counts_child_tokens_in_total_and_not_in_main() {
    let run = parse(&main_file(&sample("subagent")), 1.5, false).unwrap();
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
    assert_eq!(run.files_opened_tree, ["AGENTS.md"]);
    assert_eq!(run.shell_commands, ["echo probe"]);
    assert_eq!(run.model.as_deref(), Some("anthropic/claude-haiku-5-5"));
    assert_eq!(run.models, ["anthropic/claude-haiku-5-5"]);
    // the main session ran at medium, the pi-subagents worker at its own frontmatter's high
    assert_eq!(run.thinking_levels, ["high", "medium"]);
    assert!((run.cost_usd.unwrap() - (0.00113712 + 0.0009244)).abs() < 1e-6);
    assert_eq!(run.duration_s, 1.5);
    assert_eq!(run.ritual_steps, steps(&[]));
    assert_eq!(run.peak_main_context, 15564);
    assert_eq!(run.cache_warming, Tokens::default());
    assert_eq!(run.stop_reason.as_deref(), Some("stop"));
    assert!(run.failed_children.is_empty());
    assert_eq!(run.truncated_lines, 0);
}

#[test]
fn a_fork_copy_of_the_parent_is_counted_once() {
    let tmp = TempDir::new().unwrap();
    copy_dir(&sample("subagent"), tmp.path());
    let parent = main_file(tmp.path());
    let before = parse(&parent, 0.0, false).unwrap();
    let forks = parent.with_extension("").join("forks/f1");
    std::fs::create_dir_all(&forks).unwrap();
    let extra = r#"{"type":"message","id":"fork0001","parentId":null,"timestamp":"2026-11-12T19:30:00.000Z","message":{"role":"assistant","content":[],"usage":{"input":1,"output":2,"cacheRead":3,"cacheWrite":4,"cost":{"total":0.5}}}}"#;
    let copy = std::fs::read_to_string(&parent).unwrap() + extra + "\n";
    std::fs::write(forks.join("fork.jsonl"), copy).unwrap();
    let after = parse(&parent, 0.0, false).unwrap();
    assert_eq!(after.main_tokens, before.main_tokens);
    assert_eq!(after.total_tokens.sum(), before.total_tokens.sum() + 10);
    assert!((after.cost_usd.unwrap() - before.cost_usd.unwrap() - 0.5).abs() < 1e-9);
}

/// The trimmed real adoption session of the Haiku smoke run: the parent ran `stewards status`,
/// `stewards init --draft`, launched the briefs, and ran `stewards next`; the two children kept
/// (the architect and the api documenter) each ran `stewards submit`. Nothing ran `route`.
#[test]
fn pi_ritual_sample_shows_the_real_adoption_steps_across_parent_and_children() {
    let run = parse(&main_file(&sample("ritual")), 0.0, false).unwrap();
    // check_green is the runner's own `stewards next`, never read from a transcript
    assert_eq!(
        run.ritual_steps,
        steps(&["status", "brief", "next", "submit"])
    );
    let main = Tokens {
        input: 26,
        output: 3121,
        cache_read: 74781,
        cache_write: 18239,
    };
    assert_eq!(run.main_tokens, main);
    assert_eq!(run.total_tokens.sum(), 96167 + 100879 + 49497);
    assert_eq!(run.peak_main_context, 18243);
    assert!((run.cost_usd.unwrap() - 0.01398651).abs() < 1e-9);
    assert_eq!(run.cache_warming, Tokens::default());
    assert_eq!(run.thinking_levels, ["high", "medium"]);
    assert_eq!(run.models, ["anthropic/claude-haiku-5-5"]);
    assert_eq!(
        run.tool_calls,
        ["bash", "subagent", "bash", "subagent", "bash"]
    );
    assert!(run.files_opened.is_empty(), "{:?}", run.files_opened);
    // the architect's `for f in ...; do head -60 "$f"` adds no `$f`
    assert_eq!(
        run.files_opened_tree,
        [
            ".stewards/briefs/architect.md",
            "cli/src/main.rs",
            "client/client.ts",
            "api/items.py",
            ".stewards/repo.toml",
            ".stewards/briefs/api-documenter.md",
        ]
    );
    assert!(run.failed_children.is_empty());
}

/// simplified: the real sample has no `stewards route`, so this hand-made variant appends one bash
/// call to its parent; replace it with a real transcript once a smoke run routes.
#[test]
fn a_route_command_in_a_transcript_is_the_route_step() {
    let tmp = TempDir::new().unwrap();
    copy_dir(&sample("ritual"), tmp.path());
    let parent = main_file(tmp.path());
    let route = r#"{"type":"message","id":"route001","parentId":null,"timestamp":"2026-11-12T21:00:00.000Z","message":{"role":"assistant","content":[{"type":"toolCall","id":"t1","name":"bash","arguments":{"command":"cd /work/0/repo && stewards route api/items.py"}}],"usage":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"cost":{"total":0}},"stopReason":"stop"}}"#;
    let text = std::fs::read_to_string(&parent).unwrap() + route + "\n";
    std::fs::write(&parent, text).unwrap();
    let run = parse(&parent, 0.0, false).unwrap();
    assert_eq!(
        run.ritual_steps,
        steps(&["status", "route", "brief", "next", "submit"])
    );
}

#[test]
fn a_cache_refresh_cost_is_in_cost_usd_but_not_in_tokens() {
    let tmp = TempDir::new().unwrap();
    copy_dir(&sample("subagent"), tmp.path());
    let parent = main_file(tmp.path());
    let before = parse(&parent, 0.0, false).unwrap();
    let warm = r#"{"type":"usage","id":"w1","parentId":null,"timestamp":"2026-11-12T19:31:00.000Z","kind":"cache_warm","usage":{"input":1,"output":1,"cacheRead":9000,"cacheWrite":0,"cost":{"total":0.25}}}"#;
    let text = std::fs::read_to_string(&parent).unwrap() + warm + "\n";
    std::fs::write(&parent, text).unwrap();
    let after = parse(&parent, 0.0, false).unwrap();
    assert_eq!(after.main_tokens, before.main_tokens);
    assert_eq!(after.total_tokens, before.total_tokens);
    assert_eq!(after.peak_main_context, before.peak_main_context);
    assert_eq!(after.cache_warming.cache_read, 9000);
    assert!((after.cost_usd.unwrap() - before.cost_usd.unwrap() - 0.25).abs() < 1e-9);
}

fn shell(command: &str) -> Action {
    Action::Shell {
        command: command.into(),
    }
}

fn dispatch(text: &str) -> Action {
    Action::Dispatch { text: text.into() }
}

#[test]
fn each_ritual_step_is_detected_alone_and_absent_otherwise() {
    let cases: [(&str, Vec<Action>); 5] = [
        ("status", vec![shell("stewards status")]),
        ("route", vec![shell("cd /r && stewards route a.py")]),
        (
            "brief",
            vec![dispatch(
                r#"{"task":"Read /r/.stewards/briefs/api-maintainer.md"}"#,
            )],
        ),
        ("next", vec![shell("stewards next")]),
        (
            "submit",
            vec![shell("stewards submit api --role tester --file a.json")],
        ),
    ];
    for (name, actions) in &cases {
        let found = ritual_steps(actions);
        for other in RITUAL_STEPS {
            // submit implies brief; nothing else implies another step
            let expected = other == *name || (*name == "submit" && other == "brief");
            assert_eq!(found[other], expected, "{name}: {other}");
        }
    }
    assert_eq!(ritual_steps(&[]), steps(&[]));
}

#[test]
fn shell_reads_finds_the_files_simple_read_commands_name() {
    assert_eq!(shell_reads("cat api/items.py"), ["api/items.py"]);
    assert_eq!(
        shell_reads("head -n 20 a.py && tail -5 b.py | less c.py"),
        ["a.py", "b.py", "c.py"]
    );
    assert_eq!(
        shell_reads("sed -n '1,40p' cli/src/main.rs"),
        ["cli/src/main.rs"]
    );
    assert_eq!(shell_reads("sed 's/a/b/' x.py"), Vec::<&str>::new());
    assert_eq!(
        shell_reads("grep -n -A 3 create api/items.py client/client.ts 2>/dev/null"),
        ["api/items.py", "client/client.ts"]
    );
    assert_eq!(shell_reads("rg priority"), Vec::<&str>::new());
    assert_eq!(shell_reads("cat > new.py"), Vec::<&str>::new());
    assert_eq!(shell_reads("ls api; echo cat"), Vec::<&str>::new());
}

#[test]
fn shell_reads_skips_quoted_separators_variables_negations_and_option_values() {
    // the two smoke-run commands the second review found misread
    assert_eq!(
        shell_reads(
            "cd /work/1/repo && git ls-files | head -200 && echo --- && rg -n -i \"create\" --glob '!node_modules' -g '!*.lock' | head -100"
        ),
        Vec::<&str>::new()
    );
    assert_eq!(
        shell_reads(
            r#"cd /work/3/repo && export ITEMS_STORE=$(mktemp -u /tmp/itemsXXXX.json); node --version; python3 api/items.py create "widget"; echo "---"; cat "$ITEMS_STORE"; echo; echo "---"; node client/client.ts create-item "gadget"; echo "exit=$?"; echo "---"; cat "$ITEMS_STORE"; echo; rm -f "$ITEMS_STORE"; echo "--- tests"; grep -n "label\|name" client/client.test.ts api/test_items.py | head -30"#
        ),
        ["client/client.test.ts", "api/test_items.py"]
    );
    assert_eq!(
        shell_reads(
            r#"for f in api/items.py cli/src/main.rs; do echo "=== $f"; head -60 "$f"; done"#
        ),
        Vec::<&str>::new()
    );
    // an option value is never the path; `-e` gives the pattern, so the first word is a path
    assert_eq!(
        shell_reads("grep -A 3 -e create -m 5 api/items.py"),
        ["api/items.py"]
    );
    assert_eq!(shell_reads("rg -t py -C 2 create api"), ["api"]);
    assert_eq!(
        shell_reads("grep 'a b; c | d' x.py"),
        ["x.py"],
        "a quoted pattern with spaces and separators is one word"
    );
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

fn run_with(main: u64, total: u64, files: usize, ritual: RitualSteps) -> Run {
    let t = |n| Tokens {
        input: n,
        ..Tokens::default()
    };
    let files: Vec<String> = (0..files).map(|n| format!("f{n}")).collect();
    Run {
        main_tokens: t(main),
        total_tokens: t(total),
        cache_warming: Tokens::default(),
        peak_main_context: main / 2,
        tool_calls: vec![],
        files_opened: files.clone(),
        files_opened_tree: files,
        shell_commands: vec![],
        ritual_steps: ritual,
        cost_usd: Some(main as f64 / 1000.0),
        duration_s: 1.0,
        model: None,
        models: vec![],
        thinking_levels: vec![],
        stop_reason: Some("stop".into()),
        failed_children: vec![],
        truncated_lines: 0,
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
        thinking: "medium".into(),
        model_mismatch: false,
        thinking_mismatch: false,
        status: Status::Ok,
        failure: None,
        transcript: None,
        workdir: PathBuf::new(),
        leftovers: None,
        next_check: None,
        changed_tracked_files: true,
        result: Some(result),
        error: None,
    }
}

/// Three runs per arm of `task`: bare at 1000 main and 1000 total, full at the medians given.
fn records(task: u32, full_main: u64, full_total: u64, ritual: RitualSteps) -> Vec<RunRecord> {
    let mut out = vec![];
    for run in 1..=3 {
        // the spread around the median must not matter
        let spread = [0, 1, 0][run as usize - 1] * 5000;
        out.push(record(
            task,
            Arm::Bare,
            RunRole::Task,
            run,
            run_with(1000 + spread, 1000 + spread, 10, steps(&[])),
        ));
        out.push(record(
            task,
            Arm::Full,
            RunRole::Task,
            run,
            run_with(full_main + spread, full_total + spread, 4, ritual.clone()),
        ));
    }
    out
}

const ARMS: [Arm; 2] = [Arm::Bare, Arm::Full];

#[test]
fn token_criteria_boundaries() {
    let grade = |main, total| grade_task(1, &records(1, main, total, all()), &ARMS, 3);
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
fn cost_and_peak_context_ratios_are_report_only() {
    // full costs and peaks 3x bare, yet the token criteria alone decide the verdict
    let g = grade_task(1, &records(1, 3000, 1000, all()), &ARMS, 3);
    let ratio = |n: &str| g.report_only.iter().find(|r| r.name == n).unwrap().ratio;
    assert!((ratio("cost_usd").unwrap() - 3.0).abs() < 1e-9);
    assert!((ratio("peak_main_context").unwrap() - 3.0).abs() < 1e-9);
    assert_eq!(g.verdict, Verdict::Warn, "{:?}", g.reasons);
    let pass = grade_task(1, &records(1, 500, 900, all()), &ARMS, 3);
    assert_eq!(pass.verdict, Verdict::Pass);
    assert!((pass.report_only[0].ratio.unwrap() - 0.5).abs() < 1e-9);
}

#[test]
fn a_missing_ritual_step_is_a_warning() {
    let mut ritual = all();
    ritual.insert("submit", false);
    let g = grade_task(1, &records(1, 500, 900, ritual), &ARMS, 3);
    assert_eq!(g.verdict, Verdict::Warn);
    assert_eq!(g.reasons, ["ritual steps missing: submit"]);
}

#[test]
fn ritual_steps_that_cannot_apply_are_n_a_and_the_rest_counted() {
    let ritual = |g: &stewards::eval::TaskGrade| {
        let c = g.criteria.iter().find(|c| c.name == "ritual").unwrap();
        (c.value, c.limit)
    };
    // task 4 is a read-only question: only status and next apply
    let g = grade_task(
        4,
        &records(4, 500, 900, steps(&["status", "next"])),
        &ARMS,
        3,
    );
    assert_eq!(g.verdict, Verdict::Pass, "{:?}", g.reasons);
    assert_eq!(ritual(&g), (2.0, 2.0));
    let full = g.arms[1].ritual.as_ref().unwrap();
    for step in ["route", "brief", "submit", "check_green"] {
        assert_eq!(full[step], None, "{step}");
    }
    assert_eq!(full["status"], Some(true));

    // check_green is n/a when no run changed tracked files, counted when one did
    let mut recs = records(
        1,
        500,
        900,
        steps(&["status", "route", "brief", "next", "submit"]),
    );
    for r in &mut recs {
        r.changed_tracked_files = false;
    }
    let g = grade_task(1, &recs, &ARMS, 3);
    assert_eq!(g.verdict, Verdict::Pass, "{:?}", g.reasons);
    assert_eq!(ritual(&g), (5.0, 5.0));
    recs.iter_mut()
        .filter(|r| r.arm == Arm::Full && r.run == 1)
        .for_each(|r| r.changed_tracked_files = true);
    let g = grade_task(1, &recs, &ARMS, 3);
    assert_eq!(g.verdict, Verdict::Warn);
    assert_eq!(ritual(&g), (5.0, 6.0));
    assert_eq!(g.reasons, ["ritual steps missing: check_green"]);
    // the per-run record still shows what ran
    assert!(recs[1].result.as_ref().unwrap().ritual_steps["route"]);
}

#[test]
fn summary_md_shows_warnings_in_the_headline_costs_to_4_decimals_and_adoption() {
    let tmp = TempDir::new().unwrap();
    let mut p = plan(tmp.path(), 3, None);
    p.arms = ARMS.to_vec();
    let mut recs = records(1, 801, 1000, all()); // main_tokens over its limit: a warning
    recs[1].thinking_mismatch = true;
    recs[1].result.as_mut().unwrap().thinking_levels = vec!["high".into(), "medium".into()];
    let mut adoption = record(
        1,
        Arm::Full,
        RunRole::Setup,
        1,
        run_with(100, 16000, 0, all()),
    );
    adoption.task = None;
    adoption.label = "001-adopt".into();
    recs.insert(0, adoption);
    let setup = SetupNotes {
        gaps: vec!["cli: gap: panics on an empty name".into()],
        flows: vec!["create item: cli, client, api".into()],
        ..SetupNotes::default()
    };
    let summary = summarize(&p, &recs, false, &[], setup);
    assert_eq!(summary.verdict, Verdict::Pass, "the exit code stays a pass");
    assert_eq!(summary.adoption_in_bare_tasks, Some(16.0));
    let md = render(&summary);
    assert!(md.starts_with("# eval: pass (with warnings: 1)\n"), "{md}");
    assert!(md.contains("| 0.8010 |"), "{md}");
    assert!(
        md.contains("adoption total tokens = 16.0 median bare task totals (report-only)"),
        "{md}"
    );
    assert!(md.contains("cli: gap: panics on an empty name"), "{md}");
    assert!(
        md.contains("flows adoption recorded: create item: cli, client, api"),
        "{md}"
    );
    assert!(
        md.contains("other thinking levels used: full-t1-r1: high, medium"),
        "{md}"
    );
    assert!(md.contains("| 6/6 |"), "{md}");
}

#[test]
fn adopted_knowledge_lists_gaps_of_every_kind_and_the_flows() {
    let repo = common::fixture("single");
    common::append(
        &repo.path().join(".stewards/units/api.toml"),
        "\n[[gap]]\nkind = \"bug\"\ntitle = \"pagination skips the last item\"\n",
    );
    let (gaps, flows) = adopted_knowledge(repo.path()).unwrap();
    assert!(
        gaps.contains(&"api: gap: no pagination on list".to_string()),
        "{gaps:?}"
    );
    assert!(
        gaps.contains(&"api: bug: pagination skips the last item".to_string()),
        "{gaps:?}"
    );
    assert_eq!(flows, ["create-item: api, store"]);
}

/// Task 6 records with the full arm's task-6 files opened at `files`, against bare task 4 at `bare4`.
fn task6_against(files: usize, bare4: usize) -> Vec<RunRecord> {
    let mut out = records(6, 500, 900, all());
    for r in out.iter_mut().filter(|r| r.arm == Arm::Full) {
        r.result.as_mut().unwrap().files_opened = (0..files).map(|n| format!("f{n}")).collect();
    }
    for run in 1..=3 {
        for arm in ARMS {
            out.push(record(
                5,
                arm,
                RunRole::Setup,
                run,
                run_with(1, 1, 0, all()),
            ));
        }
        out.push(record(
            4,
            Arm::Bare,
            RunRole::Task,
            run,
            run_with(1000, 1000, bare4, steps(&[])),
        ));
    }
    out
}

fn task6(files: usize) -> Vec<RunRecord> {
    task6_against(files, 10)
}

#[test]
fn task_6_files_opened_is_at_most_half_of_bare_task_4() {
    let half = grade_task(6, &task6(5), &ARMS, 3);
    assert_eq!(half.verdict, Verdict::Pass, "{:?}", half.reasons);
    let over = grade_task(6, &task6(6), &ARMS, 3);
    assert_eq!(over.verdict, Verdict::Warn);
    assert!(over.reasons[0].starts_with("files_opened 0.60"));
    let none = grade_task(6, &task6_against(0, 0), &ARMS, 3);
    assert_eq!(none.verdict, Verdict::Warn, "0 on both sides is not a pass");
    assert_eq!(
        none.reasons,
        ["files_opened unmeasured: 0 files opened on both sides"]
    );
    assert!(
        none.criteria
            .iter()
            .any(|c| c.name == "files_opened" && c.unmeasured)
    );
}

#[test]
fn a_timed_out_or_missing_run_makes_the_task_incomplete_with_medians_kept() {
    let mut recs = records(1, 500, 900, all());
    recs[1].status = Status::TimedOut; // full run 1
    let g = grade_task(1, &recs, &ARMS, 3);
    assert_eq!(g.verdict, Verdict::Incomplete);
    assert_eq!(g.reasons, ["full task 1 run 1: timed_out"]);
    assert!(g.criteria.is_empty());
    assert_eq!(g.arms[1].main_tokens, Some(500.0));
    assert_eq!(g.arms[1].not_ok, 1);

    let mut no_transcript = records(1, 500, 900, all());
    no_transcript[0].result = None;
    no_transcript[0].error = Some("no transcript".into());
    let g = grade_task(1, &no_transcript, &ARMS, 3);
    assert_eq!(g.verdict, Verdict::Incomplete);
    assert_eq!(g.arms[0].main_tokens, None);

    let short = records(1, 500, 900, all());
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

#[test]
fn the_eval_agent_dir_holds_only_auth_the_package_store_and_pi_subagents() {
    let tmp = TempDir::new().unwrap();
    let user = tmp.path().join("user");
    std::fs::create_dir_all(user.join("npm/node_modules/pi-subagents/agents")).unwrap();
    std::fs::write(
        user.join("npm/node_modules/pi-subagents/agents/worker.md"),
        "---\nname: worker\nthinking: high\n---\n",
    )
    .unwrap();
    std::fs::create_dir_all(user.join("extensions/x")).unwrap();
    std::fs::write(user.join("AGENTS.md"), "personal rules").unwrap();
    std::fs::write(user.join("auth.json"), "{}").unwrap();
    std::fs::write(user.join("models-store.json"), "{}").unwrap();
    std::fs::write(
        user.join("settings.json"),
        r#"{"defaultModel":"m","packages":["npm:pi-subagents","npm:other"],"cacheWarming":"off"}"#,
    )
    .unwrap();
    let dir = tmp.path().join("a");
    let note = agent_dir(&user, Path::new("/home/u"), &dir, "low").unwrap();
    let settings: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("settings.json")).unwrap()).unwrap();
    assert_eq!(
        settings,
        serde_json::json!({
            "packages": ["npm:pi-subagents"],
            "skills": ["!/home/u/.agents/skills/**"],
            "defaultModel": "m",
            // every child at the eval's level, the bundled worker's own `high` overridden
            "subagents": {
                "defaultThinking": "low",
                "maxThinking": "low",
                "agentOverrides": { "worker": { "thinking": "low" } },
            },
        })
    );
    assert_eq!(
        std::fs::read_link(dir.join("auth.json")).unwrap(),
        user.join("auth.json")
    );
    assert_eq!(
        std::fs::read_link(dir.join("npm")).unwrap(),
        user.join("npm")
    );
    assert!(
        std::fs::symlink_metadata(dir.join("models-store.json"))
            .unwrap()
            .is_file()
    );
    for absent in ["AGENTS.md", "extensions"] {
        assert!(!dir.join(absent).exists(), "{absent}");
    }
    assert!(note.contains("npm:pi-subagents"), "{note}");

    std::fs::remove_dir_all(user.join("npm")).unwrap();
    let err = agent_dir(&user, Path::new("/home/u"), &tmp.path().join("b"), "low")
        .err()
        .unwrap();
    assert_eq!(
        err.downcast_ref::<CliError>().unwrap().code,
        "harness_missing"
    );
}

/// Number of live (non-zombie) processes whose cwd is under `dir`.
fn alive_under(dir: &Path) -> usize {
    let dir = dir.canonicalize().unwrap();
    std::fs::read_dir("/proc")
        .unwrap()
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.parse::<u32>().is_ok())
        })
        .filter(|e| std::fs::read_link(e.path().join("cwd")).is_ok_and(|c| c.starts_with(&dir)))
        .count()
}

#[test]
fn pi_driver_passes_thinking_and_the_agent_dir_and_a_timed_out_sessions_detached_child_is_killed() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempDir::new().unwrap();
    let bin = tmp.path().join("pi");
    // records its argv and env, writes a session file, starts a detached child, then hangs
    std::fs::write(
        &bin,
        "#!/bin/sh\necho \"$@\" > ../args\nenv > ../env\nwhile [ \"$1\" != --session-dir ]; do shift; done\necho '{\"type\":\"session\",\"id\":\"s\",\"timestamp\":\"t\",\"cwd\":\"/r\"}' > \"$2/s.jsonl\"\nsetsid sleep 300 >/dev/null 2>&1 &\nexec sleep 30\n",
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let pi = Pi {
        bin,
        limit: Duration::from_secs(1),
        agent_dir: Some(tmp.path().join("a")),
    };
    let root = tmp.path().join("run");
    let dir = root.join("repo");
    std::fs::create_dir_all(&dir).unwrap();
    let sessions = tmp.path().join("sessions");
    let env = [
        ("PATH", std::env::var_os("PATH").unwrap()),
        ("STEWARDS_HOME", tmp.path().join("h").into_os_string()),
    ];
    let s = pi.start(&dir, "hi", None, "low", &sessions, &env).unwrap();
    assert_eq!(s.status, Status::TimedOut);
    assert!(s.duration_s < 20.0, "{}", s.duration_s);
    assert_eq!(s.transcript, Some(sessions.join("s.jsonl")));
    let args = std::fs::read_to_string(root.join("args")).unwrap();
    assert!(args.contains("--thinking low"), "{args}");
    let vars = std::fs::read_to_string(root.join("env")).unwrap();
    let agent = format!("PI_CODING_AGENT_DIR={}\n", tmp.path().join("a").display());
    assert!(vars.contains(&agent), "{vars}");
    assert!(vars.contains("STEWARDS_HOME="), "{vars}");

    // the process-group kill does not reach the detached child; the runner's sweep does
    assert_eq!(alive_under(&root), 1);
    let left = kill_leftovers(&root).unwrap().unwrap();
    assert_eq!(left.killed, 1);
    assert!(left.survived.is_empty());
    assert_eq!(alive_under(&root), 0);
}

#[test]
fn a_truncated_last_line_is_tolerated_only_for_a_failed_session() {
    let tmp = TempDir::new().unwrap();
    copy_dir(&sample("subagent"), tmp.path());
    let parent = main_file(tmp.path());
    let text = std::fs::read_to_string(&parent).unwrap();
    std::fs::write(&parent, text.clone() + "{\"type\":\"mess").unwrap();
    assert!(parse(&parent, 0.0, false).is_err());
    let run = parse(&parent, 0.0, true).unwrap();
    assert_eq!(run.truncated_lines, 1);
    assert_eq!(run.main_tokens.output, 743);
    // a bad line anywhere else fails even a failed session's parse
    std::fs::write(&parent, "{\"type\":\"mess\n".to_string() + &text).unwrap();
    assert!(parse(&parent, 0.0, true).is_err());
}

#[test]
fn two_top_level_session_files_are_an_error() {
    let tmp = TempDir::new().unwrap();
    copy_dir(&sample("subagent"), tmp.path());
    let parent = main_file(tmp.path());
    std::fs::copy(&parent, tmp.path().join("zz-second.jsonl")).unwrap();
    let err = parse(&parent, 0.0, false).err().unwrap();
    assert!(
        format!("{err:#}").contains("2 top-level session files"),
        "{err:#}"
    );
}

/// The subagent sample with the last `stopReason` of the main file, or of the child, replaced.
fn stopped_sample(tmp: &Path, child: bool, reason: &str) -> PathBuf {
    let dir = tmp.join(format!("sample-{child}-{reason}"));
    std::fs::create_dir_all(&dir).unwrap();
    copy_dir(&sample("subagent"), &dir);
    let mut file = main_file(&dir);
    if child {
        file = file
            .with_extension("")
            .join("b5d51927-cb55-4247-a969-9448b23c2c26/run-0/session.jsonl");
    }
    let text = std::fs::read_to_string(&file).unwrap();
    let at = text.rfind("\"stopReason\":\"stop\"").unwrap();
    let text = format!(
        "{}\"stopReason\":\"{reason}\"{}",
        &text[..at],
        &text[at + "\"stopReason\":\"stop\"".len()..]
    );
    std::fs::write(&file, text).unwrap();
    dir
}

#[test]
fn a_main_or_child_session_that_stopped_with_an_error_is_a_failed_run() {
    let tmp = TempDir::new().unwrap();
    let main_error = parse(
        &main_file(&stopped_sample(tmp.path(), false, "error")),
        0.0,
        false,
    )
    .unwrap();
    assert_eq!(main_error.stop_reason.as_deref(), Some("error"));
    let child_length = parse(
        &main_file(&stopped_sample(tmp.path(), true, "length")),
        0.0,
        false,
    )
    .unwrap();
    assert_eq!(
        child_length.failed_children,
        ["b5d51927-cb55-4247-a969-9448b23c2c26/run-0/session.jsonl stopped with `length`"]
    );

    for (child, reason, why) in [
        (false, "aborted", "main session stopped with `aborted`"),
        (true, "error", "child b5d51927"),
    ] {
        let run = TempDir::new().unwrap();
        let driver = Replay::new(stopped_sample(tmp.path(), child, reason), Status::Ok);
        let summary = execute(&driver, &plan(run.path(), 1, None)).unwrap();
        let runs = run_files(&run.path().join("results"));
        assert_eq!(
            runs[0]["status"], "failed",
            "Pi exited 0, the transcript says otherwise"
        );
        assert!(
            runs[0]["failure"].as_str().unwrap().contains(why),
            "{}",
            runs[0]["failure"]
        );
        assert_eq!(summary.grades[0].arms[0].not_ok, 1);
    }
}

/// Copies a recorded sample as each session's transcript, with a fixed status; leaves a detached
/// process running in the run directory, and writes `adopt_writes` on the adoption prompt and
/// `task_writes` on any other.
struct Replay {
    sample: PathBuf,
    status: Status,
    adopt_writes: Vec<(&'static str, &'static str)>,
    task_writes: Vec<(&'static str, &'static str)>,
}

impl Replay {
    fn new(sample: PathBuf, status: Status) -> Replay {
        Replay {
            sample,
            status,
            adopt_writes: vec![],
            task_writes: vec![],
        }
    }
}

impl Driver for Replay {
    fn start(
        &self,
        dir: &Path,
        prompt: &str,
        _: Option<&str>,
        thinking: &str,
        sessions: &Path,
        env: &[(&str, std::ffi::OsString)],
    ) -> anyhow::Result<Session> {
        assert!(dir.join("api").is_dir() && !prompt.is_empty());
        assert_eq!(thinking, "medium");
        // nothing a session can see names the arm, stewards, or the eval
        let seen: Vec<String> = [dir, sessions]
            .iter()
            .map(|p| p.display().to_string())
            .chain(
                env.iter()
                    .skip(1)
                    .map(|(_, v)| v.to_string_lossy().into_owned()),
            )
            .collect();
        for p in &seen {
            for word in ["bare", "block", "full", "stewards", "eval", "results"] {
                assert!(!p.contains(word), "{p} names {word}");
            }
        }
        // STEWARDS_HOME is set exactly when stewards is on PATH: the full arms
        let on_path = std::env::split_paths(&env[0].1).any(|d| d.join("stewards").exists());
        assert_eq!(env.iter().any(|(k, _)| *k == "STEWARDS_HOME"), on_path);
        let writes = if prompt == ADOPT_PROMPT {
            &self.adopt_writes
        } else {
            &self.task_writes
        };
        for (path, text) in writes {
            std::fs::write(dir.join(path), text)?;
        }
        std::process::Command::new("sh")
            .args(["-c", "setsid sleep 300 >/dev/null 2>&1 &"])
            .current_dir(dir)
            .status()?;
        std::fs::create_dir_all(sessions)?;
        copy_dir(&self.sample, sessions);
        Ok(Session {
            status: self.status,
            transcript: Some(main_file(sessions)),
            duration_s: 0.1,
        })
    }

    fn parse(
        &self,
        transcript: &Path,
        duration_s: f64,
        truncation_ok: bool,
    ) -> anyhow::Result<Run> {
        parse(transcript, duration_s, truncation_ok)
    }
}

fn plan(tmp: &Path, runs: u32, max_cost: Option<f64>) -> Plan {
    Plan {
        tasks: vec![1],
        arms: vec![Arm::Bare],
        runs,
        model: None,
        thinking: "medium".into(),
        max_cost,
        setup_dir: None,
        evals: Path::new(env!("CARGO_MANIFEST_DIR")).join("evals"),
        results: tmp.join("results"),
        work: neutral_work_dir(tmp),
        keep_work: false,
        stewards: PathBuf::from(env!("CARGO_BIN_EXE_stewards")),
        path: std::env::var_os("PATH").unwrap(),
        harness_setup: None,
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

fn summary_json(tmp: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(tmp.join("results/summary.json")).unwrap())
        .unwrap()
}

#[test]
fn max_cost_stops_before_the_run_that_would_exceed_it() {
    let tmp = TempDir::new().unwrap();
    let driver = Replay::new(sample("subagent"), Status::Ok);
    // each replayed session costs about $0.00206: two fit under $0.005, a third would not
    let p = plan(tmp.path(), 3, Some(0.005));
    let summary = execute(&driver, &p).unwrap();
    assert!(summary.stopped_by_max_cost);
    assert_eq!(summary.sessions, 2);
    assert_eq!(run_files(&tmp.path().join("results")).len(), 2);
    assert_eq!(summary.verdict, Verdict::Incomplete);
    assert!(tmp.path().join("results/summary.json").is_file());
    assert!(tmp.path().join("results/summary.md").is_file());
    // transcripts are kept in the results; the work directory is gone
    assert!(main_file(&tmp.path().join("results/sessions/001-bare-t1-r1")).is_file());
    assert!(!p.work.exists());
}

#[test]
fn a_session_whose_cost_is_unknown_stops_max_cost() {
    let tmp = TempDir::new().unwrap();
    let broken = tmp.path().join("broken");
    std::fs::create_dir_all(&broken).unwrap();
    copy_dir(&sample("subagent"), &broken);
    let parent = main_file(&broken);
    let text = std::fs::read_to_string(&parent).unwrap();
    std::fs::write(&parent, "not json\n".to_string() + &text).unwrap();
    let driver = Replay::new(broken, Status::Ok);
    let summary = execute(&driver, &plan(tmp.path(), 3, Some(100.0))).unwrap();
    assert_eq!(summary.sessions, 1);
    assert!(summary.stopped_by_max_cost);
    assert_eq!(summary.unknown_cost_sessions, ["001-bare-t1-r1"]);
    let runs = run_files(&tmp.path().join("results"));
    assert!(
        runs[0]["error"].as_str().unwrap().contains(":1"),
        "{}",
        runs[0]["error"]
    );
    // without --max-cost the eval goes on
    let tmp = TempDir::new().unwrap();
    let summary = execute(&driver, &plan(tmp.path(), 2, None)).unwrap();
    assert_eq!(summary.sessions, 2);
}

#[test]
fn a_timed_out_run_is_recorded_not_dropped_and_its_leftover_process_killed() {
    let tmp = TempDir::new().unwrap();
    let driver = Replay::new(sample("subagent"), Status::TimedOut);
    let summary = execute(&driver, &plan(tmp.path(), 1, None)).unwrap();
    let runs = run_files(&tmp.path().join("results"));
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["status"], "timed_out");
    assert_eq!(runs[0]["arm"], "bare");
    assert_eq!(runs[0]["task"], 1);
    assert_eq!(runs[0]["role"], "task");
    assert_eq!(runs[0]["thinking"], "medium");
    assert_eq!(runs[0]["result"]["main_tokens"]["output"], 743);
    assert_eq!(runs[0]["leftovers"]["killed"], 1);
    assert_eq!(runs[0]["next_check"], serde_json::Value::Null);
    assert_eq!(runs[0]["changed_tracked_files"], false);
    // the replayed worker child ran at high, not the requested medium
    assert_eq!(runs[0]["thinking_mismatch"], true);
    assert_eq!(
        summary.thinking_mismatches,
        ["001-bare-t1-r1: high, medium"]
    );
    let transcript = PathBuf::from(runs[0]["transcript"].as_str().unwrap());
    assert!(transcript.starts_with(tmp.path().join("results/sessions")));
    assert!(transcript.is_file());
    assert_eq!(summary.grades[0].arms[0].not_ok, 1);
}

#[test]
fn only_a_change_to_a_tracked_file_counts_as_changed() {
    for (writes, changed) in [
        (vec![("api/new_file.py", "x\n")], false),
        (vec![("api/items.py", "changed\n")], true),
    ] {
        let tmp = TempDir::new().unwrap();
        let mut driver = Replay::new(sample("subagent"), Status::Ok);
        driver.task_writes = writes;
        execute(&driver, &plan(tmp.path(), 1, None)).unwrap();
        let runs = run_files(&tmp.path().join("results"));
        assert_eq!(runs[0]["changed_tracked_files"], changed);
    }
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

fn full_plan(tmp: &Path) -> Plan {
    let mut p = plan(tmp, 1, None);
    p.arms = vec![Arm::Bare, Arm::Full];
    p
}

fn setup_error(err: anyhow::Error) -> CliError {
    let e = err.downcast_ref::<CliError>().unwrap().clone();
    assert_eq!(e.code, "eval_setup_incomplete", "{e}");
    e
}

#[test]
fn full_arm_sets_the_hand_drawn_units_and_stops_when_adoption_is_not_clean() {
    let tmp = TempDir::new().unwrap();
    let mut driver = Replay::new(sample("subagent"), Status::Ok);
    // a tester may add a test file
    driver.adopt_writes = vec![("api/test_new.py", "pass\n")];
    let p = full_plan(tmp.path());
    // the replayed adoption session reviews nothing, so `stewards next` still lists jobs
    let err = setup_error(execute(&driver, &p).err().unwrap());
    assert!(
        err.message.contains("`stewards next` is not clean"),
        "{}",
        err.message
    );
    let summary = summary_json(tmp.path());
    let adopted = PathBuf::from(summary["adoption"]["workdir"].as_str().unwrap());
    assert_eq!(
        std::fs::read_to_string(adopted.join(".stewards/units.toml")).unwrap(),
        std::fs::read_to_string(p.evals.join("full-units.toml")).unwrap()
    );
    let note = summary["setup"]["units"].as_str().unwrap();
    assert!(
        note.contains("set by hand")
            && note.contains("proposed cli and left 5 tracked file(s) unowned"),
        "{note}"
    );
    assert_eq!(
        summary["setup"]["added_paths"],
        serde_json::json!(["AGENTS.md", "api/test_new.py"])
    );
    assert_eq!(summary["adoption"]["label"], "001-adopt");
    assert_eq!(summary["adoption"]["leftovers"]["killed"], 1);
    // the runner's own check: exit 1 with jobs listed
    let check = next_check(&p.stewards, &adopted).unwrap();
    assert_eq!(check.exit_code, Some(1));
    assert!(check.jobs.unwrap() > 0);
    // no full or bare task run started after the failed setup
    assert_eq!(run_files(&tmp.path().join("results")).len(), 1);
}

#[test]
fn adoption_that_changes_the_fixtures_code_stops_the_eval() {
    let tmp = TempDir::new().unwrap();
    let mut driver = Replay::new(sample("subagent"), Status::Ok);
    driver.adopt_writes = vec![("api/items.py", "fixed\n"), ("api/new.py", "x\n")];
    let err = setup_error(execute(&driver, &full_plan(tmp.path())).err().unwrap());
    assert!(err.message.contains("M api/items.py"), "{}", err.message);
    assert!(err.message.contains("A api/new.py"), "{}", err.message);
    assert!(err.fix.contains("api/test_*.py"), "{}", err.fix);
}

#[test]
fn a_setup_dir_must_be_a_clean_git_repository() {
    let tmp = TempDir::new().unwrap();
    let driver = Replay::new(sample("subagent"), Status::Ok);
    let mut p = full_plan(tmp.path());
    let setup = tmp.path().join("setup");
    std::fs::create_dir_all(&setup).unwrap();
    p.setup_dir = Some(setup.clone());
    let err = setup_error(execute(&driver, &p).err().unwrap());
    assert!(
        err.message.contains("not a git repository"),
        "{}",
        err.message
    );
    std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&setup)
        .status()
        .unwrap();
    std::fs::write(setup.join("loose"), "").unwrap();
    let err = setup_error(execute(&driver, &p).err().unwrap());
    assert!(
        err.message.contains("uncommitted changes"),
        "{}",
        err.message
    );
}

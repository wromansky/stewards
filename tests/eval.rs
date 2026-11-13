#[allow(dead_code)] // each test crate uses a subset of the helpers
mod common;

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use common::copy_dir;
use stewards::eval::grade::{grade_task, median, overhead_medians, render, summarize};
use stewards::eval::pi::{Pi, agent_dir, parse};
use stewards::eval::{
    ADOPT_PROMPT, Action, Arm, Driver, Plan, RITUAL_STEPS, ReviewedUnit, RitualSteps, Run,
    RunRecord, RunRole, Session, SetupNotes, Source, Status, Suite, Tokens, UnitReview, Verdict,
    adopted_knowledge, execute, existing_files, kill_leftovers, neutral_work_dir, next_check,
    path_without_stewards, reads_git_history, reviewed_units, ritual_steps, shell_reads,
    stewards_commands, task_prompt, without_developers, would_exceed,
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
    assert!(run.sessions_without_thinking.is_empty());
    assert_eq!(run.subagent_sessions, 1);
    assert!(run.reviews.is_empty() && run.git_history_commands.is_empty());
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
fn a_session_with_no_thinking_level_is_listed_and_assistant_levels_are_read() {
    let tmp = TempDir::new().unwrap();
    copy_dir(&sample("subagent"), tmp.path());
    let parent = main_file(tmp.path());
    let child = parent
        .with_extension("")
        .join("b5d51927-cb55-4247-a969-9448b23c2c26/run-0/session.jsonl");
    let strip = |file: &Path, keep_changes: bool| {
        let text: Vec<String> = std::fs::read_to_string(file)
            .unwrap()
            .lines()
            .filter_map(|l| {
                let mut r: serde_json::Value = serde_json::from_str(l).unwrap();
                if r["type"] == "thinking_level_change" && !keep_changes {
                    return None;
                }
                if let Some(m) = r["message"].as_object_mut() {
                    m.remove("thinkingLevel");
                }
                Some(r.to_string())
            })
            .collect();
        std::fs::write(file, text.join("\n") + "\n").unwrap();
    };
    // the child keeps only its assistant messages' `thinkingLevel`; the parent only its record
    let text = std::fs::read_to_string(&child).unwrap();
    let text: Vec<&str> = text
        .lines()
        .filter(|l| !l.contains("\"thinking_level_change\""))
        .collect();
    std::fs::write(&child, text.join("\n") + "\n").unwrap();
    strip(&parent, true);
    let run = parse(&parent, 0.0, false).unwrap();
    assert_eq!(run.thinking_levels, ["high", "medium"]);
    assert!(run.sessions_without_thinking.is_empty());
    strip(&child, false);
    strip(&parent, false);
    let run = parse(&parent, 0.0, false).unwrap();
    assert!(run.thinking_levels.is_empty());
    assert_eq!(
        run.sessions_without_thinking,
        [
            "main",
            "b5d51927-cb55-4247-a969-9448b23c2c26/run-0/session.jsonl"
        ]
    );
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
/// (the architect and the api documenter) each ran `stewards submit`, neither as the reviewer.
/// Nothing ran `route`.
#[test]
fn pi_ritual_sample_shows_the_real_adoption_steps_across_parent_and_children() {
    let run = parse(&main_file(&sample("ritual")), 0.0, false).unwrap();
    // check_green is the runner's own `stewards next`, never read from a transcript
    assert_eq!(run.ritual_steps, steps(&["status", "next"]));
    assert_eq!(run.subagent_sessions, 2);
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
    assert_eq!(run.ritual_steps, steps(&["status", "route", "next"]));
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
        child: false,
    }
}

fn child_shell(command: &str) -> Action {
    Action::Shell {
        command: command.into(),
        child: true,
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
        ("next", vec![child_shell("stewards next")]),
        (
            "review_dispatched",
            vec![dispatch(r#"{"task":"Read /r/.stewards/briefs/review.md"}"#)],
        ),
        (
            "reviewer_submit",
            vec![child_shell(
                "cd /r && stewards submit --role reviewer --file .stewards/briefs/review.answer.json",
            )],
        ),
    ];
    for (name, actions) in &cases {
        let found = ritual_steps(actions);
        for other in RITUAL_STEPS {
            assert_eq!(found[other], other == *name, "{name}: {other}");
        }
    }
    // another brief, another role's submit, or the reviewer submit in the main session: none
    let none = [
        dispatch(r#"{"task":"Read /r/.stewards/briefs/api-maintainer.md"}"#),
        child_shell("stewards submit api --role tester --file a.json"),
        shell("stewards submit --role=reviewer --file a.json"),
    ];
    assert_eq!(ritual_steps(&none), steps(&[]));
    assert!(
        ritual_steps(&[child_shell("stewards submit --role=reviewer --file a.json")])["reviewer_submit"]
    );
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
        [
            "/work/3/repo/client/client.test.ts",
            "/work/3/repo/api/test_items.py"
        ]
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

/// The third review's real commands: a heredoc whose body has a `\'` swallowed the reads after
/// it; `cd api; cat` names a path under `api`; a glob is returned for the runner to expand.
#[test]
fn shell_reads_skips_heredoc_bodies_and_follows_a_leading_cd() {
    // bare t3 (self1 007), shortened inside the heredoc only
    let command = r#"cd /work/7/repo && python3 - <<'E'
import re
def sub(p,a,b,c=1):
    s=open(p).read(); assert a in s,(p,a); open(p,'w').write(s.replace(a,b,c))
sub('src/status.rs','"| {} | {} | {}/{} | {} | {} | {} | {} |",\n                u.unit,','"| {} | {} | {} | {}/{} | {} | {} | {} | {} |",\n                u.unit,\n                one_line(u.owner.as_deref().unwrap_or("")).replace(\'|\', "\\\\|"),')
E
grep -n "UnitDef {" -A6 src/init.rs tests/check.rs tests/route_workspace.rs tests/route.rs | grep -n "purpose"; grep -rn "purpose |" tests | head"#;
    assert_eq!(
        shell_reads(command),
        [
            "/work/7/repo/src/init.rs",
            "/work/7/repo/tests/check.rs",
            "/work/7/repo/tests/route_workspace.rs",
            "/work/7/repo/tests/route.rs",
            "/work/7/repo/tests",
        ]
    );
    // `<<-` drops leading tabs on the closing line; a quoted `<<` is no heredoc
    assert_eq!(
        shell_reads("cat <<-EOF\n\tcat x.py\n\tEOF\ncat a.py; echo '<<B'; cat b.py"),
        ["a.py", "b.py"]
    );
    assert_eq!(shell_reads("cd api; cat items.py"), ["api/items.py"]);
    assert_eq!(shell_reads("cat docs/units/*.md"), ["docs/units/*.md"]);
}

#[test]
fn existing_files_keeps_regular_files_and_expands_simple_globs() {
    let tmp = TempDir::new().unwrap();
    for f in [
        "src/main.rs",
        "docs/units/api.md",
        "docs/units/cli.md",
        "docs/units/.hidden.md",
    ] {
        let p = tmp.path().join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, "").unwrap();
    }
    let paths = |ps: &[&str]| ps.iter().map(|p| p.to_string()).collect::<Vec<_>>();
    // `src/cli.rs` does not exist and `src` is a directory: neither counts
    assert_eq!(
        existing_files(
            tmp.path(),
            &paths(&["src/main.rs", "src/cli.rs", "src", "tests"])
        ),
        ["src/main.rs"]
    );
    assert_eq!(
        existing_files(
            tmp.path(),
            &paths(&["docs/units/*.md", "docs/units/api.md"])
        ),
        ["docs/units/api.md", "docs/units/cli.md"]
    );
    assert!(existing_files(tmp.path(), &paths(&["*/units/*.md", "nope/*.rs"])).is_empty());
}

#[test]
fn stewards_commands_reads_argv_not_text() {
    let none: Vec<Vec<&str>> = vec![];
    assert_eq!(stewards_commands("echo 'run stewards next'"), none);
    assert_eq!(stewards_commands("grep stewards README.md"), none);
    assert_eq!(
        stewards_commands(
            "cd /r && A=1 stewards -C /r --human next | head; /x/bin/stewards status"
        ),
        [vec!["next"], vec!["status"]]
    );
    assert_eq!(
        stewards_commands("stewards --repo api submit --role reviewer"),
        [vec!["submit", "--role", "reviewer"]]
    );
}

#[test]
fn git_history_reads_are_recognised_by_subcommand() {
    for c in [
        "git log --oneline",
        "cd /r && git -C /r show HEAD~1:AGENTS.md",
        "git -c core.pager=cat reflog",
        "git stash list",
        "git checkout HEAD~1 -- docs",
        "git fsck --lost-found",
        "git cat-file -p abc",
    ] {
        assert!(reads_git_history(c), "{c}");
    }
    for c in ["git status", "git diff", "echo git log", "grep 'git log' x"] {
        assert!(!reads_git_history(c), "{c}");
    }
}

#[test]
fn reviewed_units_reads_a_reviewer_submits_json_output() {
    let out = r#"{"stewards":{"version":"0.1.0","schema":1},"summary":"reviewer: api changes (2 findings; card: gap +a, anchor -b; not stamped), store approve (card unchanged; stamped abc1234)","findings":[{"unit":"api","file":"a.py","line":1,"cut":"x"},{"unit":"api","file":"a.py","line":2,"cut":"y"}]}"#;
    let unit = |u: &str, v: &str, f| ReviewedUnit {
        unit: u.into(),
        verdict: v.into(),
        findings: f,
    };
    assert_eq!(
        reviewed_units(&format!("warning: x\n{out}\n")).unwrap(),
        [unit("api", "changes", 2), unit("store", "approve", 0)]
    );
    assert_eq!(
        reviewed_units(r#"{"summary":"architect: 3 purposes set"}"#),
        None
    );
    assert_eq!(
        reviewed_units(r#"{"error":{"code":"answer_parse_failed"}}"#),
        None
    );
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
        sessions_without_thinking: vec![],
        subagent_sessions: 1,
        reviews: vec![],
        git_history_commands: vec![],
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
        changed: true,
        reviews: vec![],
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

/// The full run `run` of `recs`.
fn full_run(recs: &mut [RunRecord], run: u32) -> &mut RunRecord {
    recs.iter_mut()
        .find(|r| r.arm == Arm::Full && r.run == run)
        .unwrap()
}

#[test]
fn the_gate_passes_only_when_every_full_run_has_all_four_facts() {
    let grade = |recs: &[RunRecord]| grade_task(&fixture_suite(), 1, recs, &ARMS, 3);
    // status and route are recorded, not gated
    let gated = steps(&[
        "next",
        "review_dispatched",
        "reviewer_submit",
        "check_green",
    ]);
    let g = grade(&records(1, 3000, 9000, gated.clone()));
    assert_eq!(g.verdict, Verdict::Pass, "{:?}", g.reasons);
    assert_eq!((g.gate.unwrap().passed, g.gate.unwrap().runs), (3, 3));

    // `next` ran but the review was not dispatched: that run fails the task, with the reason
    let mut recs = records(1, 3000, 9000, gated.clone());
    let r = full_run(&mut recs, 2).result.as_mut().unwrap();
    r.ritual_steps.insert("review_dispatched", false);
    r.ritual_steps.insert("reviewer_submit", false);
    let g = grade(&recs);
    assert_eq!(g.verdict, Verdict::Fail);
    assert_eq!(g.gate.unwrap().passed, 2);
    assert_eq!(
        g.reasons,
        [
            "full run 2: `next` ran but the review was not dispatched; no child ran `stewards submit --role reviewer`"
        ]
    );

    // the runner saw no change, `next` never ran, the runner's own `next` is not clean
    let mut recs = records(1, 3000, 9000, gated.clone());
    full_run(&mut recs, 1).changed = false;
    let r = full_run(&mut recs, 3).result.as_mut().unwrap();
    r.ritual_steps.insert("next", false);
    r.ritual_steps.insert("check_green", false);
    let g = grade(&recs);
    assert_eq!(g.verdict, Verdict::Fail);
    assert_eq!(
        g.reasons,
        [
            "full run 1: changed nothing",
            "full run 3: `stewards next` never ran; the runner's `stewards next --no-brief` did not exit 0",
        ]
    );
}

#[test]
fn a_read_only_task_has_no_gate() {
    let g = grade_task(
        &fixture_suite(),
        4,
        &records(4, 500, 900, steps(&[])),
        &ARMS,
        3,
    );
    assert_eq!(g.verdict, Verdict::NotApplicable, "{:?}", g.reasons);
    assert!(g.gate.is_none() && g.reasons.is_empty());
    // the suite passes on n/a and passing tasks, fails on any failed gate
    let p = |tasks: Vec<u32>, recs: Vec<RunRecord>| {
        let tmp = TempDir::new().unwrap();
        let mut p = plan(tmp.path(), 3, None);
        p.arms = ARMS.to_vec();
        p.tasks = tasks;
        summarize(&p, &recs, false, &[], SetupNotes::default()).verdict
    };
    let mut recs = records(4, 500, 900, steps(&[]));
    recs.extend(records(1, 500, 900, all()));
    assert_eq!(p(vec![1, 4], recs), Verdict::Pass);
    let mut recs = records(4, 500, 900, steps(&[]));
    recs.extend(records(1, 500, 900, steps(&[])));
    assert_eq!(p(vec![1, 4], recs), Verdict::Fail);
}

#[test]
fn overhead_is_reported_per_task_and_as_a_median_across_tasks() {
    // full at 3x bare main, 0.9x total: reported, never judged
    let g = grade_task(&fixture_suite(), 1, &records(1, 3000, 900, all()), &ARMS, 3);
    assert_eq!(g.verdict, Verdict::Pass, "{:?}", g.reasons);
    let ratio = |g: &stewards::eval::TaskGrade, n: &str| {
        g.overhead
            .iter()
            .find(|r| r.name == n)
            .unwrap()
            .ratio
            .unwrap()
    };
    for (name, want) in [
        ("main_tokens", 3.0),
        ("total_tokens", 0.9),
        ("cost_usd", 3.0),
        ("peak_main_context", 3.0),
    ] {
        assert!((ratio(&g, name) - want).abs() < 1e-9, "{name}");
    }
    assert_eq!(g.arms[1].subagent_sessions, [1, 1, 1]);
    let grades: Vec<_> = [(1, 1000), (2, 2000), (3, 6000)]
        .into_iter()
        .map(|(t, main)| grade_task(&fixture_suite(), t, &records(t, main, 900, all()), &ARMS, 3))
        .collect();
    let medians = overhead_medians(&grades);
    let main = medians.iter().find(|m| m.name == "main_tokens").unwrap();
    assert_eq!((main.median, main.tasks), (Some(2.0), 3));
    // an incomplete task's ratios are left out of the medians
    let mut short = grades.clone();
    short[2] = grade_task(
        &fixture_suite(),
        3,
        &records(3, 6000, 900, all())[..5],
        &ARMS,
        3,
    );
    let main = overhead_medians(&short)
        .into_iter()
        .find(|m| m.name == "main_tokens")
        .unwrap();
    assert_eq!((main.median, main.tasks), (Some(1.5), 2));
}

#[test]
fn summary_md_shows_the_gate_the_overhead_the_flags_and_the_review_outcomes() {
    let tmp = TempDir::new().unwrap();
    let mut p = plan(tmp.path(), 3, None);
    p.arms = ARMS.to_vec();
    let mut recs = records(1, 801, 1000, all());
    recs[1].thinking_mismatch = true;
    let r = recs[1].result.as_mut().unwrap();
    r.thinking_levels = vec!["high".into(), "medium".into()];
    r.sessions_without_thinking = vec!["x/run-0/session.jsonl".into()];
    r.git_history_commands = vec!["git log --oneline".into()];
    recs[1].reviews = vec![UnitReview {
        round: 1,
        unit: "api".into(),
        verdict: "changes".into(),
        findings: 2,
        bugs: vec!["pagination skips the last item".into()],
        tests_ran: true,
    }];
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
    assert_eq!(summary.verdict, Verdict::Pass);
    assert_eq!(summary.adoption_in_bare_tasks, Some(16.0));
    let md = render(&summary);
    assert!(md.starts_with("# eval: pass\n"), "{md}");
    assert!(md.contains("| 0.8010 | 1,1,1 |"), "{md}");
    assert!(md.contains("- task 1: pass (gate 3/3) (overhead, full/bare: main_tokens 0.80, total_tokens 1.00, cost_usd 0.80, peak_main_context 0.80)"), "{md}");
    assert!(
        md.contains("overhead medians across tasks, full/bare (reported, not judged): main_tokens 0.80 (1 tasks)"),
        "{md}"
    );
    assert!(
        md.contains("- full-t1-r1: round 1 api changes (2 findings, bugs: pagination skips the last item, tests ran)"),
        "{md}"
    );
    assert!(md.contains("- full-t1-r2: no reviewer submit"), "{md}");
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
    assert!(
        md.contains("no thinking level recorded: full-t1-r1: x/run-0/session.jsonl"),
        "{md}"
    );
    assert!(
        md.contains("git history read: full-t1-r1: git log --oneline"),
        "{md}"
    );
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

/// Task 6 records with the full arm's task-6 files opened at `files`, against bare task 4 at 10.
fn task6(files: usize) -> Vec<RunRecord> {
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
            run_with(1000, 1000, 10, steps(&[])),
        ));
    }
    out
}

#[test]
fn task_6_files_opened_is_reported_against_bare_task_4() {
    let g = grade_task(&fixture_suite(), 6, &task6(6), &ARMS, 3);
    assert_eq!(g.verdict, Verdict::NotApplicable, "{:?}", g.reasons);
    let files = g
        .overhead
        .iter()
        .find(|r| r.name == "files_opened")
        .unwrap();
    assert_eq!((files.full, files.bare), (Some(6.0), Some(10.0)));
    assert!((files.ratio.unwrap() - 0.6).abs() < 1e-9);
}

#[test]
fn a_timed_out_or_missing_run_makes_the_task_incomplete_with_medians_kept() {
    let mut recs = records(1, 500, 900, all());
    recs[1].status = Status::TimedOut; // full run 1
    let g = grade_task(&fixture_suite(), 1, &recs, &ARMS, 3);
    assert_eq!(g.verdict, Verdict::Incomplete);
    assert_eq!(g.reasons, ["full task 1 run 1: timed_out"]);
    assert_eq!(g.arms[1].main_tokens, Some(500.0));
    assert_eq!(g.arms[1].not_ok, 1);

    let mut no_transcript = records(1, 500, 900, all());
    no_transcript[0].result = None;
    no_transcript[0].error = Some("no transcript".into());
    let g = grade_task(&fixture_suite(), 1, &no_transcript, &ARMS, 3);
    assert_eq!(g.verdict, Verdict::Incomplete);
    assert_eq!(g.arms[0].main_tokens, None);

    let short = records(1, 500, 900, all());
    let g = grade_task(&fixture_suite(), 1, &short[..4], &ARMS, 3);
    assert_eq!(g.verdict, Verdict::Incomplete);
    assert!(
        g.reasons
            .iter()
            .any(|r| r == "bare task 1: 2 of 3 runs started")
    );

    let g = grade_task(
        &fixture_suite(),
        6,
        &task6(5)[..task6(5).len() - 1],
        &ARMS,
        3,
    );
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
        assert!(dir.join(".git").is_dir() && !prompt.is_empty());
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
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap())?;
            std::fs::write(path, text)?;
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

fn evals() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("evals")
}

fn fixture_suite() -> Suite {
    Suite::load(&evals(), "fixture").unwrap()
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
        evals: evals(),
        suite: fixture_suite(),
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
    assert_eq!(runs[0]["changed"], false);
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
fn a_new_or_changed_file_outside_the_tools_own_paths_counts_as_changed() {
    for (writes, changed) in [
        (vec![("api/new_file.py", "x\n")], true),
        (vec![("api/items.py", "changed\n")], true),
        // gitignored, or the tool's own
        (vec![("api/items.json", "[]\n")], false),
        (vec![(".stewards/notes.txt", "x\n")], false),
        (vec![], false),
    ] {
        let tmp = TempDir::new().unwrap();
        let mut driver = Replay::new(sample("subagent"), Status::Ok);
        driver.task_writes = writes.clone();
        execute(&driver, &plan(tmp.path(), 1, None)).unwrap();
        let runs = run_files(&tmp.path().join("results"));
        assert_eq!(runs[0]["changed"], changed, "{writes:?}");
    }
}

#[test]
fn no_developers_cuts_the_changing_line_to_its_first_sentence() {
    let block = stewards::guide::block();
    let off = without_developers(block).unwrap();
    assert!(off.contains("\n- Changing: make the edit yourself.\n"));
    assert!(!off.contains("--role developer"));
    let changed: Vec<(&str, &str)> = block
        .lines()
        .zip(off.lines())
        .filter(|(a, b)| a != b)
        .collect();
    assert_eq!(changed.len(), 1, "{changed:?}");
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

/// `(number, read_only, setup, files_opened_vs_bare)` of each task.
fn task_facts(s: &Suite) -> Vec<(u32, bool, Option<u32>, Option<u32>)> {
    s.tasks
        .iter()
        .map(|t| (t.number, t.read_only, t.setup, t.files_opened_vs_bare))
        .collect()
}

#[test]
fn the_fixture_manifest_reproduces_the_old_hard_coded_facts() {
    let s = fixture_suite();
    assert_eq!(s.name, "fixture");
    assert_eq!(s.session_minutes, 30);
    assert_eq!(
        task_facts(&s),
        [
            (1, false, None, None),
            (2, false, None, None),
            (3, false, None, None),
            (4, true, None, None),
            (5, false, None, None),
            (6, true, Some(5), Some(4)),
        ]
    );
    let Source::Fixture {
        dir,
        units,
        unowned,
        tests,
    } = &s.source
    else {
        panic!("{:?}", s.source)
    };
    assert_eq!(dir, Path::new("fixture"));
    assert_eq!(units, Path::new("full-units.toml"));
    assert_eq!(unowned, &[".gitignore", "AGENTS.md"]);
    assert_eq!(
        tests,
        &["api/test_*.py", "client/*.test.ts", "cli/tests/**"]
    );
    assert_eq!(s.commit(), None);
    let tasks = evals().join(&s.tasks_dir);
    assert_eq!(tasks, evals().join("tasks"));
    assert_eq!(
        task_prompt(&tasks, 1).unwrap(),
        include_str!("../evals/tasks/01-count-endpoint.txt")
    );
}

#[test]
fn the_self_manifest_pins_its_commit_and_its_four_tasks() {
    let s = Suite::load(&evals(), "self").unwrap();
    assert_eq!(s.session_minutes, 60);
    assert_eq!(s.commit(), Some("bcfccf0"));
    assert!(
        matches!(&s.source, Source::Repo { build, .. } if build == &["sh", "-c", "cargo build --all-targets && cargo clippy --all-targets"])
    );
    assert_eq!(
        task_facts(&s),
        [
            (1, false, None, None),
            (2, true, None, None),
            (3, false, None, None),
            (4, true, Some(3), Some(2)),
        ]
    );
    let tasks = evals().join(&s.tasks_dir);
    assert_eq!(
        task_prompt(&tasks, 4).unwrap(),
        include_str!("../evals/suites/self/tasks/04-owner-second-session.txt")
    );
}

#[test]
fn an_unknown_suite_or_a_task_outside_the_suite_is_a_usage_error() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for (args, says) in [
        (
            vec!["eval", "--suite", "nope"],
            "unknown suite `nope`; valid: fixture, self",
        ),
        (
            vec!["eval", "--task", "7"],
            "task 7 is not in suite `fixture`; valid: 1, 2, 3, 4, 5, 6",
        ),
        (
            vec!["eval", "--suite", "self", "--task", "5"],
            "task 5 is not in suite `self`; valid: 1, 2, 3, 4",
        ),
    ] {
        let (code, v) = common::run(root, &args);
        assert_eq!(
            (code, v["error"]["code"].as_str()),
            (2, Some("usage")),
            "{v}"
        );
        assert_eq!(v["error"]["message"], says, "{v}");
    }
}

/// A tiny adopted repository standing in for stewards itself: the `single` fixture plus an
/// AGENTS.md with its own notes around an old block, a docs page, and `/target` ignored, committed once before and
/// once after both units are stamped, and an untracked `evals/` whose `self` suite pins the
/// stamped commit and whose build only writes `target/debug/marker`. Returns the repository and
/// the unstamped commit.
fn self_style_repo() -> (TempDir, String) {
    let repo = common::fixture("single");
    let dir = repo.path();
    std::fs::write(
        dir.join("AGENTS.md"),
        "# notes\n\n<!-- stewards:start -->\nold block\n<!-- stewards:end -->\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("docs/units")).unwrap();
    std::fs::write(dir.join("docs/units/api.md"), "# api\n").unwrap();
    std::fs::write(dir.join(".gitignore"), "/target\n").unwrap();
    // prose on store's card keeps its `no docs yet` documenter job away, as in tests/next.rs
    common::append(
        &dir.join(".stewards/units/store.toml"),
        "\n[doc]\noverview = \"Keeps the items.\"\n",
    );
    common::git(dir, &["add", "-A"]);
    common::git(dir, &["commit", "-qm", "notes"]);
    let unstamped = common::git(dir, &["rev-parse", "HEAD"]).trim().to_string();
    common::stamp_units(dir, &["api", "store"]);
    common::git(dir, &["add", "-A"]);
    common::git(dir, &["commit", "-qm", "stamped"]);
    let stamped = common::git(dir, &["rev-parse", "HEAD"]).trim().to_string();
    std::fs::create_dir_all(dir.join("evals/suites")).unwrap();
    std::fs::create_dir_all(dir.join("evals/tasks")).unwrap();
    std::fs::write(
        dir.join("evals/tasks/01-question.txt"),
        "Explain the api.\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("evals/suites/self.toml"),
        format!(
            "tasks_dir = \"tasks\"\nsession_minutes = 60\n\n[source]\nkind = \"repo\"\ncommit = \"{stamped}\"\nbuild = [\"sh\", \"-c\", \"mkdir -p target/debug && echo built > target/debug/marker\"]\n\n[[task]]\nnumber = 1\n"
        ),
    )
    .unwrap();
    (repo, unstamped)
}

fn self_plan(tmp: &Path, repo: &Path) -> Plan {
    let mut p = plan(tmp, 1, None);
    p.evals = repo.join("evals");
    p.suite = Suite::load(&p.evals, "self").unwrap();
    p
}

#[test]
fn a_repo_suite_prepares_each_arm_from_one_clone_and_copies_one_build() {
    use std::os::unix::fs::MetadataExt;
    let (repo, unstamped) = self_style_repo();
    let tmp = TempDir::new().unwrap();
    let mut p = self_plan(tmp.path(), repo.path());
    p.arms = vec![Arm::Bare, Arm::BlockOnly, Arm::Full];
    p.keep_work = true;
    let summary = execute(&Replay::new(sample("subagent"), Status::Ok), &p).unwrap();
    let commit = p.suite.commit().unwrap();
    assert_eq!(summary.suite, "self");
    assert_eq!(summary.commit.as_deref(), Some(commit));
    assert!(summary.setup.build_s.is_some());
    assert!(summary.adoption.is_none(), "no adoption session");
    let sha = common::git(repo.path(), &["rev-parse", commit])
        .trim()
        .to_string();
    assert_eq!(summary.setup.resolved_commit.as_deref(), Some(sha.as_str()));
    let md = render(&summary);
    assert!(
        md.contains(&format!("suite: self at {commit} ({sha})\n")),
        "{md}"
    );
    assert!(md.contains("build cache: one build"), "{md}");
    let runs = run_files(&tmp.path().join("results"));
    assert_eq!(runs.len(), 3);
    let cache = std::fs::metadata(p.work.join("target/debug/marker")).unwrap();
    let block = stewards::guide::block();
    for run in &runs {
        let dir = PathBuf::from(run["workdir"].as_str().unwrap());
        let arm = run["arm"].as_str().unwrap();
        // the one build's output in every arm, a copy: no run writes into the cache
        let marker = dir.join("target/debug/marker");
        assert_eq!(
            std::fs::read_to_string(&marker).unwrap(),
            "built\n",
            "{arm}"
        );
        assert_ne!(
            std::fs::metadata(&marker).unwrap().ino(),
            cache.ino(),
            "{arm}"
        );
        assert_eq!(common::git(&dir, &["status", "--porcelain"]), "", "{arm}");
        let agents = std::fs::read_to_string(dir.join("AGENTS.md")).ok();
        let kept = dir.join(".stewards").exists() || dir.join("docs/units").exists();
        match arm {
            "bare" => assert!(agents.is_none() && !kept),
            "block-only" => {
                assert!(!kept);
                assert_eq!(agents.unwrap(), format!("{block}\n"));
            }
            "full" => {
                assert!(dir.join(".stewards/units.toml").is_file());
                assert!(dir.join("docs/units/api.md").is_file());
                let agents = agents.unwrap();
                assert!(agents.starts_with("# notes\n") && agents.contains(block));
                assert!(!agents.contains("old block"));
                assert_eq!(run["next_check"]["exit_code"], 0);
            }
            other => panic!("{other}"),
        }
        // one root commit on `main` and no other ref, reflog, or remote
        assert_eq!(
            common::git(&dir, &["log", "--format=%s"]),
            "initial\n",
            "{arm}"
        );
        assert_eq!(
            common::git(&dir, &["for-each-ref", "--format=%(refname)"]),
            "refs/heads/main\n",
            "{arm}"
        );
        assert_eq!(common::git(&dir, &["reflog", "--all"]), "", "{arm}");
        assert_eq!(common::git(&dir, &["remote"]), "", "{arm}");
        // only `full` keeps the old objects, which hold its units' stamped trees
        let old = std::process::Command::new("git")
            .args(["cat-file", "-e", &unstamped])
            .current_dir(&dir)
            .status()
            .unwrap();
        assert_eq!(old.success(), arm == "full", "{arm}");
    }
}

/// The subagent sample with its child as a reviewer: it runs `pytest tests/api`, then a reviewer
/// `submit` whose output says api has one finding and store is approved.
fn reviewer_sample(tmp: &Path) -> PathBuf {
    let dir = tmp.join("reviewer-sample");
    std::fs::create_dir_all(&dir).unwrap();
    copy_dir(&sample("subagent"), &dir);
    let child = main_file(&dir)
        .with_extension("")
        .join("b5d51927-cb55-4247-a969-9448b23c2c26/run-0/session.jsonl");
    let call = |id: &str, command: &str| {
        serde_json::json!({"type": "message", "id": id, "parentId": null, "timestamp": "2026-11-12T19:28:00.000Z",
            "message": {"role": "assistant", "content": [{"type": "toolCall", "id": id, "name": "bash",
                "arguments": {"command": command}}], "stopReason": "toolUse"}})
        .to_string()
    };
    let output = r#"{"stewards":{"version":"0.1.0","schema":1},"summary":"reviewer: api changes (1 finding; card unchanged; not stamped), store approve (card: gap +loses writes; stamped abc1234)","findings":[{"unit":"api","file":"src/api/items.py","line":1,"cut":"x"}]}"#;
    let result = serde_json::json!({"type": "message", "id": "res2", "parentId": null, "timestamp": "2026-11-12T19:28:01.000Z",
        "message": {"role": "toolResult", "toolCallId": "call2", "toolName": "bash",
            "content": [{"type": "text", "text": output}]}})
    .to_string();
    let text = std::fs::read_to_string(&child).unwrap()
        + &[
            call("call1", "cd /r && pytest tests/api -q"),
            call(
                "call2",
                "stewards submit --role reviewer --file .stewards/briefs/review.answer.json",
            ),
            result,
        ]
        .join("\n")
        + "\n";
    std::fs::write(&child, text).unwrap();
    dir
}

#[test]
fn a_full_runs_review_outcome_comes_from_the_reviewer_submit_and_the_cards() {
    let (repo, _) = self_style_repo();
    let tmp = TempDir::new().unwrap();
    let mut p = self_plan(tmp.path(), repo.path());
    p.arms = vec![Arm::Bare, Arm::Full];
    let mut driver = Replay::new(reviewer_sample(tmp.path()), Status::Ok);
    let card = include_str!("fixtures/single/.stewards/units/store.toml").to_string()
        + "[[gap]]\nkind = \"bug\"\ntitle = \"loses writes\"\n";
    driver.task_writes = vec![(
        ".stewards/units/store.toml",
        Box::leak(card.into_boxed_str()),
    )];
    execute(&driver, &p).unwrap();
    let runs = run_files(&tmp.path().join("results"));
    let review = |arm: &str| runs.iter().find(|r| r["arm"] == arm).unwrap()["reviews"].clone();
    assert_eq!(review("bare"), serde_json::json!([]));
    assert_eq!(
        review("full"),
        serde_json::json!([
            {"round": 1, "unit": "api", "verdict": "changes", "findings": 1, "bugs": [], "tests_ran": true},
            {"round": 1, "unit": "store", "verdict": "approve", "findings": 0, "bugs": ["loses writes"], "tests_ran": false},
        ])
    );
}

/// `full` keeps the clone's objects, so after an edit the review pack carries hunks from the
/// stamped tree, not every owned file as a first review would.
#[test]
fn after_an_edit_in_a_repo_suites_full_copy_the_review_pack_shows_hunks() {
    let (repo, _) = self_style_repo();
    let tmp = TempDir::new().unwrap();
    let mut p = self_plan(tmp.path(), repo.path());
    p.arms = vec![Arm::Bare, Arm::Full];
    p.keep_work = true;
    let mut driver = Replay::new(sample("subagent"), Status::Ok);
    driver.task_writes = vec![(
        "src/api/items.py",
        "def create_item(name):\n    return name\n",
    )];
    execute(&driver, &p).unwrap();
    let runs = run_files(&tmp.path().join("results"));
    let full = runs.iter().find(|r| r["arm"] == "full").unwrap();
    assert_eq!(full["changed"], true);
    let dir = PathBuf::from(full["workdir"].as_str().unwrap());
    let (code, v) = common::run(&dir, &["next"]);
    assert_eq!(code, 1, "{v}");
    let brief = std::fs::read_to_string(dir.join(".stewards/briefs/review.md")).unwrap();
    assert!(
        brief.contains("## Hunks") && brief.contains("```diff"),
        "{brief}"
    );
    assert!(!brief.contains("has not been reviewed yet"), "{brief}");
}

#[test]
fn a_repo_suite_whose_stamped_tree_is_missing_stops_before_any_run() {
    let (repo, _) = self_style_repo();
    let dir = repo.path();
    let card = dir.join(".stewards/units/api.toml");
    let text = std::fs::read_to_string(&card).unwrap();
    let at = text.find("tree = \"").unwrap() + "tree = \"".len();
    let fake = "0123456789abcdef0123456789abcdef01234567";
    std::fs::write(&card, format!("{}{fake}{}", &text[..at], &text[at + 40..])).unwrap();
    common::git(dir, &["commit", "-qam", "fake stamp"]);
    let tmp = TempDir::new().unwrap();
    let mut p = self_plan(tmp.path(), dir);
    let Source::Repo { commit, .. } = &mut p.suite.source else {
        panic!()
    };
    *commit = common::git(dir, &["rev-parse", "HEAD"]).trim().to_string();
    p.arms = vec![Arm::Bare, Arm::Full];
    let err = setup_error(
        execute(&Replay::new(sample("subagent"), Status::Ok), &p)
            .err()
            .unwrap(),
    );
    assert!(
        err.message.contains("lacks the stamped trees of: api"),
        "{}",
        err.message
    );
    assert!(!tmp.path().join("results/runs").exists(), "no run started");
}

#[test]
fn a_repo_suite_whose_full_tree_lists_jobs_stops_before_any_run() {
    let (repo, unstamped) = self_style_repo();
    let tmp = TempDir::new().unwrap();
    let mut p = self_plan(tmp.path(), repo.path());
    let Source::Repo { commit, .. } = &mut p.suite.source else {
        panic!()
    };
    *commit = unstamped;
    p.arms = vec![Arm::Bare, Arm::Full];
    let err = setup_error(
        execute(&Replay::new(sample("subagent"), Status::Ok), &p)
            .err()
            .unwrap(),
    );
    assert!(
        err.message
            .contains("`stewards next` is not clean in the `full` tree"),
        "{}",
        err.message
    );
    assert!(err.fix.contains("evals/suites/self.toml"), "{}", err.fix);
    assert!(!tmp.path().join("results/runs").exists(), "no run started");
}

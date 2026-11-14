#[allow(dead_code)] // each test crate uses a subset of the helpers
mod common;

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use common::copy_dir;
use stewards::eval::claude::{ClaudeCode, config_dir, parse as claude_parse};
use stewards::eval::grade::{grade_task, median, overhead_medians, render, summarize};
use stewards::eval::hermes::{Hermes, fingerprint, parse as hermes_parse, template};
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
    assert_eq!(run.main_tokens.unwrap(), main);
    let mut total = main;
    total.add(child);
    assert_eq!(run.total_tokens.unwrap(), total);
    assert_eq!(run.total_tokens.unwrap().sum(), 60223 + 6148);
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
    assert_eq!(run.peak_main_context, Some(15564));
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
    assert_eq!(
        after.total_tokens.unwrap().sum(),
        before.total_tokens.unwrap().sum() + 10
    );
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
    assert_eq!(run.main_tokens.unwrap(), main);
    assert_eq!(run.total_tokens.unwrap().sum(), 96167 + 100879 + 49497);
    assert_eq!(run.peak_main_context, Some(18243));
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

/// A pi-subagents launch whose child gets `task`, beside the agent name.
fn dispatch(task: &str) -> Action {
    Action::Dispatch {
        tasks: vec![task.into()],
    }
}

const HANDOFF: &str = "Read `/r r/.stewards/briefs/api-maintainer-0123abcd.md` and follow it.";

#[test]
fn each_ritual_step_is_detected_alone_and_absent_otherwise() {
    let cases: [(&[&str], Vec<Action>); 7] = [
        (&["status"], vec![shell("stewards status")]),
        (&["route"], vec![shell("cd /r && stewards route a.py")]),
        (&["next"], vec![child_shell("stewards next")]),
        (&["review"], vec![shell("stewards review --task 'x'")]),
        // any brief's handoff, its path with a space, passed as the whole task
        (&["handoff", "handoff_exact"], vec![dispatch(HANDOFF)]),
        // the handoff inside the agent's own wording: handed off, not exactly
        (
            &["handoff"],
            vec![
                dispatch(HANDOFF),
                dispatch(&format!("{HANDOFF} Return a short summary of findings.")),
            ],
        ),
        (
            &["reviewer_submit"],
            vec![child_shell(
                "cd /r && stewards submit --role reviewer --file .stewards/briefs/review.answer.json",
            )],
        ),
    ];
    for (names, actions) in &cases {
        let found = ritual_steps(actions);
        for other in RITUAL_STEPS {
            assert_eq!(found[other], names.contains(&other), "{names:?}: {other}");
        }
    }
    // a brief path without the sentence, a relative path, a path outside the code span, a name
    // with no snapshot (an older stewards') or not a brief's, another role's submit, or the
    // reviewer submit in the main session: none
    let none = [
        dispatch("Read `/r/.stewards/briefs/review-0123abcd.md`"),
        dispatch("Read `.stewards/briefs/review-0123abcd.md` and follow it."),
        dispatch("Read `/r/notes.md` and follow it."),
        dispatch("Read /r/.stewards/briefs/review-0123abcd.md and follow it."),
        dispatch("Read `/r/.stewards/briefs/review.md` and follow it."),
        dispatch("Read `/r/.stewards/briefs/review-0123abcd.answer.json` and follow it."),
        dispatch("Read `/r/.stewards/briefs/notes-0123abcd.md` and follow it."),
        child_shell("stewards submit api --role tester --file a.json"),
        shell("stewards submit --role=reviewer --file a.json"),
    ];
    assert_eq!(ritual_steps(&none), steps(&[]));
    assert!(
        ritual_steps(&[child_shell("stewards submit --role=reviewer --file a.json")])["reviewer_submit"]
    );
}

#[test]
fn the_handoff_brief_prints_is_one_the_eval_reads_as_exact() {
    let h = stewards::brief::handoff(std::path::Path::new(
        "/w/my repo/.stewards/briefs/review-0123abcd.md",
    ))
    .unwrap();
    assert!(stewards::eval::is_handoff(&h), "{h}");
    let old = h.replace("review-0123abcd.md", "review.md");
    assert!(!stewards::eval::is_handoff(&old), "{old}");
    assert!(stewards::eval::holds_handoff(&format!(
        "please: {h} thanks"
    )));
}

/// The recorded sample `name` with each `(from, to)` replaced: the real pi-subagents argument
/// shapes (one `subagent` call with `agent`, `task`, `model`, and `options`; a `parallel`
/// workflow with `args.tasks[]`) with other task strings.
fn with_tasks(name: &str, replace: &[(&str, &str)]) -> Run {
    let tmp = TempDir::new().unwrap();
    copy_dir(&sample(name), tmp.path());
    let parent = main_file(tmp.path());
    let mut text = std::fs::read_to_string(&parent).unwrap();
    for (from, to) in replace {
        assert!(text.contains(from), "{name}: {from}");
        text = text.replace(from, to);
    }
    std::fs::write(&parent, text).unwrap();
    parse(&parent, 0.0, false).unwrap()
}

#[test]
fn handoff_exact_reads_the_task_of_a_real_pi_subagent_call() {
    let single = r#""task":"Reply with the word ok and nothing else.""#;
    let h = r#""task":"Read `/work/0/repo/.stewards/briefs/architect-0123abcd.md` and follow it.""#;
    // every child of the parallel call: its brief path, given a snapshot, inside the handoff
    let (start, end) = (
        r#""task":"Execute the stewards brief at "#,
        r#".md. You have shell access. Follow the brief, then reply with a short summary of what you did and any files written.""#,
    );
    let exact = [
        with_tasks("subagent", &[(single, h)]),
        with_tasks(
            "ritual",
            &[
                (start, r#""task":"Read `"#),
                (end, r#"-0123abcd.md` and follow it.""#),
            ],
        ),
    ];
    for run in exact {
        assert!(run.ritual_steps["handoff"] && run.ritual_steps["handoff_exact"]);
    }
    let added = r#"` and follow it. Then summarize.""#;
    let not_exact = [
        with_tasks(
            "subagent",
            &[(single, &h.replace("` and follow it.\"", added))],
        ),
        with_tasks(
            "ritual",
            &[
                (start, r#""task":"Read `"#),
                (end, &format!("-0123abcd.md{added}")),
            ],
        ),
    ];
    for run in not_exact {
        assert!(run.ritual_steps["handoff"] && !run.ritual_steps["handoff_exact"]);
    }
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
        main_tokens: Some(t(main)),
        total_tokens: Some(t(total)),
        cache_warming: Tokens::default(),
        peak_main_context: Some(main / 2),
        tool_calls: vec![],
        files_opened: files.clone(),
        files_opened_tree: files,
        shell_commands: vec![],
        ritual_steps: ritual,
        cost_usd: Some(main as f64 / 1000.0),
        harness_reported_cost_usd: None,
        duration_s: 1.0,
        model: None,
        models: vec![],
        thinking_levels: vec![],
        sessions_without_thinking: vec![],
        subagent_sessions: 1,
        uncounted_sessions: vec![],
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
    // status, route, next, handoff_exact, and reviewer_submit are recorded, not gated
    let gated = steps(&["review", "handoff", "check_green"]);
    let g = grade(&records(1, 3000, 9000, gated.clone()));
    assert_eq!(g.verdict, Verdict::Pass, "{:?}", g.reasons);
    assert_eq!((g.gate.unwrap().passed, g.gate.unwrap().runs), (3, 3));

    // `review` ran but no subagent got its handoff: that run fails the task, with the reason
    let mut recs = records(1, 3000, 9000, gated.clone());
    let r = full_run(&mut recs, 2).result.as_mut().unwrap();
    r.ritual_steps.insert("handoff", false);
    let g = grade(&recs);
    assert_eq!(g.verdict, Verdict::Fail);
    assert_eq!(g.gate.unwrap().passed, 2);
    assert_eq!(g.reasons, ["full run 2: no subagent was given a handoff"]);

    // the runner saw no change, `review` never ran, the runner's own `next` is not clean
    let mut recs = records(1, 3000, 9000, gated.clone());
    full_run(&mut recs, 1).changed = false;
    let r = full_run(&mut recs, 3).result.as_mut().unwrap();
    r.ritual_steps.insert("review", false);
    r.ritual_steps.insert("check_green", false);
    let g = grade(&recs);
    assert_eq!(g.verdict, Verdict::Fail);
    assert_eq!(
        g.reasons,
        [
            "full run 1: changed nothing",
            "full run 3: `stewards review` never ran; the runner's `stewards next --no-brief` did not exit 0",
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
    assert_eq!(run.main_tokens.unwrap().output, 743);
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
/// process running in the run directory, and writes `adopt_writes` (then stamps `adopt_stamps`)
/// on the adoption prompt and `task_writes` on any other.
struct Replay {
    sample: PathBuf,
    status: Status,
    adopt_writes: Vec<(&'static str, &'static str)>,
    adopt_stamps: &'static [&'static str],
    task_writes: Vec<(&'static str, &'static str)>,
    context: Option<(&'static str, &'static str)>,
}

impl Replay {
    fn new(sample: PathBuf, status: Status) -> Replay {
        Replay {
            sample,
            status,
            adopt_writes: vec![],
            adopt_stamps: &[],
            task_writes: vec![],
            context: None,
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
        if prompt == ADOPT_PROMPT {
            common::stamp_units(dir, self.adopt_stamps);
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

    fn context_file(&self) -> Option<(&'static str, &'static str)> {
        self.context
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
        harness: "pi",
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
    // its counts are partial, so the runner nulls them on every driver
    assert_eq!(runs[0]["result"]["main_tokens"], serde_json::Value::Null);
    assert_eq!(runs[0]["result"]["total_tokens"], serde_json::Value::Null);
    assert!(runs[0]["result"]["cost_usd"].is_number());
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
    // every unit its own test command, and none repo-wide, so no unit is shown another's
    let loaded = stewards::model::load_repo(&adopted).unwrap();
    assert!(loaded.repo.commands.test.is_empty());
    for (unit, cmd) in [
        ("api", "python3 -m unittest discover -s api"),
        ("client", "node --test client/"),
        ("cli", "cargo test --manifest-path cli/Cargo.toml"),
    ] {
        assert_eq!(loaded.tests[unit].test_command.as_deref(), Some(cmd));
    }
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
        test_commands,
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
    // each unit's own command, run from the root as the fixture's tests are
    let commands: Vec<(&str, &str)> = test_commands
        .iter()
        .map(|(u, c)| (u.as_str(), c.as_str()))
        .collect();
    assert_eq!(
        commands,
        [
            ("api", "python3 -m unittest discover -s api"),
            ("cli", "cargo test --manifest-path cli/Cargo.toml"),
            ("client", "node --test client/"),
        ]
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
fn the_self_manifest_takes_the_checkouts_commit_and_its_four_tasks() {
    let s = Suite::load(&evals(), "self").unwrap();
    assert_eq!(s.session_minutes, 60);
    assert_eq!(s.commit(), Some("HEAD"));
    let probes: Vec<(u32, &str, &str)> = s
        .absent
        .iter()
        .map(|a| (a.task, a.file.as_str(), a.text.as_str()))
        .collect();
    assert_eq!(
        probes,
        [
            (1, "src/main.rs", "Debt {"),
            (1, "src/main.rs", "Debt("),
            (3, "src/model.rs", "owner"),
        ]
    );
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

/// `commit = "HEAD"` is the checkout's commit at the start, refused with uncommitted source or
/// with an `absent` probe's text already in the tree.
#[test]
fn a_head_pinned_repo_suite_takes_the_checkouts_commit_and_refuses_a_dirty_or_done_tree() {
    let (repo, _) = self_style_repo();
    let dir = repo.path();
    let manifest = dir.join("evals/suites/self.toml");
    let pinned = std::fs::read_to_string(&manifest).unwrap();
    let head = common::git(dir, &["rev-parse", "HEAD"]).trim().to_string();
    let with = |probe: &str| {
        let text = pinned.replace(&head, "HEAD")
            + &format!("\n[[absent]]\ntask = 1\nfile = \"src/api/items.py\"\ntext = \"{probe}\"\n");
        std::fs::write(&manifest, text).unwrap();
    };
    let tmp = TempDir::new().unwrap();
    let run = |arms: Vec<Arm>| {
        let mut p = self_plan(tmp.path(), dir);
        p.arms = arms;
        execute(&Replay::new(sample("subagent"), Status::Ok), &p)
    };
    // the task's change is already there
    with("def create_item");
    let e = setup_error(run(vec![Arm::Bare]).err().unwrap());
    assert!(e.message.contains("task 1's change is already in"), "{e}");
    // an edit to the source the binary may not have been built from
    with("def count_items_by_name");
    common::append(&dir.join("src/api/items.py"), "# edited\n");
    let e = setup_error(run(vec![Arm::Bare]).err().unwrap());
    assert!(e.message.contains("uncommitted changes"), "{e}");
    common::git(dir, &["checkout", "--", "src/api/items.py"]);
    let summary = run(vec![Arm::Bare]).unwrap();
    assert_eq!(summary.commit.as_deref(), Some("HEAD"));
    assert_eq!(
        summary.setup.resolved_commit.as_deref(),
        Some(head.as_str())
    );
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
    let brief = std::fs::read_to_string(dir.join(common::brief(&dir, "review"))).unwrap();
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

fn claude_sample(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/transcripts/claude-code")
        .join(name)
}

/// The usage fields of a Claude Code usage object (`result.usage` in the stream).
fn claude_tokens(u: &serde_json::Value) -> Tokens {
    let n = |k: &str| u[k].as_u64().unwrap();
    Tokens {
        input: n("input_tokens"),
        output: n("output_tokens"),
        cache_read: n("cache_read_input_tokens"),
        cache_write: n("cache_creation_input_tokens"),
    }
}

/// The stream's `result` line of a recorded Claude Code sample.
fn claude_result(name: &str) -> serde_json::Value {
    std::fs::read_to_string(claude_sample(name).join("stdout.log"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .find(|v| v["type"] == "result")
        .unwrap()
}

/// The trimmed real Haiku run where the parent started one `general-purpose` child with `Task`
/// and the child ran `echo child`. The stream's own figures check the parse: its `result.usage`
/// is the parent alone, its `modelUsage` the parent plus the child.
#[test]
fn claude_sample_counts_child_tokens_in_total_and_not_in_main() {
    let run = claude_parse(&main_file(&claude_sample("subagent")), 2.0, false).unwrap();
    let result = claude_result("subagent");
    let main = Tokens {
        input: 6,
        output: 126,
        cache_read: 41676,
        cache_write: 131,
    };
    assert_eq!(run.main_tokens.unwrap(), main);
    assert_eq!(run.main_tokens.unwrap(), claude_tokens(&result["usage"]));
    let child = Tokens {
        input: 6,
        output: 77,
        cache_read: 15320,
        cache_write: 15402,
    };
    let mut total = main;
    total.add(child);
    assert_eq!(run.total_tokens.unwrap(), total);
    let m = &result["modelUsage"]["claude-haiku-5-5"];
    let n = |k: &str| m[k].as_u64().unwrap();
    assert_eq!(
        run.total_tokens.unwrap(),
        Tokens {
            input: n("inputTokens"),
            output: n("outputTokens"),
            cache_read: n("cacheReadInputTokens"),
            cache_write: n("cacheCreationInputTokens"),
        }
    );
    assert_eq!(run.tool_calls, ["Task"]);
    assert!(run.shell_commands.is_empty());
    assert_eq!(run.subagent_sessions, 1);
    assert_eq!(run.model.as_deref(), Some("claude-haiku-5-5"));
    assert_eq!(run.models, ["claude-haiku-5-5"]);
    // no thinking block in the parent or the child; the stream's cost prices this model at
    // another's rates
    assert_eq!(run.thinking_levels, ["off"]);
    assert!(run.sessions_without_thinking.is_empty());
    assert_eq!(run.cost_usd, None);
    assert_eq!(run.harness_reported_cost_usd, Some(0.07842855));
    assert_eq!(run.peak_main_context, Some(20838 + 131 + 2));
    assert_eq!(run.ritual_steps, steps(&[]));
    assert_eq!(run.stop_reason, None);
    assert!(run.failed_children.is_empty() && run.reviews.is_empty());
    assert_eq!(run.duration_s, 2.0);
}

/// The trimmed real Haiku run that ran one `Bash` command in the main session.
#[test]
fn claude_plain_sample_has_main_equal_to_total() {
    let run = claude_parse(&main_file(&claude_sample("plain")), 0.0, false).unwrap();
    assert_eq!(run.main_tokens, run.total_tokens);
    assert_eq!(
        run.main_tokens.unwrap(),
        claude_tokens(&claude_result("plain")["usage"])
    );
    // the larger of its two responses' input and cache counts
    assert_eq!(run.peak_main_context, Some(2 + 21022 + 404));
    assert_eq!(run.tool_calls, ["Bash"]);
    assert_eq!(
        run.shell_commands,
        ["env | cut -d= -f1 | grep -i -E 'claude|anthropic' | sort"]
    );
    assert_eq!(run.subagent_sessions, 0);
    assert!(run.files_opened.is_empty());
    // Haiku 5.5 emits an empty thinking block with no budget: that is not thinking
    assert_eq!(run.thinking_levels, ["off"]);
}

/// The trimmed real run whose child was started on the harness's retired default model: Claude
/// Code persisted the API failure as the child's one response.
#[test]
fn a_claude_child_that_hit_an_api_error_is_a_failed_child() {
    let dir = claude_sample("subagent-error");
    let run = claude_parse(&main_file(&dir), 0.0, false).unwrap();
    assert_eq!(run.subagent_sessions, 1);
    assert_eq!(run.failed_children.len(), 1, "{:?}", run.failed_children);
    assert!(run.failed_children[0].ends_with("stopped with `error`"));
    // the failure's placeholder model is not a model the tree used
    assert_eq!(run.models, ["claude-haiku-5-5"]);
    assert_eq!(run.stop_reason, None);
    // the child has no real response, so its thinking is unknown
    assert_eq!(run.thinking_levels, ["off"]);
    assert_eq!(run.sessions_without_thinking.len(), 1);
}

/// The records' output counts fall short of the final ones, so a session whose stream ended
/// before its `result` line has no measured tokens; the rest of the run still parses.
#[test]
fn a_claude_session_without_a_result_line_has_null_tokens() {
    let tmp = TempDir::new().unwrap();
    copy_dir(&claude_sample("subagent"), tmp.path());
    let stream = tmp.path().join("stdout.log");
    let text = std::fs::read_to_string(&stream).unwrap();
    let cut: Vec<&str> = text
        .lines()
        .filter(|l| !l.contains(r#""type":"result""#))
        .collect();
    std::fs::write(&stream, cut.join("\n")).unwrap();
    let run = claude_parse(&main_file(tmp.path()), 0.0, true).unwrap();
    assert_eq!((run.main_tokens, run.total_tokens), (None, None));
    assert_eq!(run.harness_reported_cost_usd, None);
    assert_eq!(run.peak_main_context, Some(20838 + 131 + 2));
    assert_eq!(run.subagent_sessions, 1);
    assert_eq!(run.tool_calls, ["Task"]);
}

#[test]
fn a_claude_session_whose_stream_reports_an_error_stopped_with_error() {
    let tmp = TempDir::new().unwrap();
    copy_dir(&claude_sample("plain"), tmp.path());
    let stream = tmp.path().join("stdout.log");
    let text = std::fs::read_to_string(&stream).unwrap();
    std::fs::write(
        &stream,
        text.replace(r#""is_error":false"#, r#""is_error":true"#),
    )
    .unwrap();
    let run = claude_parse(&main_file(tmp.path()), 0.0, false).unwrap();
    assert_eq!(run.stop_reason.as_deref(), Some("error"));
}

#[test]
fn a_missing_claude_binary_is_the_same_harness_missing_error() {
    let empty = TempDir::new().unwrap();
    let err = ClaudeCode::find(empty.path().as_os_str(), Duration::from_secs(1))
        .err()
        .unwrap();
    let err = err.downcast_ref::<CliError>().unwrap();
    assert_eq!(err.code, "harness_missing");
    assert!(err.fix.contains("Claude Code"), "{}", err.fix);
}

#[test]
fn the_eval_claude_config_dir_holds_only_the_model_and_linked_credentials() {
    let tmp = TempDir::new().unwrap();
    let user = tmp.path().join("user");
    std::fs::create_dir_all(user.join("agents")).unwrap();
    std::fs::write(user.join("agents/mine.md"), "---\nname: mine\n---\n").unwrap();
    std::fs::write(user.join("CLAUDE.md"), "personal rules").unwrap();
    std::fs::write(user.join(".credentials.json"), "{}").unwrap();
    std::fs::write(
        user.join("settings.json"),
        r#"{"model":"claude-haiku-5-5","hooks":{"Stop":[]},"enabledPlugins":{"x":true}}"#,
    )
    .unwrap();
    let dir = tmp.path().join("a");
    let (note, model) = config_dir(&user, &dir).unwrap();
    assert_eq!(model.as_deref(), Some("claude-haiku-5-5"));
    let settings: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("settings.json")).unwrap()).unwrap();
    assert_eq!(settings, serde_json::json!({ "model": "claude-haiku-5-5" }));
    assert_eq!(
        std::fs::read_link(dir.join(".credentials.json")).unwrap(),
        user.join(".credentials.json")
    );
    for absent in ["CLAUDE.md", "agents", ".claude.json"] {
        assert!(!dir.join(absent).exists(), "{absent}");
    }
    assert!(note.contains("does not read AGENTS.md"), "{note}");
    assert!(note.contains("Thinking: off"), "{note}");
}

#[test]
fn claude_driver_passes_the_model_and_config_dir_and_keeps_the_persisted_session() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempDir::new().unwrap();
    let bin = tmp.path().join("claude");
    // records argv and env, streams its init line, persists a session file, and exits
    std::fs::write(
        &bin,
        "#!/bin/sh\necho \"$@\" > ../args\nenv > ../env\necho '{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"s1\"}'\nmkdir -p \"$CLAUDE_CONFIG_DIR/projects/-r\"\necho '{\"type\":\"user\",\"isSidechain\":false,\"uuid\":\"u\",\"cwd\":\"/r\"}' > \"$CLAUDE_CONFIG_DIR/projects/-r/s1.jsonl\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let claude = ClaudeCode {
        bin,
        limit: Duration::from_secs(20),
        config_dir: Some(tmp.path().join("a")),
        default_model: Some("user-default".into()),
    };
    let dir = tmp.path().join("run/repo");
    std::fs::create_dir_all(&dir).unwrap();
    let sessions = tmp.path().join("sessions");
    let env = [
        ("PATH", std::env::var_os("PATH").unwrap()),
        ("CLAUDECODE", "1".into()),
        ("MAX_THINKING_TOKENS", "1024".into()),
    ];
    let s = claude
        .start(&dir, "hi", Some("m-1"), "medium", &sessions, &env)
        .unwrap();
    assert_eq!(s.status, Status::Ok);
    assert_eq!(s.transcript, Some(sessions.join("s1.jsonl")));
    assert!(sessions.join("s1.jsonl").is_file());
    let args = std::fs::read_to_string(tmp.path().join("run/args")).unwrap();
    assert!(
        args.starts_with("-p --output-format stream-json --verbose"),
        "{args}"
    );
    assert!(args.contains("--model m-1 -- hi"), "{args}");
    let vars = std::fs::read_to_string(tmp.path().join("run/env")).unwrap();
    let config = format!("CLAUDE_CONFIG_DIR={}\n", tmp.path().join("a").display());
    assert!(vars.contains(&config), "{vars}");
    // children on the requested model, not the user's default
    assert!(vars.contains("CLAUDE_CODE_SUBAGENT_MODEL=m-1\n"), "{vars}");
    assert!(!vars.contains("CLAUDECODE="), "{vars}");
    // no inherited thinking budget
    assert!(!vars.contains("MAX_THINKING_TOKENS="), "{vars}");
}

/// A copy of a recorded sample with each `(from, to)` replaced in every file, and its transcript.
fn replaced(dir: &Path, transcript: &Path, replace: &[(&str, &str)]) -> (TempDir, PathBuf) {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, out);
            } else {
                out.push(p);
            }
        }
    }
    let tmp = TempDir::new().unwrap();
    copy_dir(dir, tmp.path());
    let mut files = vec![];
    walk(tmp.path(), &mut files);
    for (from, _) in replace {
        assert!(
            files
                .iter()
                .any(|f| std::fs::read_to_string(f).unwrap().contains(from)),
            "{}: {from}",
            dir.display()
        );
    }
    for f in files {
        let mut text = std::fs::read_to_string(&f).unwrap();
        for (from, to) in replace {
            text = text.replace(from, to);
        }
        std::fs::write(&f, text).unwrap();
    }
    let t = tmp.path().join(transcript.strip_prefix(dir).unwrap());
    (tmp, t)
}

/// The same assertions over every driver's recorded subagent sample, so their mapping into `Run`
/// and the harness-neutral `Action`s cannot drift: child tokens are in total and not in main, the
/// launch is a `Dispatch` with the child's task (exact handoff or not), a shell command is a
/// `Shell` in the session that ran it, and the gate passes on those runs unchanged.
#[test]
fn every_driver_maps_its_subagent_sample_the_same_way() {
    let pi = Pi {
        bin: PathBuf::new(),
        limit: Duration::from_secs(1),
        agent_dir: None,
    };
    let claude = ClaudeCode {
        bin: PathBuf::new(),
        limit: Duration::from_secs(1),
        config_dir: None,
        default_model: None,
    };
    let hermes = Hermes {
        bin: PathBuf::new(),
        limit: Duration::from_secs(1),
        template: None,
        installs: vec![],
    };
    // driver, sample, the child's whole task as the transcript holds it, a shell command it ran
    // with the text around it (`{}` for the command), whether a child ran it
    type Case<'a> = (&'a dyn Driver, PathBuf, &'a str, [&'a str; 2], bool);
    let cases: [Case; 3] = [
        (
            &pi,
            sample("subagent"),
            "Reply with the word ok and nothing else.",
            [r#""command":"echo probe""#, r#""command":"{}""#],
            false,
        ),
        (
            &claude,
            claude_sample("subagent"),
            "Run `echo child` with the Bash tool and reply with one word.",
            [r#""command":"echo child""#, r#""command":"{}""#],
            true,
        ),
        // Hermes's arguments are a JSON string: the child's goal and its context, escaped
        (
            &hermes,
            hermes_sample("subagent"),
            r#"Run `echo child` in the terminal and reply with one word.\", \"context\": \"You have a terminal tool. Execute the shell command `echo child` and then reply with its output as a single word."#,
            [r#"\"command\": \"echo child\""#, r#"\"command\": \"{}\""#],
            true,
        ),
    ];
    let parse = |d: &dyn Driver, t: &Path| d.parse(t, 1.0, false).unwrap();
    for (driver, dir, task, [command, around], by_child) in cases {
        let name = dir.display().to_string();
        let transcript = main_file(&dir);
        let run = parse(driver, &transcript);
        assert_eq!(run.subagent_sessions, 1, "{name}");
        assert!(run.main_tokens.unwrap().sum() > 0, "{name}");
        assert!(
            run.total_tokens.unwrap().sum() > run.main_tokens.unwrap().sum(),
            "{name}"
        );
        assert_eq!(run.ritual_steps, steps(&[]), "{name}");

        let review = around.replace("{}", "stewards review --task 'x'");
        let submit = around.replace("{}", "stewards submit --role reviewer --file a.json");
        let (_t, t) = replaced(&dir, &transcript, &[(task, HANDOFF), (command, &review)]);
        let run = parse(driver, &t);
        assert_eq!(
            run.ritual_steps,
            steps(&["handoff", "handoff_exact", "review"]),
            "{name}"
        );
        let added = format!("{HANDOFF} Then summarize.");
        let (_t, t) = replaced(&dir, &transcript, &[(task, &added), (command, &submit)]);
        let run = parse(driver, &t);
        let mut expected = vec!["handoff"];
        if by_child {
            expected.push("reviewer_submit");
        }
        assert_eq!(run.ritual_steps, steps(&expected), "{name}");

        // the gate reads only the `Run`: three full runs with these steps pass it
        let (_t, t) = replaced(&dir, &transcript, &[(task, HANDOFF), (command, &review)]);
        let mut recs = vec![];
        for n in 1..=3 {
            let mut full = parse(driver, &t);
            full.ritual_steps.insert("check_green", true);
            recs.push(record(1, Arm::Full, RunRole::Task, n, full));
            recs.push(record(
                1,
                Arm::Bare,
                RunRole::Task,
                n,
                parse(driver, &transcript),
            ));
        }
        let g = grade_task(&fixture_suite(), 1, &recs, &ARMS, 3);
        assert_eq!(g.verdict, Verdict::Pass, "{name}: {:?}", g.reasons);
    }
}

#[test]
fn a_context_file_is_committed_into_every_arm_copy_with_an_agents_md() {
    let tmp = TempDir::new().unwrap();
    let mut driver = Replay::new(sample("subagent"), Status::Ok);
    driver.context = Some(("CLAUDE.md", "@AGENTS.md\n"));
    let mut p = plan(tmp.path(), 1, None);
    p.arms = vec![Arm::Bare, Arm::BlockOnly];
    p.keep_work = true;
    execute(&driver, &p).unwrap();
    let runs = run_files(&tmp.path().join("results"));
    for (run, has) in runs.iter().zip([false, true]) {
        let dir = PathBuf::from(run["workdir"].as_str().unwrap());
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(&dir)
                .output()
                .unwrap();
            String::from_utf8(out.stdout).unwrap()
        };
        assert_eq!(dir.join("CLAUDE.md").is_file(), has, "{}", dir.display());
        assert_eq!(git(&["ls-files", "CLAUDE.md"]).trim() == "CLAUDE.md", has);
        // folded into the arm's last commit: the history keeps its shape, nothing is changed
        let commits = git(&["rev-list", "--count", "HEAD"]);
        assert_eq!(commits.trim(), if has { "2" } else { "1" });
        if has {
            assert!(git(&["reflog"]).trim().is_empty());
        }
        assert_eq!(run["changed"], false);
    }
    let block_only = PathBuf::from(runs[1]["workdir"].as_str().unwrap());
    assert_eq!(
        std::fs::read_to_string(block_only.join("CLAUDE.md")).unwrap(),
        "@AGENTS.md\n"
    );

    // the adopted fixture gets it before the adoption session, and the unowned check accepts it
    let tmp = TempDir::new().unwrap();
    let err = setup_error(execute(&driver, &full_plan(tmp.path())).err().unwrap());
    assert!(
        err.message.contains("`stewards next` is not clean"),
        "{}",
        err.message
    );
    let summary = summary_json(tmp.path());
    let adopted = PathBuf::from(summary["adoption"]["workdir"].as_str().unwrap());
    assert!(adopted.join("CLAUDE.md").is_file());
    assert_eq!(
        summary["setup"]["added_paths"],
        serde_json::json!(["AGENTS.md"])
    );
}

/// A Claude Code session file of one main session from `records` (each `(cwd, content)` an
/// assistant record, then each `(tool_use_id, toolUseResult)` a user record), and its path.
fn claude_session(
    tmp: &Path,
    records: &[(&str, serde_json::Value)],
    results: &[(&str, serde_json::Value)],
) -> PathBuf {
    let mut lines: Vec<String> = records
        .iter()
        .enumerate()
        .map(|(n, (cwd, content))| {
            serde_json::json!({"type": "assistant", "isSidechain": false, "uuid": format!("a{n}"),
                "cwd": cwd, "message": {"id": format!("m{n}"), "model": "claude-haiku-5-5",
                "role": "assistant", "content": content}})
            .to_string()
        })
        .collect();
    lines.extend(results.iter().map(|(id, result)| {
        serde_json::json!({"type": "user", "isSidechain": false, "uuid": format!("r-{id}"),
            "cwd": "/r", "message": {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": id, "content": "x"}]},
            "toolUseResult": result})
        .to_string()
    }));
    let path = tmp.join("s.jsonl");
    std::fs::write(&path, lines.join("\n") + "\n").unwrap();
    path
}

fn tool_use(id: &str, name: &str, input: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"type": "tool_use", "id": id, "name": name, "input": input})
}

/// Files opened on Claude Code: `Read`'s file, `Grep`'s path, and shell reads resolved against
/// each record's own `cwd`, which a `cd` in an earlier `Bash` call moved; a `Glob` listing opens
/// nothing.
#[test]
fn claude_files_opened_count_read_and_grep_and_follow_each_records_cwd() {
    let tmp = TempDir::new().unwrap();
    let t = claude_session(
        tmp.path(),
        &[
            (
                "/r",
                serde_json::json!([
                    tool_use("1", "Read", serde_json::json!({"file_path": "/r/a.py"})),
                    tool_use(
                        "2",
                        "Grep",
                        serde_json::json!({"pattern": "x", "path": "/r/b.py"})
                    ),
                    tool_use("3", "Glob", serde_json::json!({"pattern": "c/*.ts"})),
                    tool_use("4", "Glob", serde_json::json!({"pattern": "none/*"})),
                    tool_use("5", "Bash", serde_json::json!({"command": "cd api"})),
                ]),
            ),
            (
                "/r/api",
                serde_json::json!([
                    tool_use(
                        "6",
                        "Bash",
                        serde_json::json!({"command": "cat items.py ../README.md"})
                    ),
                    tool_use("7", "Read", serde_json::json!({"file_path": "/r/api/x.py"})),
                ]),
            ),
        ],
        &[(
            "3",
            serde_json::json!({"filenames": ["/r/c/d.ts", "/r/c/e.ts"], "numFiles": 2}),
        )],
    );
    let run = claude_parse(&t, 0.0, false).unwrap();
    assert_eq!(
        run.files_opened,
        ["a.py", "b.py", "api/items.py", "README.md", "api/x.py"]
    );
    assert_eq!(run.files_opened_tree, run.files_opened);
}

/// A session with a non-empty thinking block thought; one with an empty block (as Haiku 5.5 emits
/// without a budget) or none did not.
#[test]
fn a_claude_session_with_a_thinking_block_records_thinking_on() {
    let tmp = TempDir::new().unwrap();
    let thought = serde_json::json!([{"type": "thinking", "thinking": "hmm", "signature": "s"}]);
    let t = claude_session(tmp.path(), &[("/r", thought)], &[]);
    let run = claude_parse(&t, 0.0, false).unwrap();
    assert_eq!(run.thinking_levels, ["on"]);
    let empty = serde_json::json!([{"type": "thinking", "thinking": "", "signature": ""}]);
    let t = claude_session(tmp.path(), &[("/r", empty)], &[]);
    let run = claude_parse(&t, 0.0, false).unwrap();
    assert_eq!(run.thinking_levels, ["off"]);
}

/// Every model of `modelUsage` is in `models`, so a side request on another model is flagged
/// like any other model mismatch.
#[test]
fn every_claude_model_usage_key_is_a_model_of_the_run() {
    let dir = claude_sample("plain");
    let side = r#""modelUsage":{"claude-3-5-haiku-20241022":{"inputTokens":7,"outputTokens":0,"cacheReadInputTokens":0,"cacheCreationInputTokens":0},"#;
    let (_t, t) = replaced(&dir, &main_file(&dir), &[(r#""modelUsage":{"#, side)]);
    let run = claude_parse(&t, 0.0, false).unwrap();
    assert_eq!(
        run.models,
        ["claude-3-5-haiku-20241022", "claude-haiku-5-5"]
    );
    assert_eq!(
        run.total_tokens.unwrap().input,
        run.main_tokens.unwrap().input + 7
    );
}

/// Claude Code turns thinking on from `think` in a user message, so no task prompt may hold it.
#[test]
fn a_task_prompt_containing_think_is_a_usage_error_and_no_suite_task_does() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("01-x.txt"), "Fix it. THINK first.\n").unwrap();
    let err = task_prompt(tmp.path(), 1).err().unwrap();
    let err = err.downcast_ref::<CliError>().unwrap();
    assert_eq!(err.code, "usage");
    assert!(err.message.contains("01-x.txt"), "{}", err.message);
    for name in ["fixture", "self"] {
        let suite = Suite::load(&evals(), name).unwrap();
        for t in suite.numbers() {
            task_prompt(&evals().join(&suite.tasks_dir), t).unwrap();
        }
    }
    assert!(!ADOPT_PROMPT.to_lowercase().contains("think"));
    // the runner reads every planned prompt before any session
    let run = TempDir::new().unwrap();
    let mut p = plan(run.path(), 1, None);
    let tasks = run.path().join("evals");
    copy_dir(&evals(), &tasks);
    std::fs::write(
        tasks.join(&p.suite.tasks_dir).join("01-count-endpoint.txt"),
        "think\n",
    )
    .unwrap();
    p.evals = tasks;
    let driver = Replay::new(sample("subagent"), Status::Ok);
    assert!(execute(&driver, &p).is_err());
    assert!(!run.path().join("results").exists());
}

/// Claude Code shows a session its last five commit subjects unasked, so no arm copy's history
/// names the eval, stewards, the fixture, adoption, or the block.
#[test]
fn no_arm_copys_git_log_names_the_eval() {
    let tmp = TempDir::new().unwrap();
    let mut driver = Replay::new(sample("subagent"), Status::Ok);
    driver.context = Some(("CLAUDE.md", "@AGENTS.md\n"));
    driver.adopt_stamps = &["api", "client", "cli"];
    let mut p = plan(tmp.path(), 1, None);
    p.arms = vec![Arm::Bare, Arm::BlockOnly, Arm::FullNoDevelopers];
    p.keep_work = true;
    execute(&driver, &p).unwrap();
    let summary = summary_json(tmp.path());
    let mut dirs = vec![PathBuf::from(
        summary["adoption"]["workdir"].as_str().unwrap(),
    )];
    let runs = run_files(&tmp.path().join("results"));
    dirs.extend(
        runs.iter()
            .filter(|r| r["role"] == "task")
            .map(|r| PathBuf::from(r["workdir"].as_str().unwrap())),
    );
    assert_eq!(dirs.len(), 4);
    for dir in dirs {
        let log = common::git(&dir, &["log", "--all"]).to_lowercase();
        assert!(log.contains("initial"), "{log}");
        for word in ["eval", "stewards", "fixture", "adopted", "block"] {
            assert!(!log.contains(word), "{}: {word} in {log}", dir.display());
        }
    }
}

/// A Claude Code eval reports no cost, so the spend is null, not zero; `--max-cost` with it is
/// refused before anything runs.
#[test]
fn spent_is_null_when_no_run_has_a_cost_and_max_cost_is_refused_for_claude_code() {
    let tmp = TempDir::new().unwrap();
    let p = plan(tmp.path(), 1, None);
    let mut recs = records(1, 801, 1000, all());
    for r in &mut recs {
        r.result.as_mut().unwrap().cost_usd = None;
    }
    let summary = summarize(&p, &recs, false, &[], SetupNotes::default());
    assert_eq!(summary.spent_usd, None);
    assert_eq!(
        serde_json::to_value(&summary).unwrap()["spent_usd"],
        serde_json::Value::Null
    );
    let md = render(&summary);
    assert!(md.contains(", spent null\n"), "{md}");
    let summary = summarize(
        &p,
        &records(1, 801, 1000, all()),
        false,
        &[],
        SetupNotes::default(),
    );
    assert!(render(&summary).contains(", spent $15.4030\n"));

    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let (code, v) = common::run(
        root,
        &["eval", "--harness", "claude-code", "--max-cost", "1"],
    );
    assert_eq!(
        (code, v["error"]["code"].as_str()),
        (2, Some("usage")),
        "{v}"
    );
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("cost_usd is null"),
        "{v}"
    );
}

/// The summary says, in one sentence, that two harnesses compare only at one thinking setting
/// whenever this eval's was not off throughout.
#[test]
fn the_summary_names_a_thinking_setting_another_harness_cannot_match() {
    let tmp = TempDir::new().unwrap();
    let mut p = plan(tmp.path(), 1, None);
    let mut recs = records(1, 801, 1000, all());
    for r in &mut recs {
        r.result.as_mut().unwrap().thinking_levels = vec!["off".into()];
    }
    p.thinking = "off".into();
    let summary = summarize(&p, &recs, false, &[], SetupNotes::default());
    assert_eq!(summary.thinking_comparison, None);
    recs[1].result.as_mut().unwrap().thinking_levels = vec!["on".into()];
    let summary = summarize(&p, &recs, false, &[], SetupNotes::default());
    let line = summary.thinking_comparison.clone().unwrap();
    assert!(line.contains("(requested off; recorded off, on)"), "{line}");
    assert!(render(&summary).contains(&line));
    p.thinking = "medium".into();
    let summary = summarize(&p, &recs, false, &[], SetupNotes::default());
    let line = summary.thinking_comparison.unwrap();
    assert!(
        line.ends_with(
            "so run each driver with thinking off (Pi and Hermes with `--thinking off`) before comparing them."
        ),
        "{line}"
    );
}

/// Recorded transcripts are committed: no fixture holds a provider key, as an `*API_KEY=` with a
/// value or a token after a common key prefix.
#[test]
fn no_fixture_holds_key_material() {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, out);
            } else {
                out.push(p);
            }
        }
    }
    let token = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    let mut files = vec![];
    walk(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"),
        &mut files,
    );
    assert!(files.len() > 10);
    for f in files {
        let Ok(text) = std::fs::read_to_string(&f) else {
            continue;
        };
        let name = f.display();
        for (at, _) in text.match_indices("API_KEY=") {
            let value = text[at + 8..].trim_start_matches(['"', '\'']);
            assert!(!value.starts_with(token), "{name}: an API_KEY with a value");
        }
        for prefix in [
            "sk-",
            "Bearer ",
            "ghp_",
            "github_pat_",
            "xoxb-",
            "AKIA",
            "AIza",
        ] {
            for (at, _) in text.match_indices(prefix) {
                if text[..at].ends_with(token) {
                    continue;
                }
                let rest = &text[at + prefix.len()..];
                let len = rest.find(|c: char| !token(c)).unwrap_or(rest.len());
                assert!(len < 16, "{name}: a key after `{prefix}`");
            }
        }
    }
}

fn hermes_sample(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/transcripts/hermes")
        .join(name)
}

/// The stream's `result` line of a recorded Hermes sample.
fn hermes_result(name: &str) -> serde_json::Value {
    std::fs::read_to_string(hermes_sample(name).join("stdout.log"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .find(|v| v["type"] == "result")
        .unwrap()
}

/// The trimmed real deepseek-flash run where the parent started one child with `delegate_task`
/// and the child ran `echo child`. The stream's `result.tokens` is the parent alone, its `input`
/// with the cache reads; the child's counts are its own exported session.
#[test]
fn hermes_sample_counts_child_tokens_in_total_and_not_in_main() {
    let run = hermes_parse(&main_file(&hermes_sample("subagent")), 2.0, false).unwrap();
    let main = Tokens {
        input: 13198,
        output: 95,
        cache_read: 12672,
        cache_write: 0,
    };
    assert_eq!(run.main_tokens.unwrap(), main);
    let stream = &hermes_result("subagent")["tokens"];
    assert_eq!(stream["input"], main.input + main.cache_read);
    assert_eq!(stream["output"], main.output);
    let child = Tokens {
        input: 9251,
        output: 102,
        cache_read: 9728,
        cache_write: 0,
    };
    let mut total = main;
    total.add(child);
    assert_eq!(run.total_tokens.unwrap(), total);
    assert_eq!(run.tool_calls, ["delegate_task"]);
    assert!(run.shell_commands.is_empty());
    assert_eq!(run.subagent_sessions, 1);
    assert_eq!(run.model.as_deref(), Some("deepseek/deepseek-flash"));
    assert_eq!(run.models, ["deepseek/deepseek-flash"]);
    // the parent's reasoning is a lone space, the child reasoned
    assert_eq!(run.thinking_levels, ["off", "on"]);
    assert!(run.sessions_without_thinking.is_empty());
    // Hermes's own figure, at DeepSeek's off-peak rates
    assert_eq!(run.cost_usd, None);
    let cost = run.harness_reported_cost_usd.unwrap();
    assert!((cost - (0.002074716 + 0.001478034)).abs() < 1e-12, "{cost}");
    // the larger of the parent's two prompts in agent.log, not the child's
    assert_eq!(run.peak_main_context, Some(13154));
    assert_eq!(run.ritual_steps, steps(&[]));
    assert_eq!(run.stop_reason.as_deref(), Some("stop"));
    assert!(run.failed_children.is_empty() && run.reviews.is_empty());
    assert_eq!(run.duration_s, 2.0);
}

/// The trimmed real run at Hermes's default one-shot cap of 2 children: the parent called
/// `delegate_task` three times, one task each (adding a `context` it was not asked for), and the
/// third was refused with Hermes's budget error and started nothing.
#[test]
fn hermes_refused_sample_counts_only_the_children_that_ran() {
    let dir = hermes_sample("refused");
    let transcript = main_file(&dir);
    let run = hermes_parse(&transcript, 0.0, false).unwrap();
    assert_eq!(run.subagent_sessions, 2);
    assert_eq!(run.tool_calls, ["delegate_task"; 3]);
    let main = run.main_tokens.unwrap();
    let stream = &hermes_result("refused")["tokens"];
    assert_eq!(stream["input"], main.input + main.cache_read);
    assert_eq!(
        run.total_tokens.unwrap().sum(),
        main.sum() + (171 + 1 + 9216) + (7083 + 1 + 2304)
    );
    assert_eq!(run.peak_main_context, Some(13613));
    assert!(run.failed_children.is_empty() && run.uncounted_sessions.is_empty());
    assert_eq!(run.stop_reason.as_deref(), Some("stop"));
    let text = std::fs::read_to_string(&transcript).unwrap();
    assert!(
        text.contains("Delegation budget for this one-shot run is exhausted (2/2 subagents used")
    );
    // the goal is the handoff, but the child was also given the model's context
    let (_t, t) = replaced(&dir, &transcript, &[("Reply with the word ok.", HANDOFF)]);
    assert_eq!(
        hermes_parse(&t, 0.0, false).unwrap().ritual_steps,
        steps(&["handoff"])
    );
    let context = "This is a minimal test task. No tools or investigation needed. Just output the single word: ok";
    let (_t, t) = replaced(
        &dir,
        &transcript,
        &[("Reply with the word ok.", HANDOFF), (context, "")],
    );
    assert_eq!(
        hermes_parse(&t, 0.0, false).unwrap().ritual_steps,
        steps(&["handoff", "handoff_exact"])
    );
}

/// The trimmed real run that ran `pwd` and wrote `made.txt` (relative) with `write_file`: both
/// acted in the `--in` directory.
#[test]
fn hermes_plain_sample_has_main_equal_to_total() {
    let dir = hermes_sample("plain");
    let run = hermes_parse(&main_file(&dir), 0.0, false).unwrap();
    assert_eq!(run.main_tokens, run.total_tokens);
    let stream = &hermes_result("plain")["tokens"];
    let main = run.main_tokens.unwrap();
    assert_eq!(stream["input"], main.input + main.cache_read);
    assert_eq!(run.tool_calls, ["terminal", "write_file"]);
    assert_eq!(run.shell_commands, ["pwd"]);
    assert_eq!(run.subagent_sessions, 0);
    assert!(run.files_opened.is_empty());
    assert_eq!(run.thinking_levels, ["off"]);
    assert_eq!(run.peak_main_context, Some(12890));
    let text = std::fs::read_to_string(dir.join("stdout.log")).unwrap();
    assert!(text.contains(r#"\"output\": \"/work/1/repo\""#), "{text}");
    assert!(
        text.contains(r#"\"resolved_path\": \"/work/1/repo/made.txt\""#),
        "{text}"
    );

    // without agent.log lines there is no per-response usage: the peak is unknown, not 0
    let tmp = TempDir::new().unwrap();
    copy_dir(&dir, tmp.path());
    std::fs::remove_file(tmp.path().join("agent.log")).unwrap();
    let run = hermes_parse(&main_file(tmp.path()), 0.0, false).unwrap();
    assert_eq!(run.peak_main_context, None);
}

/// A child Hermes reports as not completed is a failed child; a stream whose `result` carries a
/// nonzero `exit_code` is a main session stopped with `error`.
#[test]
fn a_hermes_child_reported_failed_and_a_failed_stream_are_failures() {
    let dir = hermes_sample("subagent");
    let (_t, t) = replaced(
        &dir,
        &main_file(&dir),
        &[(r#"\"status\": \"completed\""#, r#"\"status\": \"failed\""#)],
    );
    let run = hermes_parse(&t, 0.0, false).unwrap();
    assert_eq!(
        run.failed_children,
        ["task 0 of delegate_task call_00_ET_4wtYf4Se5710IvmxH2hc7650 ended `failed`"]
    );
    let (_t, t) = replaced(
        &dir,
        &main_file(&dir),
        &[(r#""exit_code": 0"#, r#""exit_code": 1"#)],
    );
    let run = hermes_parse(&t, 0.0, false).unwrap();
    assert_eq!(run.stop_reason.as_deref(), Some("error"));
}

/// One exported Hermes session at cwd `/r` whose assistant makes `calls` (`(id, tool, arguments)`)
/// and whose tool messages answer `(id, content)`, and its path.
fn hermes_session(
    tmp: &Path,
    calls: &[(&str, &str, serde_json::Value)],
    results: &[(&str, serde_json::Value)],
) -> PathBuf {
    hermes_export(tmp, &[hermes_row("s", None, "oneshot", calls, results)])
}

/// `rows` as an export file, and its path.
fn hermes_export(tmp: &Path, rows: &[serde_json::Value]) -> PathBuf {
    let path = tmp.join("transcript.jsonl");
    let lines: Vec<String> = rows.iter().map(|r| r.to_string() + "\n").collect();
    std::fs::write(&path, lines.concat()).unwrap();
    path
}

/// One exported Hermes row `id` (under `parent`, from `source`) at cwd `/r` with one input token,
/// whose assistant makes `calls`, then stops, and whose tool messages answer `results` (see
/// `hermes_session`). Rows without `started_at` keep their order in the file.
fn hermes_row(
    id: &str,
    parent: Option<&str>,
    source: &str,
    calls: &[(&str, &str, serde_json::Value)],
    results: &[(&str, serde_json::Value)],
) -> serde_json::Value {
    let calls: Vec<serde_json::Value> = calls
        .iter()
        .map(|(id, name, args)| {
            serde_json::json!({"id": id, "type": "function",
                "function": {"name": name, "arguments": args.to_string()}})
        })
        .collect();
    let mut messages = vec![serde_json::json!({"role": "assistant", "content": "",
        "tool_calls": calls, "finish_reason": "tool_calls", "timestamp": 1.0})];
    messages.extend(results.iter().map(|(id, content)| {
        serde_json::json!({"role": "tool", "tool_call_id": id, "content": content.to_string(),
            "timestamp": 2.0})
    }));
    messages.push(serde_json::json!({"role": "assistant", "content": "done",
        "finish_reason": "stop", "timestamp": 3.0}));
    serde_json::json!({"id": id, "parent_session_id": parent, "source": source, "cwd": "/r",
        "model": "m", "billing_provider": "p", "input_tokens": 1, "messages": messages})
}

/// Files opened on Hermes: `read_file`'s path, a content `search_files`'s path, and shell reads,
/// relative to the session's cwd; a `target: files` listing opens nothing.
#[test]
fn hermes_files_opened_count_read_and_content_search_and_shell_reads() {
    let tmp = TempDir::new().unwrap();
    let t = hermes_session(
        tmp.path(),
        &[
            ("1", "read_file", serde_json::json!({"path": "/r/a.py"})),
            (
                "2",
                "search_files",
                serde_json::json!({"pattern": "x", "path": "b.py"}),
            ),
            (
                "3",
                "search_files",
                serde_json::json!({"pattern": "*.ts", "target": "files", "path": "/r"}),
            ),
            ("4", "terminal", serde_json::json!({"command": "cat e.py"})),
            (
                "5",
                "write_file",
                serde_json::json!({"path": "f.py", "content": ""}),
            ),
        ],
        &[(
            "3",
            serde_json::json!({"total_count": 1, "files": ["/r/c/d.ts"]}),
        )],
    );
    let run = hermes_parse(&t, 0.0, false).unwrap();
    assert_eq!(run.files_opened, ["a.py", "b.py", "e.py"]);
    assert_eq!(run.files_opened_tree, run.files_opened);
    assert_eq!(run.shell_commands, ["cat e.py"]);
    assert_eq!(run.model.as_deref(), Some("p/m"));
    // a response with no reasoning at all
    assert_eq!(run.thinking_levels, ["off"]);

    // a `terminal` call's `workdir` is where its reads start; `target: find` is a listing too
    let t = hermes_session(
        tmp.path(),
        &[
            (
                "1",
                "terminal",
                serde_json::json!({"command": "cat x.py ../README.md", "workdir": "api"}),
            ),
            (
                "2",
                "search_files",
                serde_json::json!({"pattern": "*.py", "target": "find", "path": "/r/c"}),
            ),
        ],
        &[],
    );
    let run = hermes_parse(&t, 0.0, false).unwrap();
    assert_eq!(run.files_opened, ["api/x.py", "README.md"]);
}

/// Hermes continues a compressed session in a new row under the old one with the same `source`:
/// that row is the same session (its tokens, calls, and API calls are main's), and only a
/// `source: subagent` row is a child. A call the continuation kept from the row before counts once.
#[test]
fn a_hermes_compression_continuation_is_the_main_session_and_only_subagent_rows_are_children() {
    let tmp = TempDir::new().unwrap();
    let echo = ("1", "terminal", serde_json::json!({"command": "echo a"}));
    let submit = (
        "2",
        "terminal",
        serde_json::json!({"command": "stewards submit --role reviewer --file a.json"}),
    );
    let read = ("3", "read_file", serde_json::json!({"path": "/r/x.py"}));
    let out = serde_json::json!({"output": "a"});
    let t = hermes_export(
        tmp.path(),
        &[
            hermes_row(
                "m",
                None,
                "oneshot",
                std::slice::from_ref(&echo),
                &[("1", out.clone())],
            ),
            hermes_row("m2", Some("m"), "oneshot", &[echo, submit], &[("2", out)]),
            hermes_row("c", Some("m2"), "subagent", &[read], &[]),
        ],
    );
    std::fs::write(
        tmp.path().join("agent.log"),
        "x [m] a: API call #1: in=5 out=1\nx [m2] a: API call #1: in=8 out=1\nx [c] a: API call #1: in=50 out=1\n",
    )
    .unwrap();
    let run = hermes_parse(&t, 0.0, false).unwrap();
    assert_eq!(run.subagent_sessions, 1);
    assert_eq!(run.main_tokens.unwrap().input, 2);
    assert_eq!(run.total_tokens.unwrap().input, 3);
    assert_eq!(
        run.shell_commands,
        ["echo a", "stewards submit --role reviewer --file a.json"]
    );
    assert_eq!(run.tool_calls, ["terminal", "terminal"]);
    // main's own submit is not a reviewer child's
    assert_eq!(run.ritual_steps, steps(&[]));
    assert_eq!(run.peak_main_context, Some(8));
    assert_eq!(run.files_opened_tree, ["x.py"]);
    assert!(run.files_opened.is_empty() && run.uncounted_sessions.is_empty());
}

/// The ritual steps of one Hermes `delegate_task` call with `args`, answered by `result` (none:
/// the session ended inside the call).
fn hermes_dispatch(args: serde_json::Value, result: Option<serde_json::Value>) -> RitualSteps {
    let tmp = TempDir::new().unwrap();
    let results: Vec<(&str, serde_json::Value)> = result.into_iter().map(|r| ("d", r)).collect();
    let t = hermes_session(tmp.path(), &[("d", "delegate_task", args)], &results);
    hermes_parse(&t, 0.0, false).unwrap().ritual_steps
}

/// A child's task on Hermes is the whole text it was given: its goal, plus the context (and
/// output contract) Hermes adds to its system prompt. `tasks` may be a JSON string; the top-level
/// `goal` counts only without `tasks`. A launch Hermes refused, whole or per task, started no
/// child, so it is no subagent launch.
#[test]
fn hermes_delegate_task_shapes_and_refusals_map_to_what_the_children_ran() {
    let ran = serde_json::json!({"results": [{"task_index": 0, "status": "completed"}]});
    let exact = steps(&["handoff", "handoff_exact"]);
    let held = steps(&["handoff"]);
    for (args, result, expected) in [
        (
            serde_json::json!({"tasks": serde_json::json!([{"goal": HANDOFF}]).to_string()}),
            Some(ran.clone()),
            &exact,
        ),
        (
            serde_json::json!({"tasks": [{"goal": HANDOFF}], "goal": "Also this."}),
            Some(ran.clone()),
            &exact,
        ),
        (
            serde_json::json!({"tasks": [], "goal": HANDOFF}),
            Some(ran.clone()),
            &exact,
        ),
        (
            serde_json::json!({"tasks": [{"goal": HANDOFF, "context": "Be brief."}]}),
            Some(ran.clone()),
            &held,
        ),
        (
            serde_json::json!({"goal": HANDOFF, "output_schema": {"type": "object"}}),
            Some(ran.clone()),
            &held,
        ),
        // the session ended inside the call: its child was running
        (serde_json::json!({"goal": HANDOFF}), None, &exact),
        (
            serde_json::json!({"goal": HANDOFF}),
            Some(
                serde_json::json!({"error": "Delegation budget for this one-shot run is exhausted (2/2 subagents used; delegation.oneshot_max_children)."}),
            ),
            &steps(&[]),
        ),
        (
            serde_json::json!({"goal": HANDOFF}),
            Some(
                serde_json::json!({"results": [{"task_index": 0, "status": "error",
                "api_calls": 0, "route": {"refused": true}}]}),
            ),
            &steps(&[]),
        ),
    ] {
        assert_eq!(&hermes_dispatch(args.clone(), result), expected, "{args}");
    }
}

/// A second top-level session (a `hermes` a session started itself) is no parse failure: the
/// first-started one is the task's, counted alone, and the other is flagged in the summary.
#[test]
fn a_second_hermes_top_level_session_is_left_uncounted_and_flagged() {
    let tmp = TempDir::new().unwrap();
    let mut nested = hermes_row("n", None, "oneshot", &[], &[]);
    nested["started_at"] = 2.0.into();
    nested["input_tokens"] = 100.into();
    let mut main = hermes_row("m", None, "oneshot", &[], &[]);
    main["started_at"] = 1.0.into();
    let mut child = hermes_row("c", Some("m"), "subagent", &[], &[]);
    child["started_at"] = 1.5.into();
    let mut nested_child = hermes_row("nc", Some("n"), "subagent", &[], &[]);
    nested_child["started_at"] = 2.5.into();
    let t = hermes_export(tmp.path(), &[nested, main, child, nested_child]);
    let run = hermes_parse(&t, 0.0, false).unwrap();
    assert_eq!(run.subagent_sessions, 1);
    assert_eq!(run.total_tokens.unwrap().input, 2);
    assert_eq!(run.uncounted_sessions, ["n"]);
    let summary = summarize(
        &plan(tmp.path(), 1, None),
        &[record(1, Arm::Bare, RunRole::Task, 1, run)],
        false,
        &[],
        SetupNotes::default(),
    );
    assert_eq!(summary.uncounted_sessions, ["bare-t1-r1: n"]);
    assert!(render(&summary).contains("other top-level sessions, left uncounted: bare-t1-r1: n"));
}

/// A Hermes child whose own last response stopped with `length` fails the run, as on the other
/// drivers, though `delegate_task` reported it completed.
#[test]
fn a_hermes_child_stopped_by_length_is_a_failed_child() {
    let tmp = TempDir::new().unwrap();
    let mut child = hermes_row("c", Some("m"), "subagent", &[], &[]);
    let last = child["messages"].as_array().unwrap().len() - 1;
    child["messages"][last]["finish_reason"] = "length".into();
    let t = hermes_export(
        tmp.path(),
        &[hermes_row("m", None, "oneshot", &[], &[]), child],
    );
    let run = hermes_parse(&t, 0.0, false).unwrap();
    assert_eq!(run.failed_children, ["c stopped with `length`"]);
}

#[test]
fn every_missing_harness_binary_is_the_same_harness_missing_error() {
    let empty = TempDir::new().unwrap();
    let path = empty.path().as_os_str();
    let limit = Duration::from_secs(1);
    for err in [
        Pi::find(path, limit).err().unwrap(),
        ClaudeCode::find(path, limit).err().unwrap(),
        Hermes::find(path, limit).err().unwrap(),
    ] {
        let err = err.downcast_ref::<CliError>().unwrap();
        assert_eq!(err.code, "harness_missing");
    }
    let err = Hermes::find(path, limit).err().unwrap();
    assert!(
        err.downcast_ref::<CliError>()
            .unwrap()
            .fix
            .contains("Hermes Agent")
    );
}

/// A fake `hermes`: `sessions list` builds an `installs/`; `chat` records its argv and env,
/// starts a detached "daemon" named in `$HERMES_HOME/gateway.pid` (or names `$STALE_PID` there),
/// prints its `.env`, logs one API call in a rotated log and one in the current, and hangs;
/// `sessions export` writes one session to its last argument.
const FAKE_HERMES: &str = r#"#!/bin/sh
case "$1" in
sessions)
  if [ "$2" = list ]; then mkdir -p "$HERMES_HOME/installs/e"; echo x > "$HERMES_HOME/installs/e/f"; exit 0; fi
  for a; do out=$a; done
  echo '{"id":"s","parent_session_id":null,"cwd":"/r","messages":[]}' > "$out" ;;
chat)
  printf '%s\n' "$@" > ../args
  env > ../env
  setsid sleep 300 >/dev/null 2>&1 &
  echo "{\"pid\": ${STALE_PID:-$!}}" > "$HERMES_HOME/gateway.pid"
  echo $! > ../daemon
  cat "$HERMES_HOME/.env"
  mkdir -p "$HERMES_HOME/logs"
  echo 'x [s] a: API call #1: in=7 out=1' > "$HERMES_HOME/logs/agent.log.1"
  echo 'x [s] a: API call #2: in=9 out=1' > "$HERMES_HOME/logs/agent.log"
  exec sleep 30 ;;
esac
"#;

fn alive(pid: &str) -> bool {
    std::fs::read_link(format!("/proc/{pid}/cwd")).is_ok()
}

#[test]
fn hermes_driver_runs_each_session_in_its_own_home_and_kills_the_daemon() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempDir::new().unwrap();
    let bin = tmp.path().join("hermes");
    std::fs::write(&bin, FAKE_HERMES).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let user = tmp.path().join("user");
    std::fs::create_dir_all(user.join("memories")).unwrap();
    let key = "sk-test-0123456789abcdef";
    std::fs::write(
        user.join(".env"),
        format!("DEEPSEEK_API_KEY={key}\nexport GH_TOKEN='tok-0123456789'\nREGION=eu-west-123\n"),
    )
    .unwrap();
    std::fs::write(user.join("SOUL.md"), "personal").unwrap();
    std::fs::write(user.join("config.yaml"), "model: {}\n").unwrap();
    let a = tmp.path().join("a");
    let (note, installs) = template(&bin, &user, &a).unwrap();
    assert!(note.contains("subagent_auto_approve"), "{note}");
    assert!(note.contains("oneshot_max_children: 0"), "{note}");
    assert!(note.contains("not the task directory"), "{note}");
    assert_eq!(installs, fingerprint(&a.join("installs")).unwrap());
    assert_eq!(installs.len(), 2);
    assert_eq!(
        std::fs::read_to_string(a.join("config.yaml")).unwrap(),
        "delegation:\n  subagent_auto_approve: true\n  oneshot_max_children: 0\n"
    );
    assert_eq!(
        std::fs::read_link(a.join(".env")).unwrap(),
        user.join(".env")
    );
    for absent in ["SOUL.md", "memories"] {
        assert!(!a.join(absent).exists(), "{absent}");
    }

    let hermes = Hermes {
        bin,
        limit: Duration::from_secs(1),
        template: Some(a.clone()),
        installs,
    };
    std::fs::create_dir_all(tmp.path().join("run/repo")).unwrap();
    let root = tmp.path().join("run").canonicalize().unwrap();
    let dir = root.join("repo");
    let sessions = tmp.path().join("sessions");
    let env = [
        ("PATH", std::env::var_os("PATH").unwrap()),
        ("AI_AGENT", "pi".into()),
    ];
    let s = hermes
        .start(
            &dir,
            "-hi",
            Some("deepseek/deepseek-flash"),
            "off",
            &sessions,
            &env,
        )
        .unwrap();
    assert_eq!(s.status, Status::TimedOut);
    assert_eq!(s.transcript, Some(sessions.join("transcript.jsonl")));
    // every rotation of the log is kept, and the peak reads them all
    assert!(sessions.join("agent.log").is_file() && sessions.join("agent.log.1").is_file());
    let run = hermes
        .parse(&sessions.join("transcript.jsonl"), 0.0, true)
        .unwrap();
    assert_eq!(run.peak_main_context, Some(9));
    // the key and token the session printed are not kept; other values are
    let out = std::fs::read_to_string(sessions.join("stdout.log")).unwrap();
    assert!(
        !out.contains(key) && !out.contains("tok-0123456789"),
        "{out}"
    );
    assert!(
        out.contains("DEEPSEEK_API_KEY=[redacted]") && out.contains("REGION=eu-west-123"),
        "{out}"
    );
    let args = std::fs::read_to_string(root.join("args")).unwrap();
    let dir_line = format!("--in\n{}\n", dir.display());
    assert!(
        args.starts_with("chat\n--query=-hi\n--oneshot\n--yolo\n"),
        "{args}"
    );
    assert!(args.contains(&dir_line), "{args}");
    assert!(
        args.ends_with(
            "--format\nstream-json\n--reasoning\nnone\n--provider\ndeepseek\n-m\ndeepseek-flash\n"
        ),
        "{args}"
    );
    let vars = std::fs::read_to_string(root.join("env")).unwrap();
    let home = root.join("h");
    assert!(
        vars.contains(&format!("HERMES_HOME={}\n", home.display())),
        "{vars}"
    );
    assert!(!vars.contains("AI_AGENT="), "{vars}");
    // the session's home: the template's config, .env and installs linked, nothing else of the user's
    assert_eq!(
        std::fs::read_link(home.join(".env")).unwrap(),
        user.join(".env")
    );
    assert_eq!(
        std::fs::read_link(home.join("installs")).unwrap(),
        a.join("installs")
    );
    assert!(home.join("config.yaml").is_file() && !home.join("SOUL.md").exists());
    // the detached daemon is killed by the driver, not left to the sweep
    let daemon = std::fs::read_to_string(root.join("daemon")).unwrap();
    let daemon = daemon.trim();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while alive(daemon) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!alive(daemon), "daemon {daemon} survived");

    // a session wrote into the shared dependency environment: no later session starts on it
    std::fs::create_dir_all(a.join("installs/.leases")).unwrap();
    std::fs::write(a.join("installs/.leases/l"), "").unwrap();
    // a second session in the same directory (a task after its setup task) gets a new home
    std::fs::write(home.join("left.txt"), "").unwrap();
    // a gateway.pid naming a process of another home (a stale pid, reused) kills nothing
    let mut other = std::process::Command::new("sleep")
        .arg("300")
        .spawn()
        .unwrap();
    let mut stale = env.to_vec();
    stale.push(("STALE_PID", other.id().to_string().into()));
    hermes
        .start(&dir, "hi", None, "low", &sessions, &stale)
        .unwrap();
    assert!(
        other.try_wait().unwrap().is_none(),
        "an unrelated process was killed"
    );
    other.kill().unwrap();
    let daemon = std::fs::read_to_string(root.join("daemon")).unwrap();
    std::process::Command::new("kill")
        .args(["-KILL", daemon.trim()])
        .status()
        .unwrap();
    assert!(home.join("config.yaml").is_file() && !home.join("left.txt").exists());
    std::fs::write(a.join("installs/e/g"), "pip").unwrap();
    let err = hermes
        .start(&dir, "hi", None, "low", &sessions, &env)
        .err()
        .unwrap();
    assert!(
        format!("{err:#}").contains("changed after the eval built it"),
        "{err:#}"
    );
}

/// A daemon a session started elsewhere is found by the `HERMES_HOME` under the run directory its
/// environment names; a process that merely visited the run directory (`OLDPWD`) is left alone.
#[test]
fn kill_leftovers_kills_a_process_whose_hermes_home_is_the_run_directory() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("run");
    std::fs::create_dir_all(&root).unwrap();
    let mut daemon = std::process::Command::new("sleep")
        .arg("300")
        .current_dir(tmp.path())
        .env("HERMES_HOME", root.join("h"))
        .spawn()
        .unwrap();
    let mut editor = std::process::Command::new("sleep")
        .arg("300")
        .current_dir(tmp.path())
        .env("OLDPWD", root.join("repo"))
        .env("PWD", &root)
        .spawn()
        .unwrap();
    let left = kill_leftovers(&root).unwrap().unwrap();
    assert_eq!(left.killed, 1);
    assert!(left.survived.is_empty());
    assert!(!daemon.wait().unwrap().success());
    assert!(
        editor.try_wait().unwrap().is_none(),
        "the editor was killed"
    );
    editor.kill().unwrap();
}

/// `--max-cost` needs a cost figure, and Hermes's `--model` names its provider.
#[test]
fn hermes_refuses_max_cost_and_a_model_without_a_provider() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for (args, says) in [
        (
            &["eval", "--harness", "hermes", "--max-cost", "1"][..],
            "cost_usd is null",
        ),
        (
            &["eval", "--harness", "hermes"][..],
            "--model <provider>/<model>",
        ),
        (
            &["eval", "--harness", "hermes", "--model", "deepseek-flash"][..],
            "--model <provider>/<model>",
        ),
    ] {
        let (code, v) = common::run(root, args);
        assert_eq!(
            (code, v["error"]["code"].as_str()),
            (2, Some("usage")),
            "{v}"
        );
        assert!(
            v["error"]["message"].as_str().unwrap().contains(says),
            "{v}"
        );
    }
}

/// One rule on every driver: a listing (`ls`, `find`, Claude Code's `Glob`, Hermes's
/// `search_files` with `target: files`) opens no file, even when its result names files.
#[test]
fn a_listing_opens_no_file_on_any_driver() {
    let tmp = TempDir::new().unwrap();
    let listing = "ls api && find . -name '*.py'";
    let dirs: Vec<PathBuf> = ["pi", "claude", "hermes"]
        .iter()
        .map(|d| tmp.path().join(d))
        .collect();
    for d in &dirs {
        std::fs::create_dir_all(d).unwrap();
    }
    let pi_session = [
        serde_json::json!({"type": "session", "id": "h", "timestamp": "t0", "cwd": "/r"}),
        serde_json::json!({"type": "message", "id": "m", "timestamp": "t1", "message": {
            "role": "assistant", "content": [{"type": "toolCall", "id": "c", "name": "bash",
            "arguments": {"command": listing}}]}}),
    ];
    let pi_file = dirs[0].join("s.jsonl");
    std::fs::write(
        &pi_file,
        pi_session.map(|r| r.to_string()).join("\n") + "\n",
    )
    .unwrap();
    let claude_file = claude_session(
        &dirs[1],
        &[(
            "/r",
            serde_json::json!([
                tool_use("1", "Glob", serde_json::json!({"pattern": "**/*.py"})),
                tool_use("2", "Bash", serde_json::json!({"command": listing})),
            ]),
        )],
        &[(
            "1",
            serde_json::json!({"filenames": ["/r/api/a.py"], "numFiles": 1}),
        )],
    );
    let hermes_file = hermes_session(
        &dirs[2],
        &[
            (
                "1",
                "search_files",
                serde_json::json!({"pattern": "*.py", "target": "files", "path": "/r"}),
            ),
            ("2", "terminal", serde_json::json!({"command": listing})),
        ],
        &[(
            "1",
            serde_json::json!({"total_count": 1, "files": ["/r/api/a.py"]}),
        )],
    );
    for run in [
        parse(&pi_file, 0.0, false).unwrap(),
        claude_parse(&claude_file, 0.0, false).unwrap(),
        hermes_parse(&hermes_file, 0.0, false).unwrap(),
    ] {
        assert_eq!(run.shell_commands, [listing]);
        assert!(run.files_opened.is_empty(), "{:?}", run.files_opened);
        assert!(
            run.files_opened_tree.is_empty(),
            "{:?}",
            run.files_opened_tree
        );
    }
}

#[allow(dead_code)] // each test crate uses a subset of the helpers
mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use common::{edit, fixture};
use stewards::model::load_repo;
use stewards::output::CliError;
use stewards::run::{RunResult, allowed, run};

const REPO_TOML: &str = ".stewards/repo.toml";

/// The `single` fixture with `commands.test` set to `test`, and `script` written to `s.sh`.
fn repo(test: &str, script: &str) -> tempfile::TempDir {
    let dir = fixture("single");
    edit(
        &dir.path().join(REPO_TOML),
        "test = [\"pytest\"]",
        &format!("test = {test}"),
    );
    std::fs::write(dir.path().join("s.sh"), script).unwrap();
    dir
}

fn run_ok(d: &Path, unit: Option<&str>, cmd: &str) -> RunResult {
    let loaded = load_repo(d).unwrap();
    run(d, &allowed(&loaded, unit), cmd).unwrap()
}

fn refused(d: &Path, unit: Option<&str>, cmd: &str) {
    let loaded = load_repo(d).unwrap();
    let err = run(d, &allowed(&loaded, unit), cmd).unwrap_err();
    let err = err.downcast_ref::<CliError>().expect("a CliError");
    assert_eq!(err.code, "command_not_allowed", "{cmd}: {}", err.message);
}

#[test]
fn an_allowed_command_runs_and_reports_its_exit() {
    let dir = repo("[\"sh\"]", "echo hello; echo oops >&2; exit ${1:-0}\n");
    let d = dir.path();
    let ok = run_ok(d, None, "sh s.sh");
    assert!(ok.pass && !ok.timed_out, "{ok:?}");
    assert_eq!(ok.exit_code, Some(0));
    assert!(
        ok.output.contains("hello") && ok.output.contains("oops"),
        "{ok:?}"
    );
    let failed = run_ok(d, None, "sh s.sh 3");
    assert!(!failed.pass, "{failed:?}");
    assert_eq!(failed.exit_code, Some(3));
}

#[test]
fn a_command_outside_the_allowlist_or_with_shell_syntax_is_command_not_allowed() {
    let dir = repo("[\"pytest\", \"sh\"]", "");
    let d = dir.path();
    for cmd in ["pytest-x tests", "sh s.sh; true", "sh 's.sh'", "", "   "] {
        refused(d, None, cmd);
    }
}

#[test]
fn a_units_own_test_command_runs_only_for_that_unit() {
    let dir = repo("[]", "echo api\n");
    let d = dir.path();
    edit(
        &d.join(".stewards/units/api.test.toml"),
        "test_command = \"pytest tests/api\"",
        "test_command = \"sh s.sh\"",
    );
    assert!(run_ok(d, Some("api"), "sh s.sh").pass);
    refused(d, Some("store"), "sh s.sh");
    refused(d, None, "sh s.sh");
}

#[test]
fn a_timeout_kills_the_whole_process_group_and_is_a_result() {
    let dir = repo(
        "[{ cmd = \"sh\", timeout_seconds = 1 }]",
        "sleep 30 &\necho $! > bg.pid\necho started\nsleep 30\n",
    );
    let d = dir.path();
    let started = Instant::now();
    let r = run_ok(d, None, "sh s.sh");
    assert!(started.elapsed() < Duration::from_secs(10), "{r:?}");
    assert!(r.timed_out && !r.pass, "{r:?}");
    assert!(r.output.starts_with("started\n"), "{r:?}");
    assert!(
        r.output
            .ends_with("\ncommand_timeout: exceeded 1s; raise timeout_seconds or fix the test"),
        "{r:?}"
    );
    let pid: i32 = std::fs::read_to_string(d.join("bg.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    // the killed sleep may linger as a zombie until init reaps it
    let deadline = Instant::now() + Duration::from_secs(5);
    while Path::new(&format!("/proc/{pid}")).exists()
        && !std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .unwrap_or_default()
            .contains(") Z ")
    {
        assert!(Instant::now() < deadline, "background pid {pid} still runs");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn large_output_on_both_streams_keeps_its_tail_and_does_not_hang() {
    let dir = repo(
        "[\"sh\"]",
        "i=0\nwhile [ $i -lt 3000 ]; do echo \"out $i xxxxxxxxxxxxxxxxxxxxxxxx\"; echo \"err $i yyyyyyyyyyyyyyyyyyyyyyyy\" >&2; i=$((i+1)); done\necho \"$(printf 'bell\\007')\"\necho last\n",
    );
    let r = run_ok(dir.path(), None, "sh s.sh");
    assert!(r.pass, "{}", r.output.len());
    assert_eq!(r.output.chars().count(), 4_000);
    assert!(
        r.output.ends_with("bell \nlast\n"),
        "{}",
        &r.output[3_900..]
    );
}

#[test]
fn a_missing_program_is_a_failed_result() {
    let dir = repo("[\"no-such-program-stewards\"]", "");
    let r = run_ok(dir.path(), None, "no-such-program-stewards x");
    assert!(!r.pass && !r.timed_out && r.exit_code.is_none(), "{r:?}");
    assert!(r.output.contains("no-such-program-stewards"), "{r:?}");
}

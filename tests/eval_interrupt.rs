// Its own test binary: the runner's SIGINT/SIGTERM handler and flags are process-wide, so a test
// that signals the runner must not share a process with other evals.
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use stewards::eval::hermes::{Hermes, template};
use stewards::eval::{Arm, Plan, Suite, execute, neutral_work_dir};
use stewards::output::CliError;
use tempfile::TempDir;

/// A fake `hermes` whose `chat` starts a detached "daemon" for its home, sends SIGTERM to the
/// runner that started it, and hangs; `sessions export` writes one session.
const SIGNALLING_HERMES: &str = r#"#!/bin/sh
case "$1" in
sessions)
  if [ "$2" = list ]; then exit 0; fi
  for a; do out=$a; done
  echo '{"id":"s","parent_session_id":null,"cwd":"/r","messages":[]}' > "$out" ;;
chat)
  setsid sleep 300 >/dev/null 2>&1 &
  echo "{\"pid\": $!}" > "$HERMES_HOME/gateway.pid"
  echo $! > ../../../daemon
  kill -TERM $PPID
  exec sleep 60 ;;
esac
"#;

/// A SIGTERM to the runner stops the running session at once (its process group, the Hermes
/// daemon, the sweep), records it `failed` with `interrupted`, starts no other session, writes
/// the summary, and returns an `interrupted` error.
#[test]
fn a_sigterm_stops_the_session_records_it_interrupted_and_writes_the_summary() {
    let tmp = TempDir::new().unwrap();
    let bin = tmp.path().join("hermes");
    std::fs::write(&bin, SIGNALLING_HERMES).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::create_dir_all(tmp.path().join("user")).unwrap();
    let a = tmp.path().join("a");
    let (_, installs) = template(&bin, &tmp.path().join("user"), &a).unwrap();
    let driver = Hermes {
        bin,
        limit: Duration::from_secs(120),
        template: Some(a),
        installs,
    };
    let evals = Path::new(env!("CARGO_MANIFEST_DIR")).join("evals");
    let plan = Plan {
        harness: "hermes",
        tasks: vec![1],
        arms: vec![Arm::Bare],
        runs: 2,
        model: Some("p/m".into()),
        thinking: "off".into(),
        max_cost: None,
        setup_dir: None,
        suite: Suite::load(&evals, "fixture").unwrap(),
        evals,
        results: tmp.path().join("results"),
        work: neutral_work_dir(tmp.path()),
        keep_work: false,
        stewards: PathBuf::from(env!("CARGO_BIN_EXE_stewards")),
        path: std::env::var_os("PATH").unwrap(),
        harness_setup: None,
    };
    let started = Instant::now();
    let err = execute(&driver, &plan).err().unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "the session ran on"
    );
    assert_eq!(err.downcast_ref::<CliError>().unwrap().code, "interrupted");

    let runs: Vec<PathBuf> = std::fs::read_dir(tmp.path().join("results/runs"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(runs.len(), 1, "a second session started");
    let run: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&runs[0]).unwrap()).unwrap();
    assert_eq!(run["status"], "failed");
    assert_eq!(run["failure"], "interrupted");
    assert!(run["result"]["main_tokens"].is_null(), "{run}");
    let summary: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(tmp.path().join("results/summary.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(summary["sessions"], 1);
    // the daemon the session started is gone
    let daemon = std::fs::read_to_string(tmp.path().join("daemon")).unwrap_or_default();
    assert!(!daemon.trim().is_empty());
    assert!(!Path::new(&format!("/proc/{}/cwd", daemon.trim())).exists());
}

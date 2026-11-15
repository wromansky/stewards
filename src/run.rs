//! The allowlisted command runner: which commands may run, and running one.
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use anyhow::Result;
use serde::Serialize;

use crate::check::escape_invisible;
use crate::model::{CommandEntry, Loaded};
use crate::output::CliError;

/// Characters a shell gives meaning to: a command is plain argv without them, and without a
/// newline.
pub(crate) const SHELL_CHARS: &[char] = &[
    '\n', ';', '&', '|', '<', '>', '$', '`', '\\', '\'', '"', '(', ')', '{', '}', '*', '?', '~',
    '#', '!',
];

/// The characters of output a `RunResult` keeps, from the end.
const OUTPUT_CAP: usize = 4_000;
/// Bytes of output kept while reading; well over `OUTPUT_CAP` characters.
const BUFFER_CAP: usize = 64 * 1024;

/// One command an agent's command may start with, and how long it may run.
#[derive(Debug, Clone, PartialEq)]
pub struct Allowed {
    pub cmd: String,
    pub timeout_seconds: u64,
}

#[derive(Debug, Serialize)]
pub struct RunResult {
    pub command: String,
    pub exit_code: Option<i32>,
    pub pass: bool,
    pub timed_out: bool,
    pub output: String,
}

/// The commands a scenario, repro, or check of `unit` may start with: the repo's `commands.test`
/// and that unit's own test card `test_command` (no other unit's). Only a person or `init`
/// writes either: no answer has a field that sets them.
pub fn allowed(loaded: &Loaded, unit: Option<&str>) -> Vec<Allowed> {
    let commands = &loaded.repo.commands;
    let unit_test = unit
        .and_then(|u| loaded.tests.get(u))
        .and_then(|t| t.test_command.clone())
        .map(|cmd| Allowed {
            cmd,
            timeout_seconds: commands.timeout_seconds,
        });
    commands
        .test
        .iter()
        .map(|e| match e {
            CommandEntry::Plain(cmd) => Allowed {
                cmd: cmd.clone(),
                timeout_seconds: commands.timeout_seconds,
            },
            CommandEntry::Timed {
                cmd,
                timeout_seconds,
            } => Allowed {
                cmd: cmd.clone(),
                timeout_seconds: *timeout_seconds,
            },
        })
        .chain(unit_test)
        .collect()
}

/// The `allowed` entry `cmd` runs under, or why it may not run: `cmd` has no `SHELL_CHARS` and
/// its leading words are all the words of an entry, so `pytest` allows `pytest tests/x` but not
/// `pytest-x`. Of several matching entries the one with the most words wins (its timeout).
pub fn check<'a>(cmd: &str, allowed: &'a [Allowed]) -> Result<&'a Allowed, String> {
    if let Some(c) = cmd.chars().find(|c| SHELL_CHARS.contains(c)) {
        return Err(format!(
            "contains {c:?}; a command is plain argv with no shell syntax: no \
             separators, pipes, redirects, substitutions, quotes, or globs"
        ));
    }
    let words: Vec<&str> = cmd.split_whitespace().collect();
    allowed
        .iter()
        .filter_map(|e| {
            let entry: Vec<&str> = e.cmd.split_whitespace().collect();
            (!entry.is_empty() && words.starts_with(&entry)).then_some((entry.len(), e))
        })
        .max_by_key(|(n, _)| *n)
        .map(|(_, e)| e)
        .ok_or_else(|| {
            let list: Vec<&str> = allowed.iter().map(|a| a.cmd.as_str()).collect();
            format!(
                "`{cmd}` does not start with a commands.test entry or the unit's test command ({})",
                list.join(", ")
            )
        })
}

/// Runs `cmd` from `root` if `check` allows it: argv split, no shell, stdin null, env inherited,
/// in its own process group, which is killed at the entry's timeout. A timeout or a program that
/// cannot start is a failed result; only a refused command is an error (`command_not_allowed`).
pub fn run(root: &Path, allowed: &[Allowed], cmd: &str) -> Result<RunResult> {
    let entry = check(cmd, allowed).map_err(|problem| {
        CliError::new(
            "command_not_allowed",
            problem,
            "prefix with one of `commands.test` or the unit's test command, or add it to `repo.toml`",
        )
    })?;
    let argv: Vec<&str> = cmd.split_whitespace().collect();
    let mut result = RunResult {
        command: cmd.to_string(),
        exit_code: None,
        pass: false,
        timed_out: false,
        output: String::new(),
    };
    let mut child = match Command::new(argv[0])
        .args(&argv[1..])
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            result.output = clean(format!("cannot start `{}`: {e}", argv[0]).as_bytes());
            return Ok(result);
        }
    };
    // Both pipes are drained at once into one buffer, so a full pipe cannot block the command.
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let (done, finished) = mpsc::channel();
    let pipes: [Box<dyn Read + Send>; 2] = [
        Box::new(child.stdout.take().expect("piped")),
        Box::new(child.stderr.take().expect("piped")),
    ];
    for mut pipe in pipes {
        let (buffer, done) = (buffer.clone(), done.clone());
        std::thread::spawn(move || {
            let mut chunk = [0u8; 8192];
            while let Ok(n @ 1..) = pipe.read(&mut chunk) {
                let mut b = buffer.lock().unwrap();
                b.extend_from_slice(&chunk[..n]);
                if b.len() > 2 * BUFFER_CAP {
                    let cut = b.len() - BUFFER_CAP;
                    b.drain(..cut);
                }
            }
            let _ = done.send(());
        });
    }
    let limit = Duration::from_secs(entry.timeout_seconds);
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if started.elapsed() >= limit {
            // The unreaped leader keeps its pid, so the group id cannot have been reused.
            unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    // A process that left the group can hold a pipe open; wait for it only briefly.
    for _ in 0..2 {
        let _ = finished.recv_timeout(Duration::from_secs(1));
    }
    let mut bytes = std::mem::take(&mut *buffer.lock().unwrap());
    match status {
        Some(status) => {
            result.exit_code = status.code();
            result.pass = status.success();
        }
        None => {
            result.timed_out = true;
            if !bytes.is_empty() && !bytes.ends_with(b"\n") {
                bytes.push(b'\n');
            }
            bytes.extend_from_slice(
                format!(
                    "command_timeout: exceeded {}s; raise timeout_seconds or fix the test",
                    entry.timeout_seconds
                )
                .as_bytes(),
            );
        }
    }
    result.output = clean(&bytes);
    Ok(result)
}

/// The last `OUTPUT_CAP` characters of `bytes` as lossy UTF-8, each control character but
/// newline and tab replaced by a space and each invisible one escaped.
fn clean(bytes: &[u8]) -> String {
    let text: String = String::from_utf8_lossy(bytes)
        .chars()
        .map(|c| {
            if c.is_control() && c != '\n' && c != '\t' {
                ' '
            } else {
                c
            }
        })
        .collect();
    let text = escape_invisible(&text);
    let skip = text.chars().count().saturating_sub(OUTPUT_CAP);
    text.chars().skip(skip).collect()
}

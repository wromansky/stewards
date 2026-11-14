// A suite manifest, `evals/suites/<name>.toml`: what the runner copies for each arm and which
// tasks it runs.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use super::usage;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Suite {
    /// The manifest's file stem.
    #[serde(skip)]
    pub name: String,
    pub source: Source,
    /// The task prompts, `NN-*.txt`, relative to `evals/`.
    pub tasks_dir: PathBuf,
    /// Each session's wall-clock limit; a session still running then is recorded as timed out.
    pub session_minutes: u64,
    #[serde(rename = "task")]
    pub tasks: Vec<SuiteTask>,
    /// Probes of a `repo` suite: text each edit task would add, which must not be in the tree
    /// yet, else that task is already done there.
    #[serde(default, rename = "absent")]
    pub absent: Vec<Absent>,
}

/// A literal `text` that must not occur in `file` (repo-relative) of the tree at setup.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Absent {
    pub task: u32,
    pub file: String,
    pub text: String,
}

/// Where each arm's copy comes from.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    /// A directory under `evals/` that `arms.sh` copies; the `full` arm is `stewards init`, the
    /// hand-set units, and then an adoption session.
    Fixture {
        /// Relative to `evals/`.
        dir: PathBuf,
        /// The hand-set units of the `full` arm, relative to `evals/`.
        units: PathBuf,
        /// The only tracked files `units` may leave unowned.
        unowned: Vec<String>,
        /// Globs where adoption may add new files (a tester may add tests).
        tests: Vec<String>,
        /// Each hand-set unit's test command, run from the copy's root, written to its test card
        /// (`init` finds none for a unit with no manifest).
        #[serde(default)]
        test_commands: BTreeMap<String, String>,
    },
    /// The repository holding `evals/`, cloned at `commit`, already adopted: the `full` arm is
    /// that tree with the current AGENTS.md block, no adoption session. Every arm's history is
    /// one root commit.
    Repo {
        /// `HEAD`, the commit that checkout is on when the eval starts (so the sessions work on
        /// the source the binary was built from), or a literal commit for a rerun.
        commit: String,
        /// Run once per invocation in a clone at `commit`; its `target/` is copied into every
        /// run copy.
        build: Vec<String>,
    },
}

/// One task's facts.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuiteTask {
    pub number: u32,
    /// A question: `route`, `brief`, `submit`, and `check_green` are n/a.
    #[serde(default)]
    pub read_only: bool,
    /// A task run first in the same directory, as a `setup` run.
    pub setup: Option<u32>,
    /// Main-session files opened is compared with this `bare` task's.
    pub files_opened_vs_bare: Option<u32>,
}

impl Suite {
    /// `evals/suites/<name>.toml`; an unknown name is a `usage` error.
    pub fn load(evals: &Path, name: &str) -> Result<Suite> {
        let dir = evals.join("suites");
        let mut known: Vec<String> = std::fs::read_dir(&dir)
            .with_context(|| dir.display().to_string())?
            .filter_map(|e| {
                let path = e.ok()?.path();
                (path.extension()? == "toml")
                    .then(|| path.file_stem()?.to_str().map(String::from))?
            })
            .collect();
        known.sort();
        if !known.iter().any(|k| k == name) {
            return Err(usage(format!(
                "unknown suite `{name}`; valid: {}",
                known.join(", ")
            )));
        }
        let file = dir.join(format!("{name}.toml"));
        let text = std::fs::read_to_string(&file).with_context(|| file.display().to_string())?;
        let mut suite: Suite = toml::from_str(&text).with_context(|| file.display().to_string())?;
        suite.name = name.to_string();
        Ok(suite)
    }

    pub fn task(&self, number: u32) -> Option<&SuiteTask> {
        self.tasks.iter().find(|t| t.number == number)
    }

    pub fn numbers(&self) -> Vec<u32> {
        self.tasks.iter().map(|t| t.number).collect()
    }

    /// A `usage` error naming the first of `tasks` that is not in the suite.
    pub fn check_tasks(&self, tasks: &[u32]) -> Result<()> {
        match tasks.iter().find(|&&t| self.task(t).is_none()) {
            None => Ok(()),
            Some(t) => {
                let valid: Vec<String> = self.numbers().iter().map(u32::to_string).collect();
                Err(usage(format!(
                    "task {t} is not in suite `{}`; valid: {}",
                    self.name,
                    valid.join(", ")
                )))
            }
        }
    }

    /// The pinned commit of a `repo` suite.
    pub fn commit(&self) -> Option<&str> {
        match &self.source {
            Source::Repo { commit, .. } => Some(commit),
            Source::Fixture { .. } => None,
        }
    }
}

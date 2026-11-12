// User-level files under ~/.stewards: config and memory. Read-only: nothing here writes them.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::output::CliError;

/// `[models.<harness>]` tables: role name to model alias.
#[derive(Debug, Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    models: BTreeMap<String, BTreeMap<String, String>>,
}

impl Config {
    pub fn model(&self, harness: &str, role: &str) -> Option<&str> {
        self.models.get(harness)?.get(role).map(String::as_str)
    }
}

/// One `[[entry]]` of `memory.toml`; an absent filter matches everything.
#[derive(Debug, Deserialize)]
pub struct Entry {
    pub text: String,
    pub languages: Option<Vec<String>>,
    pub roles: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct MemoryFile {
    #[serde(default)]
    entry: Vec<Entry>,
}

/// `$STEWARDS_HOME`, else `~/.stewards`.
fn user_dir() -> PathBuf {
    match std::env::var_os("STEWARDS_HOME") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".stewards"),
    }
}

pub fn load_config() -> Result<Config> {
    load_config_from(&user_dir())
}

pub fn load_memory() -> Result<Vec<Entry>> {
    load_memory_from(&user_dir())
}

pub fn load_config_from(dir: &Path) -> Result<Config> {
    Ok(read_toml(&dir.join("config.toml"))?.unwrap_or_default())
}

pub fn load_memory_from(dir: &Path) -> Result<Vec<Entry>> {
    Ok(read_toml::<MemoryFile>(&dir.join("memory.toml"))?
        .map(|m| m.entry)
        .unwrap_or_default())
}

/// Entries for `role` and `languages`, in file order, cut before the first one that busts the budget.
pub fn memory_for<'a>(
    entries: &'a [Entry],
    role: &str,
    languages: &[String],
    budget_bytes: usize,
) -> Vec<&'a Entry> {
    let mut used = 0;
    entries
        .iter()
        .filter(|e| e.roles.as_ref().is_none_or(|r| r.iter().any(|x| x == role)))
        .filter(|e| {
            e.languages
                .as_ref()
                .is_none_or(|l| l.iter().any(|x| languages.contains(x)))
        })
        .take_while(|e| {
            used += e.text.len();
            used <= budget_bytes
        })
        .collect()
}

/// `Ok(None)` when the file does not exist; `invalid_toml` naming the file and line otherwise.
fn read_toml<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(anyhow::Error::new(e).context(path.display().to_string())),
    };
    toml::from_str(&text).map(Some).map_err(|e| {
        let line = e
            .span()
            .map_or(1, |s| text[..s.start].matches('\n').count() + 1);
        let at = format!("{}:{line}", path.display());
        CliError::new(
            "invalid_toml",
            format!("{at}: {}", e.message()),
            format!("fix the file at {at}"),
        )
        .into()
    })
}

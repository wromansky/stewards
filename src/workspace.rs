// Workspace discovery: stewards.toml repos or the enclosing git repo.
use std::path::{Path, PathBuf};

use crate::model::Loaded;
use crate::output::CliError;

pub struct Repo {
    pub name: String,
    pub root: PathBuf,
    /// None = the repo has no `.stewards/` yet.
    pub loaded: Option<Loaded>,
}

pub struct Workspace {
    pub repos: Vec<Repo>,
    pub is_workspace: bool,
}

/// Repos visible from `cwd`; `only` is the global `--repo <name>` filter.
pub fn discover(_cwd: &Path, _only: Option<&str>) -> anyhow::Result<Workspace> {
    anyhow::bail!(CliError::new(
        "not_implemented",
        "workspace discovery is not implemented yet",
        "use a released stewards that has this command",
    ))
}

impl Workspace {
    /// `<repo>/<unit>` in a workspace, `<unit>` alone otherwise.
    pub fn display_name(&self, repo: &Repo, unit: &str) -> String {
        if self.is_workspace {
            format!("{}/{unit}", repo.name)
        } else {
            unit.to_string()
        }
    }
}

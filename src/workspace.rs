// Workspace discovery: stewards.toml repos or the enclosing git repo.
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::model::{Loaded, load_repo};
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

#[derive(Deserialize)]
struct WorkspaceFile {
    repos: Vec<String>,
}

/// Repos visible from `cwd`; `only` is the global `--repo <name>` filter.
pub fn discover(cwd: &Path, only: Option<&str>) -> anyhow::Result<Workspace> {
    // A stewards.toml anywhere above wins over the nearer .git, so a repo inside a workspace sees its siblings.
    let (roots, is_workspace) = match cwd.ancestors().find(|d| d.join("stewards.toml").is_file()) {
        Some(dir) => (workspace_roots(&dir.join("stewards.toml"))?, true),
        None => {
            let root = cwd.ancestors().find(|d| d.join(".git").exists());
            let root = root.ok_or_else(|| {
                CliError::new(
                    "not_a_git_repo",
                    format!("no git repository at or above {}", cwd.display()),
                    "`git init` or run inside a repository",
                )
            })?;
            (vec![root.to_path_buf()], false)
        }
    };

    let mut repos = Vec::new();
    for root in roots {
        let loaded = if root.join(".stewards/repo.toml").is_file() {
            Some(load_repo(&root)?)
        } else {
            None
        };
        let name = match &loaded {
            Some(l) => l.repo.name.clone(),
            None => root
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
        };
        repos.push(Repo { name, root, loaded });
    }

    if let Some(name) = only {
        match repos.iter().position(|r| r.name == name) {
            Some(i) => repos = vec![repos.swap_remove(i)],
            None => {
                let names: Vec<_> = repos.iter().map(|r| r.name.as_str()).collect();
                anyhow::bail!(CliError::new(
                    "unknown_repo",
                    format!("no repo named `{name}`; available: {}", names.join(", ")),
                    "pass one of the listed names to --repo",
                ));
            }
        }
    }
    Ok(Workspace {
        repos,
        is_workspace,
    })
}

/// The repo roots listed in `file`, each checked to exist.
fn workspace_roots(file: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let text = std::fs::read_to_string(file)?;
    let listed: WorkspaceFile = toml::from_str(&text).map_err(|e| {
        CliError::new(
            "invalid_toml",
            format!("{}: {}", file.display(), e.message()),
            format!("fix the file at {}", file.display()),
        )
    })?;
    let base = file.parent().expect("a file has a parent");
    listed
        .repos
        .iter()
        .map(|p| {
            let root = base.join(p);
            if root.is_dir() {
                Ok(root)
            } else {
                Err(CliError::new(
                    "workspace_repo_missing",
                    format!("{} lists `{p}`, which does not exist", file.display()),
                    format!("fix `repos` in {}", file.display()),
                )
                .into())
            }
        })
        .collect()
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

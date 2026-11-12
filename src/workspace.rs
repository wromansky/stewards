// Workspace discovery: stewards.toml repos or the enclosing git repo.
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Context;
use serde::Deserialize;

use crate::model::{Loaded, load_repo};
use crate::output::{CliError, Global};

pub struct Repo {
    pub name: String,
    pub root: PathBuf,
    /// None = the repo has no `.stewards/` yet.
    pub loaded: Option<Loaded>,
}

pub struct Workspace {
    pub repos: Vec<Repo>,
    pub is_workspace: bool,
    /// `--repo` narrowed `repos` to the one named.
    pub selected: bool,
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
        selected: only.is_some(),
    })
}

/// The canonical current directory and the workspace visible from it.
pub fn here(g: &Global) -> anyhow::Result<(PathBuf, Workspace)> {
    let cwd = std::env::current_dir()?.canonicalize()?;
    let ws = discover(&cwd, g.repo.as_deref())?;
    Ok((cwd, ws))
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

impl Repo {
    /// The repo's `.stewards/` contents; `no_stewards_dir` when it has none.
    pub fn loaded(&self) -> anyhow::Result<&Loaded> {
        Ok(self.loaded.as_ref().ok_or_else(|| {
            CliError::new(
                "no_stewards_dir",
                format!("no .stewards/ in {}", self.root.display()),
                "stewards init",
            )
        })?)
    }
}

impl Workspace {
    /// The repo whose root is the deepest one containing `path`, so a nested repo owns its own
    /// files. `path` must be canonical.
    pub fn repo_containing(&self, path: &Path) -> Option<&Repo> {
        self.repos
            .iter()
            .filter_map(|r| {
                let root = r.root.canonicalize().ok()?;
                path.starts_with(&root)
                    .then(|| (root.components().count(), r))
            })
            .max_by_key(|(depth, _)| *depth)
            .map(|(_, r)| r)
    }

    /// The repo containing `cwd`, or the lone repo (which `--repo` may have picked from beside it).
    pub fn current_repo(&self, cwd: &Path) -> anyhow::Result<&Repo> {
        let lone = if self.repos.len() == 1 {
            self.repos.first()
        } else {
            None
        };
        Ok(self.repo_containing(cwd).or(lone).ok_or_else(|| {
            CliError::new(
                "not_in_repo",
                "the current directory is not inside a repo",
                "run inside a repo or pass --repo <name>",
            )
        })?)
    }

    /// Each repo with its `.stewards/`. Uninitialized repos are skipped as siblings in a
    /// workspace; a lone repo, or the one `--repo` picked, must be initialized.
    pub fn loaded_repos(&self) -> anyhow::Result<Vec<(&Repo, &Loaded)>> {
        let mut out = Vec::new();
        for repo in &self.repos {
            match &repo.loaded {
                None if self.is_workspace && !self.selected => {}
                _ => out.push((repo, repo.loaded()?)),
            }
        }
        Ok(out)
    }

    /// `<repo>/<unit>` in a workspace, `<unit>` alone otherwise.
    pub fn display_name(&self, repo: &Repo, unit: &str) -> String {
        if self.is_workspace {
            format!("{}/{unit}", repo.name)
        } else {
            unit.to_string()
        }
    }
}

/// The `unsafe_path` error; `at` is shown in the fix.
fn unsafe_path(message: String, at: &Path) -> anyhow::Error {
    CliError::new(
        "unsafe_path",
        message,
        format!(
            "remove the symlink at {}; stewards never writes through links",
            at.display()
        ),
    )
    .into()
}

/// `root/rel` once `rel` is made only of plain names and no existing component below `root`,
/// the last included, is a symlink. With `create_dirs`, missing parents are made one level at a
/// time, each checked to be a real directory. The one path check for reads and writes in a repo.
pub(crate) fn safe_path(root: &Path, rel: &str, create_dirs: bool) -> anyhow::Result<PathBuf> {
    let parts: Vec<&str> = rel.split('/').collect();
    for part in &parts {
        let mut c = Path::new(part).components();
        // split on '/' keeps `.` and empty parts that Path::components would drop
        let plain =
            matches!((c.next(), c.next()), (Some(Component::Normal(n)), None) if n == *part);
        if !plain {
            return Err(unsafe_path(
                format!("`{rel}` has component `{part}`, which is not a plain name"),
                Path::new(rel),
            ));
        }
    }
    let mut full = root.to_path_buf();
    for (i, part) in parts.iter().enumerate() {
        full.push(part);
        let is_parent = i + 1 < parts.len();
        let meta = match std::fs::symlink_metadata(&full) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && is_parent && create_dirs => {
                match std::fs::create_dir(&full) {
                    Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => {
                        return Err(
                            anyhow::Error::new(e).context(format!("create {}", full.display()))
                        );
                    }
                    _ => {}
                }
                let meta = std::fs::symlink_metadata(&full)
                    .with_context(|| format!("create {}", full.display()))?;
                if !meta.is_dir() && !meta.file_type().is_symlink() {
                    anyhow::bail!("{} is not a directory", full.display());
                }
                meta
            }
            // missing: nothing below it exists, so nothing below it is a link
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
            r => r.with_context(|| format!("inspect {}", full.display()))?,
        };
        if meta.file_type().is_symlink() {
            return Err(unsafe_path(
                format!("`{}` is a symlink", parts[..=i].join("/")),
                &full,
            ));
        }
    }
    Ok(root.join(rel))
}

impl Repo {
    /// Replaces `rel` with `contents` through a new temporary file renamed over it, so a link
    /// planted there is replaced, never followed.
    pub fn write(&self, rel: &str, contents: &[u8]) -> anyhow::Result<()> {
        let path = safe_path(&self.root, rel, true)?;
        let name = path.file_name().expect("checked plain").to_string_lossy();
        // pid + counter: unique across processes and across threads of one process
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let temp = path.with_file_name(format!(
            ".{name}.stewards-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .with_context(|| format!("create {}", temp.display()))?;
        // a replaced file keeps its mode; the rename would otherwise reset it to the default
        let keep_mode = match std::fs::symlink_metadata(&path) {
            Ok(old) => file.set_permissions(old.permissions()),
            Err(_) => Ok(()),
        };
        let done = keep_mode
            .and_then(|()| file.write_all(contents))
            .and_then(|()| std::fs::rename(&temp, &path));
        if done.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        done.with_context(|| format!("write {}", path.display()))
    }

    /// Appends `contents` to the regular file `rel`, creating it when missing.
    pub fn append(&self, rel: &str, contents: &[u8]) -> anyhow::Result<()> {
        let path = safe_path(&self.root, rel, true)?;
        let created = OpenOptions::new().append(true).create_new(true).open(&path);
        let mut file = match created {
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if !std::fs::symlink_metadata(&path)?.is_file() {
                    anyhow::bail!("{} is not a regular file", path.display());
                }
                OpenOptions::new().append(true).open(&path)
            }
            r => r,
        }
        .with_context(|| format!("open {}", path.display()))?;
        file.write_all(contents)
            .with_context(|| format!("append {}", path.display()))
    }

    /// Removes the regular file `rel`; a missing file is fine.
    pub fn remove(&self, rel: &str) -> anyhow::Result<()> {
        let path = safe_path(&self.root, rel, false)?;
        match std::fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Ok(m) if !m.is_file() => anyhow::bail!("{} is not a regular file", path.display()),
            _ => std::fs::remove_file(&path).with_context(|| format!("remove {}", path.display())),
        }
    }
}

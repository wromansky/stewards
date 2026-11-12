use std::cmp::Reverse;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

use anyhow::Result;
use globset::{GlobBuilder, GlobMatcher};
use serde::Serialize;

use crate::model::UnitDef;
use crate::output::{CliError, Global, Human, emit};

/// Longer literal prefix wins, then fewer wildcards; the derived order makes the best the max.
type Specificity = (usize, Reverse<usize>);

pub struct Router {
    /// Per unit in `units.toml` order: its name and each glob with its specificity.
    units: Vec<(String, Vec<(GlobMatcher, Specificity)>)>,
    docs_path: String,
}

fn normalize(path: &str) -> String {
    let mut p = path.replace('\\', "/");
    while let Some(rest) = p.strip_prefix("./") {
        p = rest.to_string();
    }
    p
}

fn specificity(pattern: &str) -> Specificity {
    let prefix = pattern.find(['*', '?', '[', '{']).unwrap_or(pattern.len());
    let wildcards = pattern
        .split(|c| c != '*')
        .filter(|s| !s.is_empty())
        .count();
    (prefix, Reverse(wildcards))
}

impl Router {
    pub fn new(units: &[UnitDef], docs_path: &str) -> Result<Router> {
        let mut compiled = Vec::new();
        for unit in units {
            let mut globs = Vec::new();
            // Patterns stay as written: `\` is the glob escape, not a path separator.
            for pattern in &unit.paths {
                let glob = GlobBuilder::new(pattern)
                    .literal_separator(true)
                    .build()
                    .map_err(|e| {
                        CliError::new(
                            "invalid_toml",
                            format!("unit `{}`: bad glob `{pattern}`: {e}", unit.name),
                            "fix paths in .stewards/units.toml",
                        )
                    })?;
                globs.push((glob.compile_matcher(), specificity(pattern)));
            }
            compiled.push((unit.name.clone(), globs));
        }
        Ok(Router {
            units: compiled,
            docs_path: normalize(docs_path).trim_end_matches('/').to_string(),
        })
    }

    /// True for `.stewards/**` and `docs_path/**`, which no unit owns; `path` is repo-relative
    /// with forward slashes.
    pub fn tool_owned(&self, path: &str) -> bool {
        let under =
            |dir: &str| path == dir || path.strip_prefix(dir).is_some_and(|r| r.starts_with('/'));
        under(".stewards") || under(&self.docs_path)
    }

    /// The matching units in `units.toml` order, each with its best glob's specificity.
    fn matches(&self, path: &str) -> Vec<(Specificity, &str)> {
        let path = normalize(path);
        if self.tool_owned(&path) {
            return vec![];
        }
        self.units
            .iter()
            .filter_map(|(name, globs)| {
                let best = globs
                    .iter()
                    .filter(|(m, _)| m.is_match(&path))
                    .map(|(_, s)| *s)
                    .max()?;
                Some((best, name.as_str()))
            })
            .collect()
    }

    pub fn owner(&self, path: &str) -> Option<&str> {
        let found = self.matches(path);
        let best = found.iter().map(|(s, _)| *s).max()?;
        found.iter().find(|(s, _)| *s == best).map(|(_, n)| *n)
    }

    /// Each file that two or more units match at equal top specificity, with those units.
    pub fn overlaps(&self, files: &[String]) -> Vec<(String, Vec<String>)> {
        files
            .iter()
            .filter_map(|file| {
                let found = self.matches(file);
                let best = found.iter().map(|(s, _)| *s).max()?;
                let tied: Vec<String> = found
                    .iter()
                    .filter(|(s, _)| *s == best)
                    .map(|(_, n)| n.to_string())
                    .collect();
                (tied.len() > 1).then(|| (file.clone(), tied))
            })
            .collect()
    }
}

#[derive(Serialize)]
struct Routes {
    routes: Vec<Route>,
}

#[derive(Serialize)]
struct Route {
    path: String,
    unit: Option<String>,
}

impl Human for Routes {
    fn render(&self) -> String {
        self.routes
            .iter()
            .map(|r| format!("{} -> {}", r.path, r.unit.as_deref().unwrap_or("-")))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// `path` resolved against `cwd`, with `.` and `..` folded away without touching the disk.
fn lexical_absolute(cwd: &Path, path: &str) -> PathBuf {
    let mut out = PathBuf::new();
    for c in cwd.join(path).components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            c => out.push(c),
        }
    }
    out
}

pub fn run(g: &Global, paths: &[String]) -> Result<ExitCode> {
    let (cwd, ws) = crate::workspace::here(g)?;
    // A lone repo must be initialized; in a workspace an uninitialized repo just owns nothing.
    if !ws.is_workspace
        && let Some(repo) = ws.repos.first()
    {
        repo.loaded()?;
    }
    let mut routers: HashMap<&Path, Router> = HashMap::new();
    let mut routes = Vec::new();
    for path in paths {
        // canonical like the repo roots, so a symlinked directory still lands inside its repo
        let abs = cwd
            .join(path)
            .canonicalize()
            .unwrap_or_else(|_| lexical_absolute(&cwd, path));
        let repo = ws.repo_containing(&abs).ok_or_else(|| {
            CliError::new(
                "path_outside_repo",
                if ws.is_workspace {
                    format!("{path} is outside every repo of the workspace")
                } else {
                    format!(
                        "{path} is outside the repo at {}",
                        ws.repos[0].root.display()
                    )
                },
                "pass a path inside the repo",
            )
        })?;
        let unit = match &repo.loaded {
            None => None,
            Some(loaded) => {
                let router = match routers.entry(&repo.root) {
                    Entry::Occupied(e) => e.into_mut(),
                    Entry::Vacant(e) => {
                        e.insert(Router::new(&loaded.units, &loaded.repo.docs_path)?)
                    }
                };
                let rel = abs
                    .strip_prefix(repo.root.canonicalize()?)
                    .expect("repo_containing checked starts_with")
                    .to_string_lossy()
                    .replace('\\', "/");
                router.owner(&rel).map(|unit| ws.display_name(repo, unit))
            }
        };
        routes.push(Route {
            path: path.clone(),
            unit,
        });
    }
    emit(&Routes { routes }, g.human);
    Ok(ExitCode::SUCCESS)
}

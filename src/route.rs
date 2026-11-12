use std::cmp::Reverse;
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
            for pattern in &unit.paths {
                let pattern = normalize(pattern);
                let glob = GlobBuilder::new(&pattern)
                    .literal_separator(true)
                    .build()
                    .map_err(|e| {
                        CliError::new(
                            "invalid_toml",
                            format!("unit `{}`: bad glob `{pattern}`: {e}", unit.name),
                            "fix paths in .stewards/units.toml",
                        )
                    })?;
                globs.push((glob.compile_matcher(), specificity(&pattern)));
            }
            compiled.push((unit.name.clone(), globs));
        }
        Ok(Router {
            units: compiled,
            docs_path: normalize(docs_path).trim_end_matches('/').to_string(),
        })
    }

    fn tool_owned(&self, path: &str) -> bool {
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
    let cwd = std::env::current_dir()?.canonicalize()?;
    let ws = crate::workspace::discover(&cwd, g.repo.as_deref())?;
    // With `--repo` the one repo may sit beside the current directory rather than around it.
    let repo = ws
        .repos
        .iter()
        .find(|r| {
            r.root
                .canonicalize()
                .is_ok_and(|root| cwd.starts_with(root))
        })
        .or(if ws.repos.len() == 1 {
            ws.repos.first()
        } else {
            None
        })
        .ok_or_else(|| {
            CliError::new(
                "not_in_repo",
                "the current directory is not inside a repo",
                "run inside a repo or pass --repo <name>",
            )
        })?;
    let loaded = repo.loaded.as_ref().ok_or_else(|| {
        CliError::new(
            "no_stewards_dir",
            format!("no .stewards/ in {}", repo.root.display()),
            "stewards init",
        )
    })?;
    let root = repo.root.canonicalize()?;
    let router = Router::new(&loaded.units, &loaded.repo.docs_path)?;
    let mut routes = Vec::new();
    for path in paths {
        let rel = lexical_absolute(&cwd, path)
            .strip_prefix(&root)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .map_err(|_| {
                CliError::new(
                    "path_outside_repo",
                    format!("{path} is outside the repo at {}", root.display()),
                    "pass a path inside the repo",
                )
            })?;
        routes.push(Route {
            path: path.clone(),
            unit: router.owner(&rel).map(String::from),
        });
    }
    emit(&Routes { routes }, g.human);
    Ok(ExitCode::SUCCESS)
}

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Result, bail};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};

use crate::output::CliError;
use crate::workspace::{plain_names, safe_path};

/// The newest `repo.toml` schema this build reads.
pub const SCHEMA: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoConfig {
    pub schema: u32,
    pub name: String,
    #[serde(default)]
    pub test_roots: Vec<String>,
    #[serde(default = "default_docs_path")]
    pub docs_path: String,
    /// Repo-relative instruction files other than `AGENTS.md` that a harness reads; `check`
    /// warns when one does not point at `AGENTS.md`.
    #[serde(default)]
    pub instruction_files: Vec<String>,
    /// Globs of files that may belong to no unit (root config, docs, CI): `check` does not warn
    /// on them, and `next` and `review` do not ask for them to be added to a unit.
    #[serde(default)]
    pub unowned_ok: Vec<String>,
    #[serde(default)]
    pub commands: Commands,
    #[serde(default)]
    pub budgets: Budgets,
    #[serde(default)]
    pub pack: PackConfig,
    #[serde(default, rename = "flow")]
    pub flows: Vec<Flow>,
    #[serde(default, rename = "decision")]
    pub decisions: Vec<Decision>,
    #[serde(default)]
    pub tracker: Option<Tracker>,
}

fn default_docs_path() -> String {
    "docs/units".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Commands {
    pub test: Vec<CommandEntry>,
    pub lint: Vec<CommandEntry>,
    pub timeout_seconds: u64,
}

impl Default for Commands {
    fn default() -> Self {
        Commands {
            test: vec![],
            lint: vec![],
            timeout_seconds: 600,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CommandEntry {
    Plain(String),
    Timed { cmd: String, timeout_seconds: u64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Budgets {
    pub maintainer: usize,
    pub tester: usize,
    pub documenter: usize,
    pub validator: usize,
    pub architect: usize,
    pub developer: usize,
    pub memory: usize,
}

impl Default for Budgets {
    fn default() -> Self {
        Budgets {
            maintainer: 24000,
            tester: 24000,
            documenter: 16000,
            validator: 8000,
            architect: 16000,
            developer: 24000,
            memory: 2000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PackConfig {
    pub exclude: Vec<String>,
}

impl Default for PackConfig {
    fn default() -> Self {
        let exclude = [
            ".env*",
            "*.pem",
            "*.key",
            "*secret*",
            "*credential*",
            "id_rsa*",
        ];
        PackConfig {
            exclude: exclude.map(String::from).to_vec(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Flow {
    pub name: String,
    pub units: Vec<String>,
    pub entry: String,
    pub check: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub title: String,
    pub why: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tracker {
    pub kind: TrackerKind,
    pub project: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackerKind {
    Github,
    Gitlab,
    Jira,
    Linear,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnitDef {
    #[serde(deserialize_with = "unit_name")]
    pub name: String,
    #[serde(default)]
    pub purpose: String,
    pub paths: Vec<String>,
}

/// Unit names become file names, so they must match `[a-z0-9][a-z0-9_-]*`.
fn unit_name<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<String, D::Error> {
    let name = String::deserialize(d)?;
    let mut chars = name.chars();
    let ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    if !ok {
        return Err(serde::de::Error::custom(format!(
            "unit name `{name}` must match [a-z0-9][a-z0-9_-]*"
        )));
    }
    Ok(name)
}

#[derive(Deserialize)]
struct UnitsFile {
    #[serde(default)]
    unit: Vec<UnitDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnitCard {
    pub unit: String,
    pub reviewed: Option<Reviewed>,
    // skip empty: an inline `gap = []` breaks a later hand-appended `[[gap]]`
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub interface: Vec<Interface>,
    #[serde(default, rename = "anchor", skip_serializing_if = "Vec::is_empty")]
    pub anchors: Vec<Anchor>,
    #[serde(default, rename = "gap", skip_serializing_if = "Vec::is_empty")]
    pub gaps: Vec<Gap>,
    pub doc: Option<Doc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reviewed {
    pub commit: String,
    pub fingerprint: String,
    pub tree: String,
    pub tester: Option<String>,
    pub documenter: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Interface {
    pub id: String,
    /// `path::name`; see `Anchor::parse`.
    pub anchor: String,
    pub contract: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anchor {
    pub path: String,
    pub name: String,
}

impl Anchor {
    /// `path` is the text before the first `::`, `name` the rest (Rust names contain `::`).
    pub fn parse(s: &str) -> Result<Anchor> {
        match s.split_once("::") {
            Some((path, name)) if !path.is_empty() && !name.is_empty() => Ok(Anchor {
                path: path.into(),
                name: name.into(),
            }),
            _ => bail!("anchor `{s}` is not `path::name`"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Gap {
    pub kind: GapKind,
    pub title: String,
    pub anchor: Option<String>,
    #[serde(default)]
    pub note: String,
    pub repro: Option<String>,
    pub severity: Option<String>,
    pub issue: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GapKind {
    Gap,
    Future,
    Bug,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Doc {
    pub overview: String,
    pub how_it_works: String,
    pub how_to_change: String,
}

/// An architect's plan, `.stewards/plans/<id>.toml`: open until `next` closes and deletes it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub title: String,
    pub created: String,
    #[serde(rename = "step")]
    pub steps: Vec<PlanStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    pub unit: String,
    pub change: String,
    #[serde(default)]
    pub interfaces: Vec<String>,
    pub check: String,
    /// The last result of `check`, which `next` writes; absent until it first runs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<StepResult>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StepResult {
    Pass,
    Fail,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestCard {
    pub unit: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub primary_paths: Vec<String>,
    pub test_command: Option<String>,
    #[serde(default, rename = "scenario", skip_serializing_if = "Vec::is_empty")]
    pub scenarios: Vec<Scenario>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scenario {
    pub id: String,
    pub interface: String,
    pub given: String,
    pub expect: String,
    pub level: String,
    pub command: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Developer,
    Maintainer,
    Tester,
    Documenter,
    Validator,
    Architect,
    Reviewer,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Developer => "developer",
            Role::Maintainer => "maintainer",
            Role::Tester => "tester",
            Role::Documenter => "documenter",
            Role::Validator => "validator",
            Role::Architect => "architect",
            Role::Reviewer => "reviewer",
        }
    }
}

impl FromStr for Role {
    type Err = CliError;
    fn from_str(s: &str) -> std::result::Result<Role, CliError> {
        Ok(match s {
            "developer" => Role::Developer,
            "maintainer" => Role::Maintainer,
            "tester" => Role::Tester,
            "documenter" => Role::Documenter,
            "validator" => Role::Validator,
            "architect" => Role::Architect,
            "reviewer" => Role::Reviewer,
            _ => {
                return Err(CliError::new(
                    "unknown_role",
                    format!("unknown role `{s}`"),
                    "one of developer, maintainer, tester, documenter, validator, architect, reviewer",
                ));
            }
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Loaded {
    pub root: PathBuf,
    pub repo: RepoConfig,
    pub units: Vec<UnitDef>,
    pub cards: BTreeMap<String, UnitCard>,
    pub tests: BTreeMap<String, TestCard>,
    /// The open plans, `(id, plan)` by id: the file name without `.toml`.
    pub plans: Vec<(String, Plan)>,
}

impl Loaded {
    /// The unit named `name`; `unknown_unit` lists the names that exist.
    pub fn unit(&self, name: &str) -> Result<&UnitDef> {
        Ok(self.units.iter().find(|u| u.name == name).ok_or_else(|| {
            let names: Vec<_> = self.units.iter().map(|u| u.name.as_str()).collect();
            CliError::new(
                "unknown_unit",
                format!("no unit named `{name}`; units: {}", names.join(", ")),
                "one of the listed units",
            )
        })?)
    }

    /// The unit's card, or an empty card for it when it has none yet.
    pub fn card(&self, unit: &str) -> UnitCard {
        self.cards.get(unit).cloned().unwrap_or_else(|| UnitCard {
            unit: unit.to_string(),
            reviewed: None,
            interface: vec![],
            anchors: vec![],
            gaps: vec![],
            doc: None,
        })
    }
}

/// Reads `.stewards/` under the git root `root`. A unit's card or test card may be absent.
pub fn load_repo(root: &Path) -> Result<Loaded> {
    if !root.join(".git").exists() {
        bail!(CliError::new(
            "not_a_git_repo",
            format!("{} is not a git repository", root.display()),
            "`git init` or run inside a repository",
        ));
    }
    if !root.join(".stewards/repo.toml").is_file() {
        bail!(CliError::new(
            "no_stewards_dir",
            format!("no .stewards/repo.toml in {}", root.display()),
            "stewards init",
        ));
    }

    #[derive(Deserialize)]
    struct SchemaOnly {
        schema: u32,
    }
    let repo_toml = ".stewards/repo.toml";
    // Checked before the full parse: a newer schema may not deserialize at all.
    let SchemaOnly { schema } = parse_toml(root, repo_toml)?.expect("repo.toml exists");
    if schema > SCHEMA {
        bail!(CliError::new(
            "schema_too_new",
            format!("{repo_toml} has schema {schema}; this stewards reads up to {SCHEMA}"),
            "upgrade stewards (`cargo install stewards`)",
        ));
    }
    let repo: RepoConfig = parse_toml(root, repo_toml)?.expect("repo.toml exists");
    // a listed link is fine to load: `check` reads it through `Repo::read`, which refuses links
    for file in &repo.instruction_files {
        plain_names(file)?;
    }
    // a units dir with no cards to read is still refused when it is a link
    safe_path(root, ".stewards/units", false)?;
    let units = parse_toml::<UnitsFile>(root, ".stewards/units.toml")?
        .map(|f| f.unit)
        .unwrap_or_default();
    // a lookup by name must find one unit
    let mut seen = BTreeSet::new();
    if let Some(u) = units.iter().find(|u| !seen.insert(&u.name)) {
        bail!(CliError::new(
            "invalid_toml",
            format!(".stewards/units.toml: duplicate unit name `{}`", u.name),
            format!(
                "rename or remove one `[[unit]]` named `{}` in .stewards/units.toml",
                u.name
            ),
        ));
    }

    let mut cards = BTreeMap::new();
    let mut tests = BTreeMap::new();
    for u in &units {
        if let Some(card) = parse_toml(root, &format!(".stewards/units/{}.toml", u.name))? {
            cards.insert(u.name.clone(), card);
        }
        if let Some(test) = parse_toml(root, &format!(".stewards/units/{}.test.toml", u.name))? {
            tests.insert(u.name.clone(), test);
        }
    }
    let plans = load_plans(root, &units)?;
    Ok(Loaded {
        root: root.to_path_buf(),
        repo,
        units,
        cards,
        tests,
        plans,
    })
}

pub const PLANS_DIR: &str = ".stewards/plans";

/// Every `.stewards/plans/*.toml`, by name, read through no link as cards are; a step naming a
/// unit `units` lacks is `invalid_toml`.
fn load_plans(root: &Path, units: &[UnitDef]) -> Result<Vec<(String, Plan)>> {
    let dir = match std::fs::read_dir(safe_path(root, PLANS_DIR, false)?) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(anyhow::Error::new(e).context(PLANS_DIR)),
    };
    let mut names = vec![];
    for entry in dir {
        let entry = entry?;
        // a directory or FIFO is no plan; a link is kept so `parse_toml` refuses it
        let kind = entry.file_type()?;
        let name = entry.file_name().into_string().ok();
        if let Some(id) = name.as_deref().and_then(|n| n.strip_suffix(".toml"))
            && (kind.is_file() || kind.is_symlink())
        {
            names.push(id.to_string());
        }
    }
    names.sort();
    let mut plans = vec![];
    for id in names {
        let rel = format!("{PLANS_DIR}/{id}.toml");
        // gone since the listing: closed by another `next`
        let Some(plan) = parse_toml::<Plan>(root, &rel)? else {
            continue;
        };
        if let Some(s) = plan
            .steps
            .iter()
            .find(|s| !units.iter().any(|u| u.name == s.unit))
        {
            bail!(CliError::new(
                "invalid_toml",
                format!("{rel}: step unit `{}` is not a unit in units.toml", s.unit),
                format!("fix the step's unit in {rel}, or delete the plan"),
            ));
        }
        plans.push((id, plan));
    }
    Ok(plans)
}

/// Reads `root/rel`; `Ok(None)` when it does not exist, `invalid_toml` naming `rel` and the line,
/// `unsafe_path` when it or a directory above it is a symlink.
fn parse_toml<T: DeserializeOwned>(root: &Path, rel: &str) -> Result<Option<T>> {
    let text = match std::fs::read_to_string(safe_path(root, rel, false)?) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(anyhow::Error::new(e).context(rel.to_string())),
    };
    toml::from_str(&text).map(Some).map_err(|e| {
        let line = e
            .span()
            .map_or(1, |s| text[..s.start].matches('\n').count() + 1);
        let at = format!("{rel}:{line}");
        CliError::new(
            "invalid_toml",
            format!("{at}: {}", e.message()),
            format!("fix the file at {at}"),
        )
        .into()
    })
}

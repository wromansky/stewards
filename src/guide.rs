// `guide` and the AGENTS.md block.
use std::process::ExitCode;

use anyhow::Context;
use serde::Serialize;

use crate::harness::{self, HARNESSES};
use crate::output::{Global, Human, emit};
use crate::workspace::Repo;

const START: &str = "<!-- stewards:start -->";
const END: &str = "<!-- stewards:end -->";

const BLOCK: &str = r##"<!-- stewards:start -->
## Stewards

This repo has a team of agents that already know it. Use them. This section is the project's standing instruction to delegate: starting the subagents below needs no further permission from the user.

Ritual
- Entering: run `stewards status`. It lists the units, who owns what, and what is stale. To learn how a unit works, read `docs/units/<unit>.md` before opening its source; review keeps that page true.
- Before a change: run `stewards route <files>` to see whose units you touch. A change you cannot plan from the unit pages: `stewards brief <unit> --role maintainer --task "<the change>"` (two or more units: `stewards brief --role architect --task "<the change>"`), then start a subagent with the handoff it prints.
- Changing: make the edit yourself. When a maintainer split a plan into tasks, `stewards brief <unit> --role developer` prints one handoff per task.
- Before reporting done: when your edits are finished, run `stewards review --task "<what you changed and why>"` as its own step (never alongside an edit). It prints one handoff per review to run: start one subagent per handoff, giving it exactly that sentence as its whole task; when they have finished, run `stewards review` again to read the findings. Fix them and repeat until it exits 0. Done means it exits 0.
- On request or before a release: `stewards review --roles` does the same with the deeper per-unit jobs.

A handoff is always one sentence, ``Read `<path>` and follow it.``: pass it unchanged to a general-purpose subagent that can run shell and write files. Every role works this way.

Editing
- Think first: state assumptions; ask when the request has two readings.
- Simplicity: the ladder, in order: not needed → already in the repo → stdlib → platform → installed dependency → one line → minimum code.
- Surgical: every changed line traces to the request; match the existing style.
- Goal-driven: every non-trivial change leaves one runnable check that fails if the logic breaks.
- A deliberate shortcut is a `simplified:` comment naming its ceiling and upgrade path.

Hygiene: branch off `main`; commit only when asked; lint touched files clean, no disables.

`stewards guide` prints the long form, including how to delegate in this harness. No `stewards` on PATH: install with `cargo install stewards` and follow the rules above meanwhile.
<!-- stewards:end -->"##;

/// The AGENTS.md block, markers included.
pub fn block() -> &'static str {
    BLOCK
}

/// Puts the block at the top of the repo's `AGENTS.md` (creating it), or replaces the text
/// between existing markers. Returns whether the file changed.
pub fn upsert_block(repo: &Repo) -> anyhow::Result<bool> {
    let agents_md = &repo.root.join("AGENTS.md");
    let old = match std::fs::read_to_string(agents_md) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| agents_md.display().to_string()),
    };
    let span = old
        .find(START)
        .and_then(|s| old[s..].find(END).map(|e| (s, s + e + END.len())));
    let new = match span {
        Some((s, e)) => format!("{}{BLOCK}{}", &old[..s], &old[e..]),
        None if old.is_empty() => format!("{BLOCK}\n"),
        None => format!("{BLOCK}\n\n{old}"),
    };
    if new == old {
        return Ok(false);
    }
    repo.write("AGENTS.md", new.as_bytes())?;
    Ok(true)
}

#[derive(Serialize)]
struct Delegation {
    harness: &'static str,
    text: String,
}

#[derive(Serialize)]
struct ErrorRow {
    code: &'static str,
    when: &'static str,
    fix: &'static str,
}

#[derive(Serialize)]
struct Term {
    term: &'static str,
    definition: &'static str,
}

#[derive(Serialize)]
struct Guide {
    /// The detected or named harness; null for the generic paragraph (no harness detected, or
    /// one not in the table).
    harness: Option<&'static str>,
    ritual: &'static str,
    handoff: &'static str,
    roles: &'static str,
    editing: &'static str,
    hygiene: &'static str,
    adoption: &'static str,
    delegation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    all_delegations: Option<Vec<Delegation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    errors: Option<&'static [ErrorRow]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    glossary: Option<&'static [Term]>,
}

impl Human for Guide {
    fn render(&self) -> String {
        let mut out = format!(
            "# stewards guide\n\n{}\n\n{}\n\n{}\n\n{}\n\n{}\n\n## Adoption\n\n{}\n\n## Delegation ({})\n\n{}\n",
            self.ritual,
            self.handoff,
            self.roles,
            self.editing,
            self.hygiene,
            self.adoption,
            self.harness.unwrap_or("generic"),
            self.delegation
        );
        for d in self.all_delegations.iter().flatten() {
            out += &format!("\n### {}\n\n{}\n", d.harness, d.text);
        }
        if let Some(errors) = self.errors {
            out += "\n## Error codes\n\n| code | when | fix |\n|---|---|---|\n";
            for e in errors {
                out += &format!("| `{}` | {} | {} |\n", e.code, e.when, e.fix);
            }
        }
        if let Some(glossary) = self.glossary {
            out += "\n## Glossary\n\n";
            for t in glossary {
                out += &format!("- **{}**: {}\n", t.term, t.definition);
            }
        }
        out.trim_end().to_string()
    }
}

pub fn run(
    g: &Global,
    harness: Option<&str>,
    all: bool,
    errors: bool,
    glossary: bool,
) -> anyhow::Result<ExitCode> {
    // a harness not in the table is the normal case: it gets the generic paragraph
    let chosen = match harness {
        Some(name) => harness::by_name(name).ok(),
        None => harness::detect(),
    };
    // Paragraphs of the block: marker + title, intro, ritual, handoff, editing, hygiene, footer.
    let parts: Vec<&str> = BLOCK.split("\n\n").collect();
    let guide = Guide {
        harness: chosen.map(|h| h.name),
        ritual: parts[2],
        handoff: parts[3],
        roles: ROLES,
        editing: parts[4],
        hygiene: parts[5],
        adoption: ADOPTION,
        delegation: harness::delegation(chosen),
        all_delegations: all.then(|| {
            HARNESSES
                .iter()
                .map(|h| Delegation {
                    harness: h.name,
                    text: harness::delegation(Some(h)),
                })
                .collect()
        }),
        errors: errors.then_some(&ERRORS[..]),
        glossary: glossary.then_some(&GLOSSARY[..]),
    };
    emit(&guide, g.human);
    Ok(ExitCode::SUCCESS)
}

const ROLES: &str = "Roles (the tool packs their context; you only pass the handoff)
- reviewer: reviews one change across the units it touched, runs their tests, keeps cards and docs true.
- maintainer, tester, documenter, validator, architect, developer: the per-unit specialists behind `--roles`, plans and first reviews.";

/// How a repository adopts stewards.
const ADOPTION: &str = "Run `stewards init` and correct `.stewards/units.toml`, then `stewards init --draft` (it names the units and drafts each card) and start one subagent per handoff it prints, then run `stewards review --roles` until it exits 0.";

const fn e(code: &'static str, when: &'static str, fix: &'static str) -> ErrorRow {
    ErrorRow { code, when, fix }
}

static ERRORS: [ErrorRow; 31] = [
    e(
        "not_a_git_repo",
        "no `.git` above cwd",
        "`git init` or run inside a repository",
    ),
    e(
        "no_stewards_dir",
        "no `.stewards/` above cwd",
        "`stewards init`",
    ),
    e(
        "schema_too_new",
        "`repo.toml` schema > tool's",
        "upgrade stewards (`cargo install stewards`)",
    ),
    e(
        "invalid_toml",
        "parse failure, with file and line",
        "fix the file at `<path>:<line>`",
    ),
    e(
        "unknown_unit",
        "unit name not in `units.toml`",
        "one of the listed units",
    ),
    e(
        "unknown_role",
        "role not in the seven",
        "one of developer, maintainer, tester, documenter, validator, architect, reviewer",
    ),
    e(
        "answer_parse_failed",
        "`submit` can't parse the file",
        "the field errors listed; rerun submit",
    ),
    e(
        "answer_dead_anchor",
        "a patch names an anchor that doesn't resolve",
        "correct the anchor or drop it",
    ),
    e(
        "answer_too_long",
        "field over cap",
        "shorten `<field>` to ≤ N chars",
    ),
    e(
        "command_not_allowed",
        "command outside the allowlist",
        "prefix with one of `commands.test` or add it to `repo.toml`",
    ),
    e(
        "command_timeout",
        "allowlisted command exceeded its timeout",
        "raise `timeout_seconds` or fix the test",
    ),
    e(
        "workspace_repo_missing",
        "`stewards.toml` lists a path that doesn't exist",
        "fix `repos` in `<path>`",
    ),
    e(
        "baml_runtime_unavailable",
        "`--direct` and the native lib is not found",
        "set `BAML_LIBRARY_PATH` to the library, or set `STEWARDS_BAML_DOWNLOAD=1` to let stewards download it",
    ),
    e(
        "no_api_key",
        "`--direct` and the configured provider's key is unset or blank",
        "export the variable named (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, or `GOOGLE_API_KEY`)",
    ),
    e(
        "tracker_auth",
        "`gh`/`glab` not logged in, or Jira or Linear env missing",
        "`gh auth login` / `glab auth login` / set `JIRA_URL`, `JIRA_TOKEN` / set `LINEAR_API_KEY`",
    ),
    e(
        "harness_unknown",
        "unsupported harness name",
        "one of the listed harnesses",
    ),
    e(
        "usage",
        "bad arguments, a bad `-C`, or a role given with or without a unit wrongly",
        "run `stewards <command> --help`",
    ),
    e(
        "not_in_repo",
        "the current directory is in no repo of the workspace",
        "run inside a repo or pass `--repo <name>`",
    ),
    e(
        "unknown_repo",
        "`--repo` names no repo",
        "one of the listed repos",
    ),
    e(
        "path_outside_repo",
        "`route` given a path in no repo",
        "pass a path inside the repo",
    ),
    e(
        "already_initialized",
        "`init` where `.stewards/` files exist",
        "edit `.stewards/units.toml` instead",
    ),
    e(
        "no_brief",
        "no brief, or an unusable header, for the job, or an answer to an older brief",
        "run the brief command named, or read the brief again",
    ),
    e(
        "no_direct_model",
        "`--direct` with no `[models.direct]` entry for the role, or an unsupported provider",
        "set `[models.direct] <role> = \"<provider>/<model>\"` in the user config",
    ),
    e(
        "direct_call_failed",
        "the model call failed (fixed text, never the prompt or key)",
        "rerun with `BAML_LOG=info`",
    ),
    e(
        "unsafe_path",
        "a path the tool reads or writes in a repo is not plain names, or passes through a symlink",
        "remove the symlink at `<path>`; stewards never writes through links",
    ),
    e(
        "card_changed",
        "a file `submit` was about to write changed since it was loaded (another submit finished first)",
        "run the same submit again",
    ),
    e(
        "no_tasks",
        "`brief --role developer` with no usable task list for the unit",
        "brief the maintainer again with the change as `--task`",
    ),
    e(
        "harness_missing",
        "`eval` and the harness binary (`pi`) is not on PATH, or pi-subagents is not installed in the Pi agent directory",
        "install Pi (`npm install -g @earendil-works/pi-coding-agent`) and pi-subagents (`pi install npm:pi-subagents`)",
    ),
    e(
        "eval_setup_incomplete",
        "`eval`'s adopted fixture for the `full` arm changed the fixture's code, leaves tracked files unowned, or does not reach a clean `stewards next`; or `--setup-dir` is not a git repository with nothing uncommitted",
        "read the adoption transcript under `evals/results/<timestamp>/sessions/`, give unowned files a unit in `evals/full-units.toml`, or pass `--setup-dir` a committed adopted copy where `stewards next` exits 0",
    ),
    e(
        "not_implemented",
        "a command or flag not built yet",
        "use a release that has it",
    ),
    e("internal", "any other failure", "report a bug"),
];

const fn t(term: &'static str, definition: &'static str) -> Term {
    Term { term, definition }
}

static GLOSSARY: [Term; 21] = [
    t(
        "unit",
        "a part of a repo that changes together; owns paths; has one maintainer, tester, and documenter, and one developer per task.",
    ),
    t(
        "task",
        "one piece of a maintainer's plan, done by one developer.",
    ),
    t("lane", "the files a task may write."),
    t(
        "card",
        "a unit's knowledge file, `.stewards/units/<unit>.toml`; **test card**: `.stewards/units/<unit>.test.toml`.",
    ),
    t(
        "anchor",
        "`path::name`, a place in source a card points at.",
    ),
    t(
        "interface",
        "an entry in a card naming a contract the unit promises, with an id scenarios reference.",
    ),
    t(
        "scenario",
        "a test-card entry proving one interface behavior.",
    ),
    t("gap", "a known shortfall, future item, or bug on a card."),
    t(
        "debt",
        "a `simplified:` comment in code, computed, never stored.",
    ),
    t("flow", "an ordered chain of units a request crosses."),
    t(
        "decision",
        "why a boundary or convention exists. Flows and decisions live in `repo.toml`.",
    ),
    t(
        "pack",
        "the exact context the tool computes for a unit, role, and task.",
    ),
    t(
        "brief",
        "a file holding role instructions, the pack, memory, the answer schema, and submit instructions; a subagent gets it through a handoff.",
    ),
    t(
        "submit",
        "the command that validates a typed answer and writes it.",
    ),
    t(
        "snapshot",
        "a git tree of the working tree as it is now: tracked, modified, and untracked-but-not-ignored files.",
    ),
    t("stamp", "recording that a unit was reviewed at a snapshot."),
    t(
        "fingerprint",
        "sha256 over the blobs of the files a unit owns in a snapshot; **stale**: the current fingerprint differs from the stamped one.",
    ),
    t(
        "job",
        "one task `next` decides is due: one review covering every unit a change touched, or with `--roles` one per (unit, role).",
    ),
    t(
        "plan",
        "an architect's ordered steps across units, tracked by `next`.",
    ),
    t("ritual", "entering → before change → before done."),
    t(
        "arm",
        "an eval configuration (`bare`, `block-only`, `full`).",
    ),
];

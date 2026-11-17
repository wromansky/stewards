use std::path::PathBuf;
use std::process::ExitCode;

use clap::error::ErrorKind;
use clap::{CommandFactory, Parser, Subcommand};
use stewards::model::Role;
use stewards::output::{CliError, Global};
use stewards::*;

#[derive(Parser)]
#[command(
    name = "stewards",
    version,
    about = "A team of agents that already knows your repo"
)]
struct Cli {
    /// Markdown instead of JSON
    #[arg(long, global = true)]
    human: bool,
    /// Act on one repo of a workspace
    #[arg(long, global = true)]
    repo: Option<String>,
    /// Run as if started in <dir>
    #[arg(short = 'C', global = true, value_name = "dir")]
    dir: Option<PathBuf>,
    /// Call the model directly (also STEWARDS_DIRECT=1)
    #[arg(long, global = true)]
    direct: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// The ritual, roles, editing rules, delegation text, error codes, glossary
    Guide {
        #[arg(long)]
        harness: Option<String>,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        errors: bool,
        #[arg(long)]
        glossary: bool,
    },
    /// Propose units.toml and repo.toml and write the AGENTS.md block
    Init {
        #[arg(long)]
        draft: bool,
        #[arg(long)]
        ci: bool,
    },
    /// Units, freshness, coverage, gaps
    Status {
        #[arg(long)]
        telemetry: bool,
    },
    /// The owning unit of each path
    Route {
        #[arg(required = true)]
        paths: Vec<String>,
    },
    /// The context pack (no unit for the architect)
    Pack {
        unit: Option<String>,
        #[arg(long)]
        role: String,
        #[arg(long)]
        diff: bool,
        #[arg(long)]
        task: Option<String>,
    },
    /// Write an explain brief
    Brief {
        unit: Option<String>,
        #[arg(long)]
        role: String,
        #[arg(long)]
        task: Option<String>,
        #[arg(long)]
        stdout: bool,
    },
    /// Validate a typed answer and write the update
    Submit {
        unit: Option<String>,
        #[arg(long)]
        role: String,
        #[arg(long)]
        file: PathBuf,
    },
    /// Record [reviewed] at the current snapshot (human override)
    Stamp { unit: String },
    /// Find what changed, write the review brief, print the jobs
    Next {
        #[arg(long)]
        task: Option<String>,
        #[arg(long)]
        no_brief: bool,
        /// The per-unit jobs (maintainer, tester, documenter, validator, architect) instead of one review
        #[arg(long)]
        roles: bool,
        #[arg(long)]
        no_run: bool,
        #[arg(long)]
        pr: Option<u64>,
    },
    /// Stamps, anchors, scenario ids
    Check,
    /// Record and judge bug reports; export them as issues
    Validate {
        /// A bug report: one JSON object, e.g. {"unit": "api", "title": "count off by one",
        /// "repro": "<a commands.test prefix> tests/api -k count", "evidence": "expected 2, got 3",
        /// "interface": null}; recorded on the unit's card, its repro run, a validator briefed
        #[arg(long)]
        report: Option<PathBuf>,
        #[arg(long)]
        export: bool,
    },
    /// Collect what the subagents submitted, then list the jobs `next` lists with their handoffs
    Review {
        /// What you changed and why: the reviewer checks every changed line against it
        #[arg(long)]
        task: Option<String>,
        /// The per-unit jobs (maintainer, tester, documenter, validator, architect) instead of one review
        #[arg(long)]
        roles: bool,
        #[arg(long)]
        pr: Option<u64>,
        /// A reason not to apply a finding of the last review, as `<unit>: <file>:<line>: <reason>`
        /// with the prefix review printed for it; repeatable. A reviewer accepts or contests it.
        #[arg(long)]
        answer: Vec<String>,
    },
    /// `simplified:` comments attached to units
    Debt,
    /// Render docs/units/<unit>.md and the index
    Docs { unit: Option<String> },
    /// Run the eval suite
    Eval {
        #[arg(long)]
        harness: Option<String>,
        /// The suite manifest under evals/suites/
        #[arg(long, default_value = "fixture")]
        suite: String,
        /// bare, block-only, or full; all three when omitted
        #[arg(long)]
        arm: Option<String>,
        /// Repeatable; every task of the suite when omitted
        #[arg(long)]
        task: Vec<u32>,
        #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u32).range(1..))]
        runs: u32,
        #[arg(long, value_name = "usd")]
        max_cost: Option<f64>,
        /// The full arm with the developer role switched off
        #[arg(long)]
        no_developers: bool,
        #[arg(long, value_name = "id")]
        model: Option<String>,
        /// The thinking level of every session
        #[arg(long, default_value = "medium", value_parser = ["off", "minimal", "low", "medium", "high", "xhigh", "max"])]
        thinking: String,
        /// An adopted fixture to copy for the full arm (fixture suites)
        #[arg(long, value_name = "dir")]
        setup_dir: Option<PathBuf>,
        /// Keep each run's fixture copy instead of deleting it after the run
        #[arg(long)]
        keep_work: bool,
    },
    /// Print the completion script of a shell
    Completions { shell: clap_complete::Shell },
    /// Print the man page (roff)
    Man,
}

fn main() -> ExitCode {
    // SAFETY: the first statement of main, before any thread exists.
    unsafe { stewards::llm::prepare_baml_env() };
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) if matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) => {
            e.exit()
        }
        Err(e) => {
            let text = e.render().to_string();
            CliError::new(
                "usage",
                text.trim().trim_start_matches("error: "),
                "run `stewards <command> --help`",
            )
            .emit(std::env::args().any(|a| a == "--human"));
            return ExitCode::from(2);
        }
    };
    let g = Global {
        human: cli.human,
        repo: cli.repo,
        direct: cli.direct || std::env::var("STEWARDS_DIRECT").is_ok_and(|v| v == "1"),
    };
    let result = match cli.dir {
        Some(dir) => std::env::set_current_dir(&dir).map_err(|e| {
            CliError::new(
                "usage",
                format!("-C {}: {e}", dir.display()),
                "pass an existing directory",
            )
            .into()
        }),
        None => Ok(()),
    }
    .and_then(|()| dispatch(&g, cli.command));
    match result {
        Ok(code) => code,
        Err(e) => {
            match e.downcast_ref::<CliError>() {
                Some(err) => err.emit(g.human),
                None => CliError::new("internal", format!("{e:#}"), "report a bug").emit(g.human),
            }
            ExitCode::from(2)
        }
    }
}

fn dispatch(g: &Global, command: Command) -> anyhow::Result<ExitCode> {
    match command {
        Command::Guide {
            harness,
            all,
            errors,
            glossary,
        } => guide::run(g, harness.as_deref(), all, errors, glossary),
        Command::Init { draft, ci } => init::run(g, draft, ci),
        Command::Status { telemetry } => status::run(g, telemetry),
        Command::Route { paths } => route::run(g, &paths),
        Command::Pack {
            unit,
            role,
            diff,
            task,
        } => pack::run(
            g,
            unit.as_deref(),
            role.parse::<Role>()?,
            diff,
            task.as_deref(),
        ),
        Command::Brief {
            unit,
            role,
            task,
            stdout,
        } => brief::run(
            g,
            unit.as_deref(),
            role.parse::<Role>()?,
            task.as_deref(),
            stdout,
        ),
        Command::Submit { unit, role, file } => {
            submit::run(g, unit.as_deref(), role.parse::<Role>()?, &file)
        }
        Command::Stamp { unit } => check::run_stamp(g, &unit),
        Command::Next {
            task,
            no_brief,
            roles,
            no_run,
            pr,
        } => next::run(g, task.as_deref(), no_brief, roles, no_run, pr),
        Command::Check => check::run(g),
        Command::Validate { report, export } => validate::run(g, report.as_deref(), export),
        Command::Review {
            task,
            roles,
            pr,
            answer,
        } => review::run(g, task.as_deref(), roles, pr, &answer),
        Command::Debt => debt::run(g),
        Command::Docs { unit } => docs::run(g, unit.as_deref()),
        Command::Eval {
            harness,
            suite,
            arm,
            task,
            runs,
            max_cost,
            no_developers,
            model,
            thinking,
            setup_dir,
            keep_work,
        } => eval::run(
            g,
            eval::Flags {
                harness: harness.as_deref(),
                suite: &suite,
                arm: arm.as_deref(),
                tasks: &task,
                runs,
                max_cost,
                no_developers,
                model: model.as_deref(),
                thinking: &thinking,
                setup_dir: setup_dir.as_deref(),
                keep_work,
            },
        ),
        Command::Completions { shell } => completions::run(shell, Cli::command()),
        Command::Man => completions::run_man(Cli::command()),
    }
}

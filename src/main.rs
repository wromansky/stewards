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
        hooks: bool,
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
    /// Find stale units, write review briefs, print the jobs
    Next {
        #[arg(long)]
        task: Option<String>,
        #[arg(long)]
        no_brief: bool,
        #[arg(long)]
        no_run: bool,
        #[arg(long)]
        pr: Option<u64>,
    },
    /// Stamps, anchors, scenario ids
    Check,
    /// Record and judge bug reports; export them as issues
    Validate {
        #[arg(long)]
        report: Option<PathBuf>,
        #[arg(long)]
        export: bool,
    },
    /// Maintainer review briefs on a PR or MR diff
    Review {
        #[arg(long)]
        pr: u64,
    },
    /// `simplified:` comments attached to units
    Debt,
    /// Render docs/units/<unit>.md and the index
    Docs { unit: Option<String> },
    /// Write the six native role agents
    Render {
        #[arg(long)]
        harness: String,
        #[arg(long)]
        repo_level: bool,
    },
    /// Run the eval suite
    Eval {
        #[arg(long)]
        harness: Option<String>,
        /// bare, block-only, or full; all three when omitted
        #[arg(long)]
        arm: Option<String>,
        /// Repeatable; all six when omitted
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=6))]
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
        /// An adopted fixture to copy for the full arm
        #[arg(long, value_name = "dir")]
        setup_dir: Option<PathBuf>,
        /// Keep each run's fixture copy instead of deleting it after the run
        #[arg(long)]
        keep_work: bool,
    },
    /// Shell completions
    Completions { shell: String },
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
        Command::Init { draft, hooks, ci } => init::run(g, draft, hooks, ci),
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
            no_run,
            pr,
        } => next::run(g, task.as_deref(), no_brief, no_run, pr),
        Command::Check => check::run(g),
        Command::Validate { report, export } => validate::run(g, report.as_deref(), export),
        Command::Review { pr } => review::run(g, pr),
        Command::Debt => debt::run(g),
        Command::Docs { unit } => docs::run(g, unit.as_deref()),
        Command::Render {
            harness,
            repo_level,
        } => render::run(g, &harness, repo_level),
        Command::Eval {
            harness,
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
        Command::Completions { shell } => completions::run(g, &shell, Cli::command()),
    }
}

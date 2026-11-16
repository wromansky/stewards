use std::io::Write;
use std::process::ExitCode;

use clap_complete::Shell;

/// Writes `bytes` to stdout. A closed pipe (`stewards man | head`) is dropped quietly, as `emit` does.
fn write_stdout(bytes: &[u8]) -> anyhow::Result<()> {
    let mut out = std::io::stdout().lock();
    match out.write_all(bytes).and_then(|()| out.flush()) {
        Err(e) if e.kind() != std::io::ErrorKind::BrokenPipe => Err(e.into()),
        _ => Ok(()),
    }
}

/// The completion script for `shell` on stdout, how to save it on stderr.
pub fn run(shell: Shell, mut cli: clap::Command) -> anyhow::Result<ExitCode> {
    let mut script = Vec::new();
    clap_complete::generate(shell, &mut cli, "stewards", &mut script);
    write_stdout(&script)?;
    let (name, dest) = match shell {
        Shell::Bash => (
            "bash",
            "~/.local/share/bash-completion/completions/stewards",
        ),
        Shell::Zsh => ("zsh", "~/.zfunc/_stewards"),
        Shell::Fish => ("fish", "~/.config/fish/completions/stewards.fish"),
        Shell::Elvish => ("elvish", "~/.config/elvish/lib/stewards.elv"),
        _ => ("powershell", "stewards.ps1"),
    };
    let after = match shell {
        Shell::Zsh => " (with `fpath+=~/.zfunc` before `compinit` in ~/.zshrc)",
        Shell::Elvish => " (then `use stewards` in rc.elv)",
        Shell::PowerShell => " (then `. stewards.ps1` in your profile)",
        _ => "",
    };
    eprintln!("save it: stewards completions {name} > {dest}{after}");
    Ok(ExitCode::SUCCESS)
}

/// The roff man page on stdout, how to save it on stderr.
pub fn run_man(cli: clap::Command) -> anyhow::Result<ExitCode> {
    let mut page = Vec::new();
    clap_mangen::Man::new(cli).render(&mut page)?;
    write_stdout(&page)?;
    eprintln!("save it: stewards man > ~/.local/share/man/man1/stewards.1");
    Ok(ExitCode::SUCCESS)
}

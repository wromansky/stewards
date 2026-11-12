use std::process::ExitCode;

use crate::output::{Global, not_implemented};

pub fn run(_g: &Global, _shell: &str, _cli: clap::Command) -> anyhow::Result<ExitCode> {
    not_implemented("completions")
}

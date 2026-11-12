use std::process::ExitCode;

use crate::output::{Global, not_implemented};

pub fn run(_g: &Global, _draft: bool, _hooks: bool, _ci: bool) -> anyhow::Result<ExitCode> {
    not_implemented("init")
}

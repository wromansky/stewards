use std::process::ExitCode;

use crate::output::{Global, not_implemented};

pub fn run(_g: &Global, _pr: u64) -> anyhow::Result<ExitCode> {
    not_implemented("review")
}

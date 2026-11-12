use std::process::ExitCode;

use crate::output::{Global, not_implemented};

pub fn run(
    _g: &Global,
    _task: Option<&str>,
    _no_brief: bool,
    _no_run: bool,
    _pr: Option<u64>,
) -> anyhow::Result<ExitCode> {
    not_implemented("next")
}

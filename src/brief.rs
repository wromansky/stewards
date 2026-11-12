use std::process::ExitCode;

use crate::model::Role;
use crate::output::{Global, not_implemented};

pub fn run(
    _g: &Global,
    _unit: Option<&str>,
    _role: Role,
    _task: Option<&str>,
    _stdout: bool,
) -> anyhow::Result<ExitCode> {
    not_implemented("brief")
}

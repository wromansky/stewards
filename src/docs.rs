use std::process::ExitCode;

use crate::output::{Global, not_implemented};

pub fn run(_g: &Global, _unit: Option<&str>) -> anyhow::Result<ExitCode> {
    not_implemented("docs")
}

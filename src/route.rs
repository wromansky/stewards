use std::process::ExitCode;

use crate::output::{Global, not_implemented};

pub fn run(_g: &Global, _paths: &[String]) -> anyhow::Result<ExitCode> {
    not_implemented("route")
}

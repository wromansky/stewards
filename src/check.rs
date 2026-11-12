use std::process::ExitCode;

use crate::output::{Global, not_implemented};

pub fn run(_g: &Global) -> anyhow::Result<ExitCode> {
    not_implemented("check")
}

pub fn run_stamp(_g: &Global, _unit: &str) -> anyhow::Result<ExitCode> {
    not_implemented("stamp")
}

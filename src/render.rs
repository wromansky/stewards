use std::process::ExitCode;

use crate::output::{Global, not_implemented};

pub fn run(_g: &Global, _harness: &str, _repo_level: bool) -> anyhow::Result<ExitCode> {
    not_implemented("render")
}

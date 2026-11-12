use std::path::Path;
use std::process::ExitCode;

use crate::output::{Global, not_implemented};

pub fn run(_g: &Global, _report: Option<&Path>, _export: bool) -> anyhow::Result<ExitCode> {
    not_implemented("validate")
}

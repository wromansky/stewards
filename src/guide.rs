use std::process::ExitCode;

use crate::output::{Global, not_implemented};

pub fn run(
    _g: &Global,
    _harness: Option<&str>,
    _all: bool,
    _errors: bool,
    _glossary: bool,
) -> anyhow::Result<ExitCode> {
    not_implemented("guide")
}

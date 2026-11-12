use std::process::ExitCode;

use crate::output::{Global, not_implemented};

pub fn run(
    _g: &Global,
    _harness: Option<&str>,
    _arm: Option<&str>,
    _task: Option<u32>,
    _max_cost: Option<f64>,
) -> anyhow::Result<ExitCode> {
    not_implemented("eval")
}

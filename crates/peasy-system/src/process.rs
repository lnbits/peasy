//! Native command execution shared with cancellable desktop queries.
use std::{
    process::{Command, Output},
    time::Duration,
};

pub fn run(mut command: Command, timeout: Duration) -> anyhow::Result<Output> {
    peasy_core::process::run(&mut command, timeout)
}

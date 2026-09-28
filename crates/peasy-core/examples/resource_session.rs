//! VM-only probe: invokes the real closed adapters without an AI provider.
//! Not installed in Peasy packages. Never run storage changes on a host disk.
use anyhow::{Result, bail};
use peasy_core::{
    resource_native::{self, SessionRunner},
    *,
};
use serde::Deserialize;
use std::io::Read;
#[derive(Deserialize)]
#[serde(tag = "test", rename_all = "snake_case", deny_unknown_fields)]
enum Probe {
    Inspect { query: ResourceQuery },
    Apply { change: ResourceChange },
    Snapshot { change: ResourceChange },
    Render { changes: Vec<ResourceChange> },
    Display { change: ResourceChange, keep: bool },
}
fn main() -> Result<()> {
    let mut input = String::new();
    std::io::stdin().take(65537).read_to_string(&mut input)?;
    if input.len() > 65536 {
        bail!("request too large");
    }
    match serde_json::from_str(&input)? {
        Probe::Inspect { query } => {
            println!("{}", resource_native::inspect(&query, &SessionRunner)?)
        }
        Probe::Apply { change } => {
            let before = resource_native::snapshot(&change, &SessionRunner)?;
            resource_native::apply_live(&change, &before, &SessionRunner)?;
        }
        Probe::Snapshot { change } => {
            println!("{}", resource_native::snapshot(&change, &SessionRunner)?)
        }
        Probe::Render { changes } => {
            let mut state = PackageState::default();
            for change in changes {
                let caller = if matches!(change, ResourceChange::UserCreate { .. }) {
                    ("guest", 1001)
                } else {
                    ("peasytest", 1000)
                };
                state.resources = state.resources.changed(&change, Some(caller))?;
            }
            println!("{}", render_packages_module(&state)?);
        }
        Probe::Display { change, keep } => {
            let before = resource_native::snapshot(&change, &SessionRunner)?;
            let trial = display_trial::DisplayTrial::start(&change, &before)?;
            if !keep {
                std::thread::sleep(std::time::Duration::from_secs(21));
            }
            println!("{}", trial.finish(keep)?);
        }
    }
    Ok(())
}

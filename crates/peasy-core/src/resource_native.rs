//! Fixed native adapters. Model values cannot select an executable or arguments.
use crate::resources::*;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{io::Read, path::PathBuf, process::Command, time::Duration};
#[cfg(test)]
#[path = "../../../peapod/tests/resource-native.rs"]
mod tests;

#[path = "../../../peapod/audio/native.rs"]
mod audio;
#[path = "../../../peapod/diagnostics/native.rs"]
mod diagnostics;
#[path = "../../../peapod/displays/native.rs"]
mod displays;
#[path = "../../../peapod/firewall/native.rs"]
mod firewall;
#[path = "../../../peapod/nix_maintenance/native.rs"]
mod nix_maintenance;
#[path = "../../../peapod/power/native.rs"]
mod power;
#[path = "../../../peapod/printing/native.rs"]
mod printing;
#[path = "../../../peapod/services/native.rs"]
mod services;
#[path = "../../../peapod/storage/native.rs"]
mod storage;
#[path = "../../../peapod/users/native.rs"]
mod users;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tool {
    Systemctl,
    Lsblk,
    Udisksctl,
    Busctl,
    Nix,
    NixEnv,
    Df,
    Ip,
    Wpctl,
    PwDump,
    Powerprofilesctl,
    Lpstat,
    Lpinfo,
    Ippfind,
    Lpadmin,
    Lpoptions,
    Lp,
    Hyprctl,
    KscreenDoctor,
    Gdctl,
    Upower,
}
impl Tool {
    pub fn name(self) -> &'static str {
        match self {
            Self::Systemctl => "systemctl",
            Self::Lsblk => "lsblk",
            Self::Udisksctl => "udisksctl",
            Self::Busctl => "busctl",
            Self::Nix => "nix",
            Self::NixEnv => "nix-env",
            Self::Df => "df",
            Self::Ip => "ip",
            Self::Wpctl => "wpctl",
            Self::PwDump => "pw-dump",
            Self::Powerprofilesctl => "powerprofilesctl",
            Self::Lpstat => "lpstat",
            Self::Lpinfo => "lpinfo",
            Self::Ippfind => "ippfind",
            Self::Lpadmin => "lpadmin",
            Self::Lpoptions => "lpoptions",
            Self::Lp => "lp",
            Self::Hyprctl => "hyprctl",
            Self::KscreenDoctor => "kscreen-doctor",
            Self::Gdctl => "gdctl",
            Self::Upower => "upower",
        }
    }
    pub fn path(self) -> Result<PathBuf> {
        let key = format!(
            "PEASY_{}",
            self.name().to_ascii_uppercase().replace('-', "_")
        );
        let path = std::env::var_os(key).map(PathBuf::from).unwrap_or_else(|| {
            PathBuf::from(format!("/run/current-system/sw/bin/{}", self.name()))
        });
        if !path.is_absolute() {
            bail!("trusted tool path must be absolute");
        }
        Ok(path)
    }
}
pub trait ResourceRunner {
    fn run(&self, tool: Tool, args: &[&str]) -> Result<String>;
}
pub struct SessionRunner;
impl ResourceRunner for SessionRunner {
    fn run(&self, tool: Tool, args: &[&str]) -> Result<String> {
        let mut cmd = Command::new(tool.path()?);
        cmd.args(args).env("LC_ALL", "C");
        let output = crate::process::run(&mut cmd, Duration::from_secs(120))?;
        if !output.status.success() {
            if tool == Tool::Lpstat
                && args == ["-v"]
                && String::from_utf8_lossy(&output.stderr).trim()
                    == "lpstat: No destinations added."
            {
                return Ok(String::new());
            }
            bail!(
                "{} failed ({}). Check that the desktop service is available and you are authorized.",
                tool.name(),
                output.status
            );
        }
        bounded_output(&output.stdout)
    }
}
pub fn bounded_output(bytes: &[u8]) -> Result<String> {
    if bytes.len() > 256 * 1024 {
        bail!("resource response exceeds its size limit");
    }
    Ok(String::from_utf8_lossy(bytes)
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect())
}
pub fn read_bounded(path: &str) -> Result<String> {
    let mut data = Vec::new();
    std::fs::File::open(path)?
        .take(64 * 1024 + 1)
        .read_to_end(&mut data)?;
    if data.len() > 64 * 1024 {
        bail!("resource file exceeds its size limit");
    }
    bounded_output(&data)
}

pub fn review_identity(change: &ResourceChange, snapshot: &Value) -> String {
    match change {
        ResourceChange::Disk { device, .. } => format!(
            "Selected filesystem: {device}; UUID {}; {} bytes; device serial {}.",
            snapshot["uuid"].as_str().unwrap_or("none"),
            snapshot["bytes"],
            snapshot["serial"].as_str().unwrap_or("unavailable")
        ),
        ResourceChange::Service { unit, .. } => format!(
            "Service {unit} is currently {} / {}.",
            snapshot["ActiveState"].as_str().unwrap_or("unknown"),
            snapshot["SubState"].as_str().unwrap_or("unknown")
        ),
        ResourceChange::Audio { .. } => format!(
            "Selected audio device: {} ({})",
            snapshot["description"]
                .as_str()
                .or(snapshot["name"].as_str())
                .unwrap_or("unnamed"),
            snapshot["class"].as_str().unwrap_or("unknown")
        ),
        ResourceChange::Printer { .. } => format!(
            "Printer destination: {}",
            snapshot["uri"].as_str().unwrap_or("unknown")
        ),
        ResourceChange::Display { .. } => format!(
            "Current desktop: {}",
            snapshot["backend"].as_str().unwrap_or("unknown")
        ),
        _ => "The host will check the reviewed resources again before applying.".into(),
    }
}
pub fn json_run(r: &dyn ResourceRunner, t: Tool, a: &[&str]) -> Result<Value> {
    serde_json::from_str(&r.run(t, a)?).context("invalid resource JSON")
}
pub fn inspect(query: &ResourceQuery, r: &dyn ResourceRunner) -> Result<Value> {
    query.validate()?;
    let value = match query.domain {
        ResourceDomain::Diagnostics => diagnostics::inspect(r)?,
        ResourceDomain::Services => services::inspect(query.target.as_deref(), r)?,
        ResourceDomain::Storage => storage::inspect(r)?,
        ResourceDomain::NixMaintenance => nix_maintenance::inspect(r)?,
        ResourceDomain::Users => users::inspect()?,
        ResourceDomain::Firewall => firewall::inspect(r)?,
        ResourceDomain::Audio => audio::inspect(r)?,
        ResourceDomain::Power => power::inspect(r)?,
        ResourceDomain::Printing => printing::inspect(r)?,
        ResourceDomain::Displays => displays::inspect(r)?,
    };
    if serde_json::to_vec(&value)?.len() > 48 * 1024 {
        bail!("too many resources; request a narrower inspection");
    }
    Ok(value)
}
/// Re-read the selected resource and return only identity/precondition fields.
/// Snapshot data never comes from the model and is compared again at apply.
pub fn snapshot(change: &ResourceChange, r: &dyn ResourceRunner) -> Result<Value> {
    change.validate()?;
    match change {
        ResourceChange::Service { unit, .. } => services::snapshot(unit, r),
        ResourceChange::Disk { .. } | ResourceChange::PersistentMount { .. } => {
            storage::snapshot(change, r)
        }
        ResourceChange::NixDeleteGenerations { generations } => {
            nix_maintenance::snapshot(generations, r)
        }
        ResourceChange::Audio { .. } => audio::snapshot(change, r),
        ResourceChange::Printer { .. } => printing::snapshot(change, r),
        ResourceChange::Display { .. } => displays::snapshot(change, r),
        ResourceChange::PowerProfile { profile } => power::snapshot(*profile, r),
        _ => Ok(json!({})),
    }
}
pub fn apply_live(change: &ResourceChange, snapshot: &Value, r: &dyn ResourceRunner) -> Result<()> {
    change.validate()?;
    if change.persistent() {
        bail!("persistent operations require a NixOS transaction");
    }
    if &self::snapshot(change, r)? != snapshot {
        bail!("selected resources changed; review again");
    }
    crate::cancellation::Cancellation::current().protect()?;
    match change {
        ResourceChange::Service { unit, action } => {
            r.run(Tool::Systemctl, &[action.value(), "--", unit])?;
        }
        ResourceChange::Disk { .. } => storage::apply(change, r)?,
        ResourceChange::NixOptimise {} => {
            r.run(Tool::Nix, &["store", "optimise"])?;
        }
        ResourceChange::NixGarbageCollect {} => {
            r.run(Tool::Nix, &["store", "gc"])?;
        }
        ResourceChange::NixDeleteGenerations { generations } => {
            nix_maintenance::remove(generations, r)?
        }
        ResourceChange::Audio { .. } => audio::apply(change, r)?,
        ResourceChange::PowerProfile { profile } => {
            r.run(Tool::Powerprofilesctl, &["set", profile.value()])?;
        }
        ResourceChange::Printer { .. } => printing::apply(change, r)?,
        ResourceChange::Display { .. } => {
            bail!("display changes require the confirmation watchdog")
        }
        _ => bail!("unsupported live resource change"),
    }
    Ok(())
}

pub(crate) fn prepare_display(
    change: &ResourceChange,
    before: &Value,
    r: &dyn ResourceRunner,
) -> Result<()> {
    change.validate()?;
    if !matches!(change, ResourceChange::Display { .. }) {
        bail!("not a display change");
    }
    if &displays::snapshot(change, r)? != before {
        bail!("selected resources changed; review again");
    }
    Ok(())
}
pub(crate) fn set_display(
    change: &ResourceChange,
    before: &Value,
    r: &dyn ResourceRunner,
) -> Result<()> {
    displays::apply(change, before, r)
}
pub(crate) fn restore_display(
    change: &ResourceChange,
    before: &Value,
    r: &dyn ResourceRunner,
) -> Result<()> {
    displays::restore(change, before, r)
}

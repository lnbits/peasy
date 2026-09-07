//! bluetooth pea: unprivileged discovery, review and execution.
use crate::CancellableCommand;
use crate::{LocalAction, LocalProposal, LocalResult, PeasyClient, Resolution, safe_stderr};
use anyhow::{Context, Result, bail};
use peasy_core::{DiffKind, DiffLine, validate_query};
use std::process::Command;

pub(super) fn valid_bluetooth_address(value: &str) -> bool {
    let parts = value.split(':').collect::<Vec<_>>();
    parts.len() == 6
        && parts
            .iter()
            .all(|part| part.len() == 2 && part.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

impl PeasyClient {
    pub(super) fn propose_bluetooth(&self, query: &str) -> Result<Resolution> {
        let query = validate_query(query)?;
        let _ = Command::new(&self.tools.bluetoothctl)
            .args(["--timeout", "8", "scan", "on"])
            .cancellable_output();
        let output = Command::new(&self.tools.bluetoothctl)
            .arg("devices")
            .cancellable_output()
            .context("listing Bluetooth devices")?;
        if !output.status.success() {
            bail!("Bluetooth is unavailable");
        }
        let words = query
            .to_lowercase()
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let mut devices = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| {
                let rest = line.strip_prefix("Device ")?;
                let (address, name) = rest.split_once(' ')?;
                let lower = name.to_lowercase();
                (valid_bluetooth_address(address) && words.iter().all(|word| lower.contains(word)))
                    .then(|| (name.to_owned(), address.to_owned()))
            })
            .collect::<Vec<_>>();
        devices.sort();
        devices.dedup();
        let (name, address) = match devices.as_slice() {
            [] => bail!("I couldn't find a Bluetooth device matching `{query}`"),
            [device] => device.clone(),
            _ => bail!(
                "More than one Bluetooth device matched: {}",
                devices
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        Ok(Resolution::LocalProposal(LocalProposal {
            title: format!("Connect Bluetooth device {name}"),
            diff: vec![DiffLine {
                kind: DiffKind::Add,
                text: format!("Bluetooth: {name} ({address})"),
            }],
            action: LocalAction::Bluetooth { name, address },
        }))
    }

    pub(super) fn apply_bluetooth(&self, name: &str, address: &str) -> Result<LocalResult> {
        let connected = Command::new(&self.tools.bluetoothctl)
            .args(["--timeout", "45", "connect", address])
            .cancellable_output()
            .context("connecting Bluetooth device")?;
        if !connected.status.success() {
            let paired = Command::new(&self.tools.bluetoothctl)
                .args(["--timeout", "45", "pair", address])
                .cancellable_output()
                .context("pairing Bluetooth device")?;
            if !paired.status.success() {
                bail!(
                    "Bluetooth could not pair {name}: {}",
                    safe_stderr(&paired.stderr)
                );
            }
            let retry = Command::new(&self.tools.bluetoothctl)
                .args(["--timeout", "45", "connect", address])
                .cancellable_output()?;
            if !retry.status.success() {
                bail!(
                    "Bluetooth could not connect {name}: {}",
                    safe_stderr(&retry.stderr)
                );
            }
        }
        Ok(LocalResult {
            completed: true,
            message: format!("Connected Bluetooth device {name}."),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bluetooth_addresses_and_error_text_are_closed() {
        assert!(valid_bluetooth_address("AA:BB:CC:DD:EE:FF"));
        assert!(!valid_bluetooth_address("AA:BB:CC:DD:EE:FF;reboot"));
        assert!(!valid_bluetooth_address("../../device"));
        assert_eq!(
            safe_stderr(b"failure\x1b]52;secret\n"),
            "failure]52;secret\n"
        );
    }
}

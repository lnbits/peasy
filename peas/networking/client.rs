//! Fixed NetworkManager calls. Discovery is data; plans are validated twice.
use crate::{CancellableCommand, LocalAction, LocalProposal, LocalResult, PeasyClient, Resolution};
use anyhow::{Context, Result, bail};
use peasy_core::{
    DiffKind, DiffLine, IpcRequest, IpcResponse, NetworkKind, NetworkPlan, NetworkScope, WifiMode,
    validate_interface, validate_network_uuid,
};
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    process::{Command, Stdio},
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NetworkDevice {
    pub interface: String,
    pub kind: String,
    pub state: String,
    pub ap: bool,
    #[serde(skip)]
    pub hardware_address: String,
    pub ipv4: Vec<String>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NetworkConnection {
    pub uuid: String,
    pub name: String,
    pub kind: String,
    pub device: String,
    pub properties: Vec<String>,
}
impl NetworkConnection {
    pub(crate) fn needs_local_password(&self) -> bool {
        if !matches!(self.kind.as_str(), "wifi" | "802-11-wireless") {
            return false;
        }
        let value = |key: &str| {
            self.properties.iter().find_map(|line| {
                line.strip_prefix(key)
                    .and_then(|tail| tail.strip_prefix(':'))
            })
        };
        let secured = matches!(
            value("802-11-wireless-security.key-mgmt"),
            Some("wpa-psk" | "sae")
        );
        // Otherwise NetworkManager uses its saved credential or secret agent.
        let unsaved = value("802-11-wireless-security.psk-flags")
            .and_then(|flags| flags.split_whitespace().next())
            .and_then(|flags| flags.parse::<u32>().ok())
            .is_some_and(|flags| flags & 2 != 0);
        secured && unsaved
    }
}

#[cfg(test)]
mod credential_tests {
    use super::*;
    #[test]
    fn only_unsaved_secured_profiles_require_a_password() {
        for (security, flags, expected) in [
            ("--", "2 (not saved)", false),
            ("wpa-psk", "0 (none)", false),
            ("wpa-psk", "1 (agent owned)", false),
            ("wpa-psk", "2 (not saved)", true),
            ("sae", "2", true),
        ] {
            let connection = NetworkConnection {
                uuid: String::new(),
                name: String::new(),
                kind: "wifi".into(),
                device: String::new(),
                properties: vec![
                    format!("802-11-wireless-security.key-mgmt:{security}"),
                    format!("802-11-wireless-security.psk-flags:{flags}"),
                ],
            };
            assert_eq!(connection.needs_local_password(), expected);
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NetworkSnapshot {
    pub devices: Vec<NetworkDevice>,
    pub connections: Vec<NetworkConnection>,
}

pub(super) fn instructions() -> &'static str {
    include_str!("instructions.txt").trim_end()
}
fn fields(line: &str) -> Result<Vec<String>> {
    let mut out = vec![String::new()];
    let mut escaped = false;
    for c in line.chars() {
        if c.is_control() {
            bail!("invalid NetworkManager output");
        }
        if escaped {
            if !matches!(c, ':' | '\\') {
                bail!("invalid NetworkManager escaping");
            }
            out.last_mut().unwrap().push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == ':' {
            out.push(String::new());
        } else {
            out.last_mut().unwrap().push(c);
        }
    }
    if escaped {
        bail!("truncated NetworkManager output");
    }
    Ok(out)
}
impl PeasyClient {
    fn network_read(&self, args: &[&str]) -> Result<String> {
        let output = Command::new(&self.tools.nmcli)
            .env("LC_ALL", "C")
            .args(args)
            .cancellable_output()
            .context("NetworkManager is unavailable")?;
        if !output.status.success() {
            bail!("NetworkManager could not inspect these resources");
        }
        if output.stdout.len() > 64 * 1024 {
            bail!("NetworkManager returned too much data");
        }
        String::from_utf8(output.stdout).context("invalid NetworkManager output")
    }
    pub(super) fn network_snapshot(&self) -> Result<NetworkSnapshot> {
        let devices = self.network_read(&[
            "--terse",
            "--escape",
            "yes",
            "--fields",
            "DEVICE,TYPE,STATE",
            "device",
            "status",
        ])?;
        let mut snapshot = NetworkSnapshot {
            devices: vec![],
            connections: vec![],
        };
        if devices.lines().count() > 32 {
            bail!("too many network devices");
        }
        for line in devices.lines() {
            let row = fields(line)?;
            if row.len() != 3 {
                bail!("invalid device row");
            }
            if !matches!(row[1].as_str(), "ethernet" | "wifi") {
                continue;
            }
            validate_interface(&row[0])?;
            let ap = if row[1] == "wifi" {
                self.network_read(&[
                    "--get-values",
                    "WIFI-PROPERTIES.AP",
                    "device",
                    "show",
                    &row[0],
                ])?
                .trim()
                    == "yes"
            } else {
                false
            };
            let hardware_address = self
                .network_read(&["--get-values", "GENERAL.HWADDR", "device", "show", &row[0]])?
                .trim()
                .to_owned();
            if hardware_address.len() > 64
                || !hardware_address
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() || b == b':')
            {
                bail!("invalid device identity");
            }
            let ipv4 = self.network_read(&[
                "--terse",
                "--escape",
                "yes",
                "--fields",
                "IP4.ADDRESS,IP4.GATEWAY,IP4.DNS,IP4.ROUTE",
                "device",
                "show",
                &row[0],
            ])?;
            let ipv4: Vec<_> = ipv4.lines().take(32).map(str::to_owned).collect();
            if ipv4
                .iter()
                .any(|l| l.len() > 512 || l.chars().any(char::is_control))
            {
                bail!("invalid network addressing data");
            }
            snapshot.devices.push(NetworkDevice {
                interface: row[0].clone(),
                kind: row[1].clone(),
                state: row[2].clone(),
                ap,
                hardware_address,
                ipv4,
            });
        }
        let profiles = self.network_read(&[
            "--terse",
            "--escape",
            "yes",
            "--fields",
            "UUID,NAME,TYPE,DEVICE",
            "connection",
            "show",
        ])?;
        if profiles.lines().count() > 128 {
            bail!("too many connection profiles");
        }
        for line in profiles.lines() {
            let row = fields(line)?;
            if row.len() != 4
                || !validate_network_uuid(&row[0])
                || row.iter().any(|v| v.len() > 256)
            {
                bail!("invalid connection row");
            }
            let safe_fields = if matches!(row[2].as_str(), "802-11-wireless" | "wifi") {
                "connection.interface-name,connection.autoconnect,ipv4.method,ipv4.addresses,ipv4.gateway,ipv4.dns,ipv4.routes,ipv4.route-metric,ipv4.never-default,802-11-wireless.ssid,802-11-wireless.mode,802-11-wireless-security.key-mgmt,802-11-wireless-security.psk-flags"
            } else {
                "connection.interface-name,connection.autoconnect,ipv4.method,ipv4.addresses,ipv4.gateway,ipv4.dns,ipv4.routes,ipv4.route-metric,ipv4.never-default"
            };
            let properties = self.network_read(&[
                "--terse",
                "--escape",
                "yes",
                "--fields",
                safe_fields,
                "connection",
                "show",
                "uuid",
                &row[0],
            ])?;
            if properties.lines().count() > 32
                || properties
                    .lines()
                    .any(|l| l.len() > 1024 || l.chars().any(char::is_control))
            {
                bail!("connection metadata exceeds its limit");
            }
            snapshot.connections.push(NetworkConnection {
                uuid: row[0].clone(),
                name: row[1].clone(),
                kind: row[2].clone(),
                device: row[3].clone(),
                properties: properties.lines().map(str::to_owned).collect(),
            });
        }
        snapshot
            .devices
            .sort_by(|a, b| a.interface.cmp(&b.interface));
        snapshot.connections.sort_by(|a, b| a.uuid.cmp(&b.uuid));
        Ok(snapshot)
    }
    pub(super) fn propose_network(&self, plan: NetworkPlan) -> Result<Resolution> {
        let snapshot = self.network_snapshot()?;
        validate_resources(&plan, &snapshot)?;
        if plan.scope == NetworkScope::System {
            let state = match self.ipc.request(&IpcRequest::GetManagedModule)? {
                IpcResponse::ManagedModule { module } => {
                    peasy_core::parse_packages_module(&module)?
                }
                _ => bail!("unexpected managed network state response"),
            };
            for p in &plan.profiles {
                if snapshot
                    .connections
                    .iter()
                    .any(|c| c.uuid == p.uuid() || c.name == p.connection_id())
                    && !state.networks.iter().any(|owned| owned.id == p.id)
                {
                    bail!(
                        "a connection with this identity already exists outside Peasy's managed state"
                    );
                }
            }
            return match self.ipc.request(&IpcRequest::ProposeNetwork { plan })? {
                IpcResponse::Proposal { proposal } => Ok(Resolution::Proposal(proposal)),
                _ => bail!("unexpected network proposal response"),
            };
        }
        let mut diff = vec![DiffLine { kind: DiffKind::Context, text: "Live network change; may interrupt connectivity. Temporary profiles use NetworkManager runtime storage and are not NixOS state. Shared mode follows the current default route; this does not change the system firewall. IPv6 is disabled on new profiles.".into() }];
        for connection in snapshot.connections.iter().filter(|c| {
            plan.activate.as_ref() == Some(&c.uuid) || plan.deactivate.as_ref() == Some(&c.uuid)
        }) {
            diff.push(DiffLine {
                kind: DiffKind::Context,
                text: format!(
                    "Connection: {} ({}, UUID {}), current device: {}",
                    connection.name, connection.kind, connection.uuid, connection.device
                ),
            });
            diff.extend(connection.properties.iter().map(|property| DiffLine {
                kind: DiffKind::Context,
                text: property.clone(),
            }));
        }
        for p in &plan.profiles {
            for current in snapshot
                .connections
                .iter()
                .filter(|c| c.device == p.interface)
            {
                diff.push(DiffLine { kind: DiffKind::Context, text: format!("Currently active on {}: {} ({}). Activating the new profile may disconnect it.", p.interface, current.name, current.uuid) });
            }
            for (key, value) in p.properties() {
                diff.push(DiffLine {
                    kind: DiffKind::Add,
                    text: format!("{key}: {value}"),
                });
            }
        }
        if let Some(id) = &plan.activate {
            diff.push(DiffLine {
                kind: DiffKind::Add,
                text: format!("Activate connection: {id}"),
            });
        }
        if let Some(id) = &plan.deactivate {
            diff.push(DiffLine {
                kind: DiffKind::Remove,
                text: format!("Deactivate connection: {id}"),
            });
        }
        Ok(Resolution::LocalProposal(LocalProposal {
            title: "Change network connections".into(),
            diff,
            action: LocalAction::Network { plan, snapshot },
        }))
    }
    fn network_mutate(&self, args: &[String], password: Option<&str>) -> Result<()> {
        let mut command = Command::new(&self.tools.nmcli);
        command.env("LC_ALL", "C").args(["--wait", "45"]);
        if password.is_some() {
            command.arg("--ask");
        }
        let mut child = command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("starting NetworkManager")?;
        if let Some(mut stdin) = child.stdin.take()
            && let Some(password) = password
        {
            stdin.write_all(password.as_bytes())?;
            stdin.write_all(b"\n")?;
        }
        if !child.wait()?.success() {
            bail!("NetworkManager rejected the change or authorization failed");
        }
        Ok(())
    }
    pub(super) fn apply_network(
        &self,
        plan: &NetworkPlan,
        expected: &NetworkSnapshot,
        password: Option<&str>,
    ) -> Result<LocalResult> {
        let current = self.network_snapshot()?;
        validate_resources(plan, &current)?;
        if &current != expected {
            bail!("Network resources changed since review; request a new proposal");
        }
        if plan.scope != NetworkScope::Session {
            bail!("persistent network changes require system review");
        }
        if plan.password_required()
            && !password.is_some_and(|p| {
                p.len() >= 8 && p.len() <= 63 && p.is_ascii() && !p.chars().any(char::is_control)
            })
        {
            bail!("a local WPA2 password of 8–63 printable ASCII characters is required");
        }
        let new_profile = plan.profiles.first();
        let previous = new_profile
            .and_then(|p| current.connections.iter().find(|c| c.device == p.interface))
            .map(|c| c.uuid.clone());
        if let Some(p) = new_profile {
            let mut args: Vec<String> = ["connection", "add", "save", "no"]
                .into_iter()
                .map(str::to_owned)
                .collect();
            for (key, value) in p.properties() {
                args.extend([key.into(), value]);
            }
            self.network_mutate(&args, None)?;
        }
        let uuid = new_profile
            .map(|p| p.uuid())
            .or_else(|| plan.activate.clone())
            .or_else(|| plan.deactivate.clone())
            .context("missing network target")?;
        let verb = if plan.activate.is_some() {
            "up"
        } else {
            "down"
        };
        let result = self
            .network_mutate(
                &[
                    "connection".into(),
                    verb.into(),
                    "uuid".into(),
                    uuid.clone(),
                ],
                password,
            )
            .and_then(|()| {
                let observed = self.network_snapshot()?;
                let active = observed
                    .connections
                    .iter()
                    .any(|c| c.uuid == uuid && c.device != "--" && !c.device.is_empty());
                if active != plan.activate.is_some() {
                    bail!("the requested activation state was not observed");
                }
                Ok(())
            });
        if let Err(error) = result {
            let mut recovered = true;
            if new_profile.is_some() {
                recovered &= self
                    .network_mutate(
                        &["connection".into(), "delete".into(), "uuid".into(), uuid],
                        None,
                    )
                    .is_ok();
            }
            if let Some(previous) = previous {
                recovered &= self
                    .network_mutate(
                        &["connection".into(), "up".into(), "uuid".into(), previous],
                        None,
                    )
                    .is_ok();
            }
            bail!(
                "{error}. {}",
                if new_profile.is_none() {
                    "No profile was created; check the existing connection locally."
                } else if recovered {
                    "Temporary profile cleanup completed; check connectivity."
                } else {
                    "Recovery was incomplete; check NetworkManager locally."
                }
            );
        }
        Ok(LocalResult {
            completed: true,
            message: "Network connection state updated. Internet reachability has not been tested."
                .into(),
        })
    }
}
fn validate_resources(plan: &NetworkPlan, snapshot: &NetworkSnapshot) -> Result<()> {
    plan.validate()?;
    for p in &plan.profiles {
        let device = snapshot
            .devices
            .iter()
            .find(|d| d.interface == p.interface)
            .context("interface was not discovered")?;
        if device.kind
            != match p.kind {
                NetworkKind::Wifi => "wifi",
                NetworkKind::Ethernet => "ethernet",
            }
            || device.state == "unmanaged"
        {
            bail!("interface does not support this profile");
        }
        if p.wifi_mode == Some(WifiMode::Ap) && !device.ap {
            bail!("interface does not support access-point mode");
        }
        if plan.scope == NetworkScope::Session
            && snapshot
                .connections
                .iter()
                .any(|c| c.uuid == p.uuid() || c.name == p.connection_id())
        {
            bail!(
                "connection already exists; review its persistent configuration or activate its discovered UUID"
            );
        }
    }
    if plan.profiles.is_empty() {
        for id in plan.activate.iter().chain(plan.deactivate.iter()) {
            if !snapshot.connections.iter().any(|c| &c.uuid == id) {
                bail!("connection UUID was not discovered");
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn escaped_network_names_are_data() {
        assert_eq!(
            fields(r"id:Coffee\:shop\\wifi:wifi:--").unwrap(),
            vec!["id", "Coffee:shop\\wifi", "wifi", "--"]
        );
        assert!(fields("bad\\").is_err());
        assert!(fields("bad\nname").is_err());
    }
    #[test]
    fn nonexistent_resources_are_rejected() {
        let plan = NetworkPlan {
            scope: NetworkScope::Session,
            profiles: vec![],
            remove: vec![],
            activate: Some("12345678-1234-1234-1234-123456789abc".into()),
            deactivate: None,
        };
        assert!(
            validate_resources(
                &plan,
                &NetworkSnapshot {
                    devices: vec![],
                    connections: vec![]
                }
            )
            .is_err()
        );
    }
}

#[cfg(test)]
mod execution_tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt};
    fn fixture() -> (tempfile::TempDir, PeasyClient, NetworkPlan) {
        let temp = tempfile::tempdir().unwrap();
        let tool = temp.path().join("nmcli");
        fs::write(
            &tool,
            format!(
                r#"#!/bin/sh
cd '{}'
printf '%s\n' "$@" >> args
case "$*" in
  *'device status') echo 'wlan0:wifi:disconnected' ;;
  *'WIFI-PROPERTIES.AP device show wlan0') echo yes ;;
  *'GENERAL.HWADDR device show wlan0') echo '02:00:00:00:00:01' ;;
  *'connection show uuid'*) echo 'connection.interface-name:wlan0' ;;
  *'IP4.ADDRESS,IP4.GATEWAY,IP4.DNS,IP4.ROUTE device show wlan0')
    if [ -f stale ]; then echo 'IP4.GATEWAY:192.168.1.1'; fi ;;
  *'connection show')
    if [ -f uuid ]; then
      read -r uuid < uuid
      device=--
      if [ -f active ]; then device=wlan0; fi
      echo "$uuid:peasy-wireless:802-11-wireless:$device"
    fi ;;
  *'connection add save no'*)
    while [ "$#" -gt 0 ]; do
      if [ "$1" = connection.uuid ]; then shift; echo "$1" > uuid; break; fi
      shift
    done ;;
  *'connection up uuid'*)
    if [ -f fail ]; then exit 10; fi
    read -r secret
    test "$secret" = local-test-secret || exit 11
    touch active ;;
  *'connection down uuid'*) rm -f active ;;
  *'connection delete uuid'*) rm -f uuid active ;;
  *) exit 12 ;;
esac
"#,
                temp.path().display()
            ),
        )
        .unwrap();
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o700)).unwrap();
        let engine = std::env::var_os("PEASY_TEST_ENGINE").expect("compiled guest");
        let mut client = PeasyClient::with_provider(
            temp.path().join("unused-socket"),
            std::path::Path::new(&engine),
            crate::ModelProvider::Ollama {
                base_url: "http://127.0.0.1:11434".into(),
                model: "unused".into(),
            },
        )
        .unwrap();
        client.tools.nmcli = tool;
        let mut plan: NetworkPlan = serde_json::from_str(include_str!("example.json")).unwrap();
        plan.scope = NetworkScope::Session;
        plan.profiles.truncate(1);
        (temp, client, plan)
    }
    #[test]
    #[ignore = "requires PEASY_TEST_ENGINE; packaged checks run this"]
    fn network_review_stale_check_secret_input_and_activation_use_only_mock_tools() {
        let (temp, client, plan) = fixture();
        let Resolution::LocalProposal(proposal) = client.propose_network(plan).unwrap() else {
            panic!("local review expected");
        };
        assert!(proposal.password_required());
        assert!(!temp.path().join("uuid").exists());
        fs::write(temp.path().join("stale"), "").unwrap();
        assert!(
            client
                .apply_local(&proposal, Some("local-test-secret"))
                .err()
                .unwrap()
                .to_string()
                .contains("changed since review")
        );
        assert!(!temp.path().join("uuid").exists());
        fs::remove_file(temp.path().join("stale")).unwrap();
        assert!(
            client
                .apply_local(&proposal, Some("local-test-secret"))
                .unwrap()
                .completed
        );
        let log = fs::read_to_string(temp.path().join("args")).unwrap();
        assert!(log.contains("connection\nadd\nsave\nno"));
        assert!(!log.contains("local-test-secret"));
        assert!(temp.path().join("active").exists());
    }
    #[test]
    #[ignore = "requires PEASY_TEST_ENGINE; packaged checks run this"]
    fn failed_activation_cleans_up_only_the_new_temporary_profile() {
        let (temp, client, plan) = fixture();
        let Resolution::LocalProposal(proposal) = client.propose_network(plan).unwrap() else {
            panic!("local review expected");
        };
        fs::write(temp.path().join("fail"), "").unwrap();
        assert!(
            client
                .apply_local(&proposal, Some("local-test-secret"))
                .is_err()
        );
        assert!(!temp.path().join("uuid").exists());
        assert!(!temp.path().join("active").exists());
    }
}

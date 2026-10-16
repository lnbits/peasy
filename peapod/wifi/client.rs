//! Wi-Fi review binds the access point, interface and negotiated security policy.
use super::networking::fields;
use crate::{LocalAction, LocalProposal, LocalResult, PeasyClient, Resolution};
use anyhow::{Context, Result, bail};
use peasy_core::{DiffKind, DiffLine, validate_interface, validate_network_uuid, validate_ssid};

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct WifiNetwork {
    pub ssid: String,
    pub security: String,
    pub bssid: String,
    pub interface: String,
}
impl WifiNetwork {
    fn key_management(&self) -> Result<&'static str> {
        match self.security.as_str() {
            "" | "--" => Ok("none"),
            "WPA2" | "WPA1 WPA2" => Ok("wpa-psk"),
            "WPA3" | "WPA2 WPA3" | "WPA1 WPA2 WPA3" => Ok("sae"),
            _ => bail!("This Wi-Fi security mode requires the desktop network settings"),
        }
    }
}
impl PeasyClient {
    pub(super) fn propose_wifi(
        &self,
        requested_ssid: &str,
        password: Option<String>,
    ) -> Result<Resolution> {
        validate_ssid(requested_ssid)?;
        let network = select_network(self.wifi_networks()?, requested_ssid)?;
        let password_required = network.key_management()? != "none";
        Ok(Resolution::LocalProposal(LocalProposal {
            title: format!("Connect to Wi-Fi {}", network.ssid),
            diff: vec![
                DiffLine { kind: DiffKind::Add, text: format!("Wi-Fi network: {}", network.ssid) },
                DiffLine { kind: DiffKind::Context, text: format!("Security: {}; access point: {}; interface: {}", if password_required { &network.security } else { "open network" }, network.bssid, network.interface) },
                DiffLine { kind: DiffKind::Context, text: "Session connection: password is not saved; automatic reconnection is disabled. The current connection on this interface may disconnect.".into() },
            ],
            action: LocalAction::Wifi { network, password, password_required },
        }))
    }

    pub(super) fn list_wifi(&self) -> Result<Resolution> {
        let mut networks: Vec<_> = self
            .wifi_networks()?
            .into_iter()
            .map(|n| (n.ssid, n.security))
            .collect();
        networks.sort();
        networks.dedup();
        if networks.is_empty() {
            return Ok(Resolution::Explain(
                "No nearby Wi-Fi networks are currently visible.".into(),
            ));
        }
        let lines = networks
            .into_iter()
            .take(20)
            .map(|(ssid, security)| {
                format!(
                    "• {ssid} — {}",
                    if security.is_empty() || security == "--" {
                        "open"
                    } else {
                        &security
                    }
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        Ok(Resolution::Explain(format!(
            "Nearby Wi-Fi networks:\n{lines}"
        )))
    }

    pub(super) fn wifi_networks(&self) -> Result<Vec<WifiNetwork>> {
        let output = self.network_read(&[
            "--terse",
            "--escape",
            "yes",
            "--fields",
            "SSID,SECURITY,BSSID,DEVICE",
            "device",
            "wifi",
            "list",
            "--rescan",
            "yes",
        ])?;
        parse_networks(&output)
    }

    pub(super) fn apply_wifi(
        &self,
        network: &WifiNetwork,
        password: &Option<String>,
        supplied_password: Option<&str>,
    ) -> Result<LocalResult> {
        let key_management = network.key_management()?;
        let password = supplied_password
            .filter(|p| !p.is_empty())
            .or(password.as_deref());
        if key_management != "none"
            && !password.is_some_and(|p| {
                (8..=63).contains(&p.len()) && p.is_ascii() && !p.chars().any(char::is_control)
            })
        {
            bail!("a local Wi-Fi password of 8–63 printable ASCII characters is required");
        }
        if !self.wifi_networks()?.contains(network) {
            bail!("Wi-Fi access point or security changed since review; request a new proposal");
        }
        let previous = self.network_read(&[
            "--get-values",
            "GENERAL.CON-UUID",
            "device",
            "show",
            &network.interface,
        ])?;
        let previous = previous.trim();
        if !previous.is_empty() && previous != "--" && !validate_network_uuid(previous) {
            bail!("invalid current connection identity");
        }
        let uuid = std::fs::read_to_string("/proc/sys/kernel/random/uuid")?;
        let uuid = uuid.trim();
        if !validate_network_uuid(uuid) {
            bail!("could not generate connection identity");
        }
        // Explicit key management and RSN forbid a downgrade even if the AP
        // changes after the rescan. Never let `device wifi connect` infer security.
        let mut args: Vec<String> = [
            "connection",
            "add",
            "save",
            "no",
            "connection.id",
            "peasy-wifi-session",
            "connection.uuid",
            uuid,
            "connection.type",
            "802-11-wireless",
            "connection.interface-name",
            &network.interface,
            "connection.autoconnect",
            "false",
            "802-11-wireless.ssid",
            &network.ssid,
            "802-11-wireless.bssid",
            &network.bssid,
            "802-11-wireless.mode",
            "infrastructure",
            "ipv4.method",
            "auto",
            "ipv6.method",
            "auto",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        if key_management != "none" {
            args.extend(
                [
                    "802-11-wireless-security.key-mgmt",
                    key_management,
                    "802-11-wireless-security.proto",
                    "rsn",
                    "802-11-wireless-security.psk-flags",
                    "2",
                ]
                .map(str::to_owned),
            );
        }
        // An open profile omits security entirely. key-mgmt=none with a
        // security setting present describes static WEP and requests a WEP key.
        self.network_mutate(&args, None)?;
        let result = self
            .network_mutate(
                &["connection".into(), "up".into(), "uuid".into(), uuid.into()],
                if key_management == "none" {
                    None
                } else {
                    password
                },
            )
            .and_then(|()| {
                let active = self.network_read(&[
                    "--get-values",
                    "GENERAL.CON-UUID",
                    "device",
                    "show",
                    &network.interface,
                ])?;
                if active.trim() != uuid {
                    bail!("the reviewed Wi-Fi connection did not become active");
                }
                Ok(())
            });
        if let Err(error) = result {
            let mut recovered = self
                .network_mutate(
                    &[
                        "connection".into(),
                        "delete".into(),
                        "uuid".into(),
                        uuid.into(),
                    ],
                    None,
                )
                .is_ok();
            if validate_network_uuid(previous) {
                recovered &= self
                    .network_mutate(
                        &[
                            "connection".into(),
                            "up".into(),
                            "uuid".into(),
                            previous.into(),
                        ],
                        None,
                    )
                    .is_ok();
            }
            bail!(
                "{error}. {}",
                if recovered {
                    "Temporary connection cleaned up; check connectivity."
                } else {
                    "Recovery was incomplete; check NetworkManager locally."
                }
            );
        }
        Ok(LocalResult {
            display_trial: None,
            completed: true,
            message: format!(
                "Connected to Wi-Fi {} for this session. Internet reachability has not been tested.",
                network.ssid
            ),
        })
    }
}

fn parse_networks(output: &str) -> Result<Vec<WifiNetwork>> {
    let mut networks = Vec::new();
    for line in output.lines() {
        let row = fields(line)?;
        if row.len() != 4 {
            bail!("invalid Wi-Fi scan row");
        }
        if row[0].is_empty() {
            continue;
        }
        validate_ssid(&row[0])?;
        validate_interface(&row[3])?;
        if row[2].len() != 17
            || !row[2]
                .split(':')
                .all(|part| part.len() == 2 && part.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            bail!("invalid Wi-Fi access point identity");
        }
        networks.push(WifiNetwork {
            ssid: row[0].clone(),
            security: row[1].clone(),
            bssid: row[2].to_ascii_uppercase(),
            interface: row[3].clone(),
        });
    }
    networks.sort();
    networks.dedup();
    Ok(networks)
}
fn select_network(mut networks: Vec<WifiNetwork>, requested: &str) -> Result<WifiNetwork> {
    let exact = networks.iter().any(|n| n.ssid == requested);
    networks.retain(|n| {
        if exact {
            n.ssid == requested
        } else {
            n.ssid.to_lowercase() == requested.to_lowercase()
        }
    });
    networks.sort();
    let selected = networks
        .first()
        .context("The requested Wi-Fi network was not found")?;
    if networks
        .iter()
        .any(|n| n.ssid != selected.ssid || n.security != selected.security)
    {
        bail!(
            "Wi-Fi name is ambiguous; choose the network and security in desktop network settings"
        );
    }
    Ok(selected.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::redact_wifi_password;
    use std::{fs, os::unix::fs::PermissionsExt};

    const SCAN: &str = r"Cafe:WPA2:02\:00\:00\:00\:00\:01:wlan0";

    #[test]
    fn scans_preserve_identity_and_reject_ambiguous_security() {
        let network = parse_networks(SCAN).unwrap().remove(0);
        assert_eq!(network.bssid, "02:00:00:00:00:01");
        assert_eq!(network.key_management().unwrap(), "wpa-psk");
        let open = parse_networks(&SCAN.replace("Cafe:WPA2", "cafe:--"))
            .unwrap()
            .remove(0);
        assert_eq!(
            select_network(vec![network.clone(), open.clone()], "cafe").unwrap(),
            open
        );
        assert!(select_network(vec![network.clone(), open], "CAFE").is_err());
        let downgrade = parse_networks(&SCAN.replace("WPA2", "--"))
            .unwrap()
            .remove(0);
        assert!(select_network(vec![network, downgrade], "Cafe").is_err());
        for security in ["WEP", "WPA1", "WPA2 802.1X", "OWE"] {
            let network = parse_networks(&SCAN.replace("WPA2", security))
                .unwrap()
                .remove(0);
            assert!(network.key_management().is_err());
        }
        let escaped = parse_networks(&SCAN.replace("Cafe", r"Cafe\:shop\\wifi")).unwrap();
        assert_eq!(escaped[0].ssid, r"Cafe:shop\wifi");
        assert!(parse_networks("Cafe:WPA2:invalid:wlan0").is_err());
        assert!(parse_networks("Cafe:WPA2").is_err());
    }

    fn fixture() -> (tempfile::TempDir, PeasyClient) {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("scan"), format!("{SCAN}\n")).unwrap();
        let tool = temp.path().join("nmcli");
        fs::write(
            &tool,
            format!(
                r#"#!/bin/sh
cd '{}'
printf '%s\n' "$@" >> args
case "$*" in
  *'device wifi list --rescan yes') cat scan ;;
  *'GENERAL.CON-UUID device show wlan0')
    if [ -f active ]; then cat active; else echo 12345678-1234-1234-1234-123456789abc; fi ;;
  *'connection add save no'*)
    while [ "$#" -gt 0 ]; do
      if [ "$1" = connection.uuid ]; then shift; echo "$1" > uuid; break; fi
      shift
    done ;;
  *'connection up uuid 12345678-1234-1234-1234-123456789abc') touch restored ;;
  *'connection up uuid'*)
    if [ -f fail ]; then exit 10; fi
    case "$*" in *--ask*) read -r secret; test "$secret" = local-test-secret || exit 11 ;; esac
    if [ ! -f unconfirmed ]; then cp uuid active; fi ;;
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
        (temp, client)
    }

    #[test]
    #[ignore = "requires PEASY_TEST_ENGINE; packaged checks run this"]
    fn wifi_rejects_changed_access_point_before_any_mutation() {
        for changed in [
            SCAN.replace("WPA2", "--"),
            SCAN.replace("00\\:01", "00\\:02"),
            SCAN.replace("wlan0", "wlan1"),
        ] {
            let (temp, client) = fixture();
            let Resolution::LocalProposal(proposal) = client.propose_wifi("Cafe", None).unwrap()
            else {
                panic!("review expected");
            };
            fs::write(temp.path().join("scan"), changed).unwrap();
            assert!(
                client
                    .apply_local(&proposal, Some("local-test-secret"))
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("changed since review")
            );
            assert!(
                !fs::read_to_string(temp.path().join("args"))
                    .unwrap()
                    .contains("add\nsave")
            );
        }
    }

    #[test]
    #[ignore = "requires PEASY_TEST_ENGINE; packaged checks run this"]
    fn wifi_pins_security_and_keeps_credentials_off_the_command_line() {
        for (security, expected_key) in [("WPA2", "wpa-psk"), ("WPA2 WPA3", "sae"), ("--", "none")]
        {
            let (temp, client) = fixture();
            fs::write(temp.path().join("scan"), SCAN.replace("WPA2", security)).unwrap();
            let Resolution::LocalProposal(proposal) = client.propose_wifi("Cafe", None).unwrap()
            else {
                panic!("review expected");
            };
            assert_eq!(proposal.password_required(), expected_key != "none");
            assert!(
                client
                    .apply_local(&proposal, Some("local-test-secret"))
                    .unwrap()
                    .completed
            );
            let args = fs::read_to_string(temp.path().join("args")).unwrap();
            assert!(args.contains("802-11-wireless.bssid\n02:00:00:00:00:01\n"));
            assert!(!args.contains("local-test-secret"));
            if expected_key != "none" {
                assert!(args.contains(&format!(
                    "802-11-wireless-security.key-mgmt\n{expected_key}\n"
                )));
                assert!(args.contains("802-11-wireless-security.proto\nrsn\n"));
                assert!(args.contains("802-11-wireless-security.psk-flags\n2\n"));
            } else {
                assert!(!args.contains("802-11-wireless-security"));
                assert!(!args.contains("--ask"));
            }
        }
    }

    #[test]
    #[ignore = "requires PEASY_TEST_ENGINE; packaged checks run this"]
    fn wifi_failed_or_unconfirmed_activation_cleans_up_and_restores_previous() {
        for failure in ["fail", "unconfirmed"] {
            let (temp, client) = fixture();
            let Resolution::LocalProposal(proposal) = client.propose_wifi("Cafe", None).unwrap()
            else {
                panic!("review expected");
            };
            fs::write(temp.path().join(failure), "").unwrap();
            assert!(
                client
                    .apply_local(&proposal, Some("local-test-secret"))
                    .is_err()
            );
            assert!(!temp.path().join("uuid").exists());
            assert!(temp.path().join("restored").exists());
        }
    }

    #[test]
    fn wifi_password_is_removed_before_the_model_boundary() {
        for request in [
            "connect to wifi CoolCafe with password test-secret",
            "password: test-secret",
            "connect to Cafe psk=test-secret",
            "connect to Cafe, passphrase is hidden value",
            "wifi key is test-secret",
            "wi-fi key is test-secret",
            "my API_KEY=test-secret",
            "sk-proj-test1234567890",
            "connect to Cafe with PASSWORD\ntest-secret",
        ] {
            let error = redact_wifi_password(request).unwrap_err().to_string();
            assert!(!error.contains("test-secret"));
        }
        assert!(redact_wifi_password("install a password manager").is_ok());
        assert!(redact_wifi_password("install task-manager").is_ok());

        let (request, password) = redact_wifi_password("what Wi-Fi is available?").unwrap();
        assert_eq!(request, "what Wi-Fi is available?");
        assert!(password.is_none());
    }
}

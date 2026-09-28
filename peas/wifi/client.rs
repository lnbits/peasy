//! wifi pea: unprivileged discovery, review and execution.
use crate::CancellableCommand;
use crate::{LocalAction, LocalProposal, LocalResult, PeasyClient, Resolution, safe_stderr};
use anyhow::{Context, Result, bail};
use peasy_core::{DiffKind, DiffLine, validate_ssid};
use std::{
    io::Write,
    process::{Command, Stdio},
};

impl PeasyClient {
    pub(super) fn propose_wifi(
        &self,
        requested_ssid: &str,
        password: Option<String>,
    ) -> Result<Resolution> {
        validate_ssid(requested_ssid)?;
        let (ssid, security) = select_network(self.wifi_networks()?, requested_ssid)?;
        let password_required = !security.trim().is_empty() && security.trim() != "--";
        Ok(Resolution::LocalProposal(LocalProposal {
            title: format!("Connect to Wi-Fi {ssid}"),
            diff: vec![
                DiffLine {
                    kind: DiffKind::Add,
                    text: format!("Wi-Fi network: {ssid}"),
                },
                DiffLine {
                    kind: DiffKind::Context,
                    text: if password.is_some() {
                        "Password: supplied locally (hidden)".into()
                    } else if password_required {
                        "Password: required locally before connecting".into()
                    } else {
                        "Security: open network".into()
                    },
                },
            ],
            action: LocalAction::Wifi {
                ssid,
                password,
                password_required,
            },
        }))
    }

    pub(super) fn list_wifi(&self) -> Result<Resolution> {
        let networks = self.wifi_networks()?;
        if networks.is_empty() {
            return Ok(Resolution::Explain(
                "No nearby Wi-Fi networks are currently visible.".into(),
            ));
        }
        let lines = networks
            .into_iter()
            .take(20)
            .map(|(ssid, security)| {
                if security.trim().is_empty() || security.trim() == "--" {
                    format!("• {ssid} — open")
                } else {
                    format!("• {ssid} — secured")
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        Ok(Resolution::Explain(format!(
            "Nearby Wi-Fi networks:\n{lines}"
        )))
    }

    pub(super) fn wifi_networks(&self) -> Result<Vec<(String, String)>> {
        let output = Command::new(&self.tools.nmcli)
            .args([
                "--terse",
                "--escape",
                "no",
                "--fields",
                "SSID,SECURITY",
                "device",
                "wifi",
                "list",
                "--rescan",
                "auto",
            ])
            .cancellable_output()
            .context("listing nearby Wi-Fi networks")?;
        if !output.status.success() {
            bail!("NetworkManager could not list Wi-Fi networks");
        }
        let mut networks = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| {
                let (ssid, security) = line.rsplit_once(':').unwrap_or((line, ""));
                (!ssid.is_empty() && validate_ssid(ssid).is_ok())
                    .then(|| (ssid.to_owned(), security.to_owned()))
            })
            .collect::<Vec<_>>();
        networks.sort();
        networks.dedup();
        Ok(networks)
    }

    pub(super) fn apply_wifi(
        &self,
        ssid: &str,
        password: &Option<String>,
        password_required: &bool,
        supplied_password: Option<&str>,
    ) -> Result<LocalResult> {
        let password = supplied_password
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .or_else(|| password.clone());
        if *password_required && password.is_none() {
            bail!("a Wi-Fi password is required");
        }
        let mut command = Command::new(&self.tools.nmcli);
        command.args(["--wait", "45"]);
        if password.is_some() {
            command.arg("--ask").stdin(Stdio::piped());
        }
        command.args(["device", "wifi", "connect", ssid]);
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("starting NetworkManager connection")?;
        if let Some(password) = password
            && let Some(mut stdin) = child.stdin.take()
        {
            stdin.write_all(password.as_bytes())?;
            stdin.write_all(b"\n")?;
        }
        let output = child.wait_with_output()?;
        if !output.status.success() {
            bail!(
                "NetworkManager could not connect: {}",
                safe_stderr(&output.stderr)
            );
        }
        Ok(LocalResult {
            completed: true,
            message: format!("Connected to Wi-Fi {ssid}."),
        })
    }
}

fn select_network(
    mut networks: Vec<(String, String)>,
    requested: &str,
) -> Result<(String, String)> {
    let exact = networks.iter().any(|(ssid, _)| ssid == requested);
    networks.retain(|(ssid, _)| {
        if exact {
            ssid == requested
        } else {
            ssid.to_lowercase() == requested.to_lowercase()
        }
    });
    networks.sort();
    networks.dedup();
    if networks.len() > 1 {
        bail!(
            "Wi-Fi name `{requested}` is ambiguous. Choose the exact network name and security in the desktop network settings."
        );
    }
    networks
        .pop()
        .with_context(|| format!("I couldn't find the Wi-Fi network `{requested}`"))
}

#[cfg(test)]
mod tests {
    use super::select_network;
    use crate::redact_wifi_password;
    #[test]
    fn wifi_names_are_case_sensitive_and_security_conflicts_are_not_guessed() {
        let networks = vec![("Cafe".into(), "WPA2".into()), ("cafe".into(), "--".into())];
        assert_eq!(select_network(networks.clone(), "cafe").unwrap().1, "--");
        assert!(select_network(networks, "CAFE").is_err());
        assert!(
            select_network(
                vec![("Cafe".into(), "WPA2".into()), ("Cafe".into(), "--".into())],
                "Cafe"
            )
            .is_err()
        );
        assert_eq!(
            select_network(vec![("Cafe".into(), "WPA2".into())], "cafe")
                .unwrap()
                .0,
            "Cafe"
        );
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

//! wifi pea: unprivileged discovery, review and execution.
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
        let requested = requested_ssid.to_lowercase();
        let mut matches = self
            .wifi_networks()?
            .into_iter()
            .filter(|(ssid, _)| ssid.to_lowercase() == requested)
            .collect::<Vec<_>>();
        matches.sort();
        matches.dedup();
        let (ssid, security) = matches
            .into_iter()
            .next()
            .with_context(|| format!("I couldn't find the Wi-Fi network `{requested_ssid}`"))?;
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
            .output()
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
        networks.sort_by_key(|network| network.0.to_lowercase());
        networks.dedup_by(|left, right| left.0 == right.0);
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

#[cfg(test)]
mod tests {
    use crate::redact_wifi_password;
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

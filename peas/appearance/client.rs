//! appearance pea: unprivileged discovery, review and execution.
use crate::{PeasyClient, Resolution, runtime_desktop_kind, safe_stderr, tool_path};
use anyhow::{Context, Result, bail};
use peasy_core::{IpcRequest, IpcResponse, ThemeSettings};
use std::{fs, path::Path, process::Command};

pub(super) mod adapters;

/// Apply only Peasy's validated GNOME appearance enums in the current user's
/// session. This deliberately runs without privilege and never accepts an
/// executable, schema, key, or value from a model provider.
pub fn apply_live_theme_with(gsettings: &Path, theme: &ThemeSettings) -> Result<()> {
    const SCHEMA: &str = "org.gnome.desktop.interface";
    let mut changes = Vec::new();
    if let Some(color) = theme.accent_color {
        changes.push(("accent-color", color.to_string()));
    }
    if let Some(scheme) = theme.color_scheme {
        changes.push(("color-scheme", scheme.gsettings_value().to_owned()));
    }
    for (key, _) in &changes {
        let writable = Command::new(gsettings)
            .args(["writable", SCHEMA, key])
            .output()
            .with_context(|| format!("checking GNOME setting {key}"))?;
        if !writable.status.success() || String::from_utf8_lossy(&writable.stdout).trim() != "true"
        {
            bail!("GNOME setting {key} is unavailable or locked in this session");
        }
    }
    for (key, value) in changes {
        let changed = Command::new(gsettings)
            .args(["set", SCHEMA, key, &value])
            .output()
            .with_context(|| format!("applying GNOME setting {key}"))?;
        if !changed.status.success() {
            bail!("GNOME rejected {key}: {}", safe_stderr(&changed.stderr));
        }
    }
    Ok(())
}

pub fn sync_live_theme_from_file(theme_file: &Path, gsettings: &Path) -> Result<()> {
    let metadata = fs::metadata(theme_file).context("reading active Peasy theme metadata")?;
    if !metadata.is_file() || metadata.len() > 4096 {
        bail!("active Peasy theme state is not a small regular file");
    }
    let theme: ThemeSettings = serde_json::from_slice(&fs::read(theme_file)?)
        .context("parsing active Peasy theme state")?;
    adapters::apply(
        runtime_desktop_kind(),
        &theme,
        gsettings,
        &tool_path(
            "PEASY_PLASMA_COLORSCHEME",
            "/run/current-system/sw/bin/plasma-apply-colorscheme",
        ),
        &tool_path(
            "PEASY_KWRITECONFIG",
            "/run/current-system/sw/bin/kwriteconfig6",
        ),
    )
}

pub(super) fn theme_choices() -> String {
    adapters::choices(runtime_desktop_kind())
}

impl PeasyClient {
    pub(super) fn propose_theme(&self, theme: ThemeSettings) -> Result<Resolution> {
        let current = match self.ipc.request(&IpcRequest::GetTheme)? {
            IpcResponse::Theme { theme } => theme,
            _ => bail!("unexpected response to GetTheme"),
        };
        runtime_desktop_kind().validate_appearance(&current.merged(&theme))?;
        match self.ipc.request(&IpcRequest::ProposeTheme { theme })? {
            IpcResponse::Proposal { proposal } => Ok(Resolution::Proposal(*proposal)),
            _ => bail!("unexpected response to ProposeTheme"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn live_theme_uses_only_fixed_gsettings_arguments() {
        let temp = tempfile::tempdir().unwrap();
        let tool = temp.path().join("gsettings");
        let log = temp.path().join("arguments");
        fs::write(
            &tool,
            format!(
                "#!/bin/sh\nif [ \"$1\" = writable ]; then echo true; exit 0; fi\nprintf '%s\\n' \"$@\" >> '{}'\n",
                log.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o700)).unwrap();
        let theme = ThemeSettings {
            accent_color: Some(peasy_core::AccentColor::Purple),
            color_scheme: Some(peasy_core::ColorScheme::Dark),
        };
        apply_live_theme_with(&tool, &theme).unwrap();
        assert_eq!(
            fs::read_to_string(&log)
                .unwrap()
                .lines()
                .collect::<Vec<_>>(),
            [
                "set",
                "org.gnome.desktop.interface",
                "accent-color",
                "purple",
                "set",
                "org.gnome.desktop.interface",
                "color-scheme",
                "prefer-dark",
            ]
        );

        let state = temp.path().join("theme.json");
        fs::write(
            &state,
            r#"{"accent_color":"blue","color_scheme":"light","command":"sh"}"#,
        )
        .unwrap();
        assert!(sync_live_theme_from_file(&state, &tool).is_err());
    }
}

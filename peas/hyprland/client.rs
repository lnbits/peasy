//! hyprland pea: unprivileged discovery, review and execution.
use crate::{
    LocalAction, LocalProposal, LocalResult, PeasyClient, Resolution, runtime_desktop_kind,
    safe_stderr,
};
use anyhow::{Context, Result, bail};
use peasy_core::{
    DesktopEnvironment as DesktopKind, DiffKind, DiffLine, HyprlandDispatch, HyprlandSettingChange,
};
use serde_json::Value;
use std::process::Command;

pub(super) fn hyprland_session_available() -> bool {
    runtime_desktop_kind() == DesktopKind::Hyprland
}

pub(super) fn safe_hyprland_text(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(160)
        .collect()
}

pub(super) fn ensure_hyprland_success(output: std::process::Output, action: &str) -> Result<()> {
    if !output.status.success() {
        bail!(
            "Hyprland could not {action}: {}",
            safe_stderr(&output.stderr)
        );
    }
    let reply = String::from_utf8_lossy(&output.stdout);
    if !reply.trim().is_empty() && !reply.trim().eq_ignore_ascii_case("ok") {
        bail!(
            "Hyprland could not {action}: {}",
            safe_hyprland_text(&reply)
        );
    }
    Ok(())
}

pub(super) fn hyprland_dispatch_description(
    dispatch: HyprlandDispatch,
    argument: Option<&str>,
) -> String {
    match dispatch {
        HyprlandDispatch::SwitchWorkspace => {
            format!("Switch to Hyprland workspace {}", argument.unwrap_or("?"))
        }
        HyprlandDispatch::MoveWindowToWorkspace => format!(
            "Move the active window to Hyprland workspace {}",
            argument.unwrap_or("?")
        ),
        HyprlandDispatch::FocusDirection => {
            format!("Move Hyprland focus {}", argument.unwrap_or("?"))
        }
        HyprlandDispatch::ToggleFloating => "Toggle floating for the active window".into(),
        HyprlandDispatch::ToggleFullscreen => "Toggle fullscreen for the active window".into(),
    }
}

pub(super) fn modern_hyprland_dispatch(
    dispatch: HyprlandDispatch,
    argument: Option<&str>,
) -> String {
    match dispatch {
        HyprlandDispatch::SwitchWorkspace => format!(
            "hl.dispatch(hl.dsp.focus({{ workspace = \"{}\" }}))",
            argument.unwrap_or("1")
        ),
        HyprlandDispatch::MoveWindowToWorkspace => format!(
            "hl.dispatch(hl.dsp.window.move({{ workspace = \"{}\" }}))",
            argument.unwrap_or("1")
        ),
        HyprlandDispatch::FocusDirection => format!(
            "hl.dispatch(hl.dsp.focus({{ direction = \"{}\" }}))",
            argument.unwrap_or("l")
        ),
        HyprlandDispatch::ToggleFloating => {
            "hl.dispatch(hl.dsp.window.float({ action = \"toggle\" }))".into()
        }
        HyprlandDispatch::ToggleFullscreen => {
            "hl.dispatch(hl.dsp.window.fullscreen({ action = \"toggle\", mode = \"fullscreen\" }))"
                .into()
        }
    }
}

pub(super) fn legacy_hyprland_dispatch(
    dispatch: HyprlandDispatch,
    argument: Option<&str>,
) -> (&'static str, String) {
    match dispatch {
        HyprlandDispatch::SwitchWorkspace => ("workspace", argument.unwrap_or("1").into()),
        HyprlandDispatch::MoveWindowToWorkspace => {
            ("movetoworkspace", argument.unwrap_or("1").into())
        }
        HyprlandDispatch::FocusDirection => ("movefocus", argument.unwrap_or("l").into()),
        HyprlandDispatch::ToggleFloating => ("togglefloating", "active".into()),
        HyprlandDispatch::ToggleFullscreen => ("fullscreen", "0".into()),
    }
}

impl PeasyClient {
    pub(super) fn hyprland_status(&self) -> Result<Resolution> {
        let version = self.hyprland_json("version")?;
        let workspace = self.hyprland_json("activeworkspace")?;
        let window = self.hyprland_json("activewindow")?;
        let monitors = self.hyprland_json("monitors")?;
        let version = version
            .get("tag")
            .or_else(|| version.get("version"))
            .and_then(Value::as_str)
            .map(safe_hyprland_text)
            .unwrap_or_else(|| "unknown version".into());
        let workspace = workspace
            .get("name")
            .and_then(Value::as_str)
            .map(safe_hyprland_text)
            .or_else(|| {
                workspace
                    .get("id")
                    .and_then(Value::as_i64)
                    .map(|id| id.to_string())
            })
            .unwrap_or_else(|| "unknown".into());
        let active_window = window
            .get("title")
            .and_then(Value::as_str)
            .filter(|title| !title.is_empty())
            .or_else(|| window.get("class").and_then(Value::as_str))
            .map(safe_hyprland_text)
            .unwrap_or_else(|| "none".into());
        let monitor_names = monitors
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|monitor| monitor.get("name").and_then(Value::as_str))
            .map(safe_hyprland_text)
            .take(8)
            .collect::<Vec<_>>();
        Ok(Resolution::Explain(format!(
            "Hyprland {version}\n• Active workspace: {workspace}\n• Active window: {active_window}\n• Monitors: {}",
            if monitor_names.is_empty() {
                "none reported".into()
            } else {
                monitor_names.join(", ")
            }
        )))
    }

    pub(super) fn hyprland_json(&self, command: &str) -> Result<Value> {
        let output = Command::new(&self.tools.hyprctl)
            .args(["-j", command])
            .output()
            .with_context(|| format!("querying Hyprland {command}"))?;
        if !output.status.success() {
            bail!(
                "Hyprland is not available in this desktop session: {}",
                safe_stderr(&output.stderr)
            );
        }
        serde_json::from_slice(&output.stdout)
            .with_context(|| format!("Hyprland returned invalid {command} data"))
    }

    pub(super) fn propose_hyprland_setting(
        &self,
        mut change: HyprlandSettingChange,
    ) -> Result<Resolution> {
        change.value = change.setting.normalize_value(&change.value)?;
        // A read proves this process is in a live Hyprland session and that the
        // selected, fixed option exists in the compositor actually in use.
        let current = self.hyprland_option(&change)?;
        Ok(Resolution::LocalProposal(LocalProposal {
            title: format!("Change Hyprland {}", change.setting),
            diff: vec![
                DiffLine {
                    kind: DiffKind::Remove,
                    text: format!("{} = {current}", change.setting),
                },
                DiffLine {
                    kind: DiffKind::Add,
                    text: format!("{} = {}", change.setting, change.value),
                },
                DiffLine {
                    kind: DiffKind::Context,
                    text:
                        "Live session change; Hyprland config reload restores the configured value."
                            .into(),
                },
            ],
            action: LocalAction::HyprlandSetting { change },
        }))
    }

    pub(super) fn hyprland_option(&self, change: &HyprlandSettingChange) -> Result<String> {
        for option in [
            change.setting.option_path().to_owned(),
            change.setting.legacy_option_path(),
        ] {
            let output = Command::new(&self.tools.hyprctl)
                .args(["-j", "getoption", &option])
                .output()
                .context("reading the current Hyprland setting")?;
            if output.status.success() {
                let value: Value = serde_json::from_slice(&output.stdout)
                    .context("Hyprland returned invalid option data")?;
                for key in ["current", "value", "str", "int", "float"] {
                    if let Some(current) = value.get(key) {
                        return Ok(safe_hyprland_text(&current.to_string()));
                    }
                }
                return Ok("current value".into());
            }
        }
        bail!(
            "this Hyprland version does not expose `{}`",
            change.setting.option_path()
        )
    }

    pub(super) fn propose_hyprland_dispatch(
        &self,
        dispatch: HyprlandDispatch,
        argument: Option<String>,
    ) -> Result<Resolution> {
        let argument = dispatch.normalize_argument(argument.as_deref())?;
        // A lightweight query prevents presenting a control proposal in a
        // non-Hyprland session.
        let _ = self.hyprland_json("version")?;
        let description = hyprland_dispatch_description(dispatch, argument.as_deref());
        Ok(Resolution::LocalProposal(LocalProposal {
            title: description.clone(),
            diff: vec![DiffLine {
                kind: DiffKind::Add,
                text: description,
            }],
            action: LocalAction::HyprlandDispatch { dispatch, argument },
        }))
    }

    pub(super) fn hyprland_uses_lua(&self) -> Result<bool> {
        let output = Command::new(&self.tools.hyprctl)
            .arg("--help")
            .output()
            .context("checking the installed hyprctl interface")?;
        if !output.status.success() {
            bail!("hyprctl could not report its supported interface");
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|line| line.trim_start().starts_with("eval ")))
    }

    pub(super) fn apply_hyprland_setting(
        &self,
        change: &HyprlandSettingChange,
    ) -> Result<LocalResult> {
        let normalized = change.setting.normalize_value(&change.value)?;
        let modern = self.hyprland_uses_lua()?;
        let output = if modern {
            let expression = format!(
                "hl.config({{ [\"{}\"] = {} }})",
                change.setting.option_path(),
                change.setting.lua_value(&normalized)
            );
            Command::new(&self.tools.hyprctl)
                .args(["eval", &expression])
                .output()
                .context("applying the Hyprland setting")?
        } else {
            Command::new(&self.tools.hyprctl)
                .args(["keyword", &change.setting.legacy_option_path(), &normalized])
                .output()
                .context("applying the Hyprland setting")?
        };
        ensure_hyprland_success(output, "change the setting")?;
        Ok(LocalResult {
            completed: true,
            message: format!("Hyprland {} changed for this live session.", change.setting),
        })
    }

    pub(super) fn apply_hyprland_dispatch(
        &self,
        dispatch: &HyprlandDispatch,
        argument: &Option<String>,
    ) -> Result<LocalResult> {
        let argument = dispatch.normalize_argument(argument.as_deref())?;
        let modern = self.hyprland_uses_lua()?;
        let output = if modern {
            let expression = modern_hyprland_dispatch(*dispatch, argument.as_deref());
            Command::new(&self.tools.hyprctl)
                .args(["eval", &expression])
                .output()
                .context("controlling Hyprland")?
        } else {
            let (name, value) = legacy_hyprland_dispatch(*dispatch, argument.as_deref());
            Command::new(&self.tools.hyprctl)
                .args(["dispatch", name, &value])
                .output()
                .context("controlling Hyprland")?
        };
        ensure_hyprland_success(output, "perform the requested action")?;
        Ok(LocalResult {
            completed: true,
            message: hyprland_dispatch_description(*dispatch, argument.as_deref()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hyprland_commands_are_generated_only_from_typed_values() {
        assert_eq!(
            modern_hyprland_dispatch(HyprlandDispatch::SwitchWorkspace, Some("3")),
            "hl.dispatch(hl.dsp.focus({ workspace = \"3\" }))"
        );
        assert_eq!(
            modern_hyprland_dispatch(HyprlandDispatch::ToggleFloating, None),
            "hl.dispatch(hl.dsp.window.float({ action = \"toggle\" }))"
        );
        assert_eq!(
            legacy_hyprland_dispatch(HyprlandDispatch::MoveWindowToWorkspace, Some("4")),
            ("movetoworkspace", "4".into())
        );
    }
}

//! Trusted adapters for the existing generic, closed ThemeSettings action.
use crate::{apply_live_theme_with, safe_stderr};
use anyhow::{Context, Result, bail};
use peasy_core::{AccentColor, ColorScheme, DesktopEnvironment, ThemeSettings};
use std::path::Path;
use std::process::Command;

#[derive(Debug, PartialEq)]
enum Adapter {
    Gnome,
    Plasma(Vec<PlasmaCommand>),
    NoChange,
}

#[derive(Debug, PartialEq)]
enum PlasmaCommand {
    ColorScheme(Vec<&'static str>),
    WriteAccent(&'static str),
}

fn adapter(desktop: DesktopEnvironment, theme: &ThemeSettings) -> Result<Adapter> {
    desktop.validate_appearance(theme)?;
    if theme.is_empty() {
        return Ok(Adapter::NoChange);
    }
    match desktop {
        DesktopEnvironment::Gnome => Ok(Adapter::Gnome),
        DesktopEnvironment::KdePlasma => {
            let mut calls = Vec::new();
            if let Some(scheme) = theme.color_scheme {
                calls.push(PlasmaCommand::ColorScheme(vec![match scheme {
                    ColorScheme::Light => "BreezeLight",
                    ColorScheme::Dark => "BreezeDark",
                    ColorScheme::System => {
                        unreachable!("capability validation rejects system default")
                    }
                }]));
            }
            if let Some(accent) = theme.accent_color {
                calls.push(PlasmaCommand::WriteAccent(match accent {
                    AccentColor::Blue => "53,132,228",
                    AccentColor::Teal => "33,144,164",
                    AccentColor::Green => "58,148,74",
                    AccentColor::Yellow => "200,136,0",
                    AccentColor::Orange => "237,91,0",
                    AccentColor::Red => "230,45,66",
                    AccentColor::Pink => "213,97,153",
                    AccentColor::Purple => "145,65,172",
                    AccentColor::Slate => "111,131,150",
                }));
                calls.push(PlasmaCommand::ColorScheme(vec![
                    "--accent-color",
                    match accent {
                        AccentColor::Blue => "#3584e4",
                        AccentColor::Teal => "#2190a4",
                        AccentColor::Green => "#3a944a",
                        AccentColor::Yellow => "#c88800",
                        AccentColor::Orange => "#ed5b00",
                        AccentColor::Red => "#e62d42",
                        AccentColor::Pink => "#d56199",
                        AccentColor::Purple => "#9141ac",
                        AccentColor::Slate => "#6f8396",
                    },
                ]));
            }
            // Plasma's CLI treats accent and scheme as mutually exclusive modes.
            // Separate fixed calls are required when changing both.
            Ok(Adapter::Plasma(calls))
        }
        _ => unreachable!("capability validation rejects unsupported nonempty themes"),
    }
}

pub fn apply(
    desktop: DesktopEnvironment,
    theme: &ThemeSettings,
    gsettings: &Path,
    plasma: &Path,
    kconfig: &Path,
) -> Result<()> {
    match adapter(desktop, theme)? {
        Adapter::Gnome => apply_live_theme_with(gsettings, theme),
        Adapter::Plasma(calls) => {
            for call in calls {
                let (program, args) = match call {
                    PlasmaCommand::ColorScheme(args) => (plasma, args),
                    // Upstream's accent CLI recolours the palette but does not
                    // persist General/AccentColor. Save only this fixed key;
                    // otherwise the next native scheme change loses the accent.
                    PlasmaCommand::WriteAccent(rgb) => (
                        kconfig,
                        vec![
                            "--file",
                            "kdeglobals",
                            "--group",
                            "General",
                            "--key",
                            "AccentColor",
                            rgb,
                        ],
                    ),
                };
                let output = Command::new(program)
                    .args(args)
                    .output()
                    .context("applying KDE Plasma appearance with fixed desktop tools")?;
                if !output.status.success() {
                    bail!(
                        "KDE Plasma could not apply appearance: {}",
                        safe_stderr(&output.stderr)
                    );
                }
            }
            Ok(())
        }
        Adapter::NoChange => Ok(()),
    }
}

pub fn choices(desktop: DesktopEnvironment) -> String {
    let capabilities = desktop.capabilities();
    if !capabilities.light_dark {
        return if capabilities.hyprland_controls {
            "Hyprland: Peasy supports reviewed live gaps, borders, corner radius, opacity, blur and animation controls. Generic colour schemes, accent colours and wallpapers are not supported yet.".into()
        } else {
            format!(
                "UnsupportedCapability: Peasy has no appearance adapter for {desktop}. Package, calendar and other supported actions remain available."
            )
        };
    }
    format!(
        "{desktop} appearance choices:\n• Accent colours: blue, teal, green, yellow, orange, red, pink, purple, slate\n• Modes: light, dark{}\n{}\nWallpaper changes are not supported yet.",
        if capabilities.system_default {
            ", system default"
        } else {
            ""
        },
        if desktop == DesktopEnvironment::KdePlasma {
            "Light/dark selects the installed BreezeLight/BreezeDark colour scheme."
        } else {
            ""
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_appearance_routes_without_cross_desktop_fallbacks() {
        let theme = ThemeSettings {
            accent_color: Some(AccentColor::Purple),
            color_scheme: Some(ColorScheme::Dark),
        };
        assert_eq!(
            adapter(DesktopEnvironment::Gnome, &theme).unwrap(),
            Adapter::Gnome
        );
        assert_eq!(
            adapter(DesktopEnvironment::KdePlasma, &theme).unwrap(),
            Adapter::Plasma(vec![
                PlasmaCommand::ColorScheme(vec!["BreezeDark"]),
                PlasmaCommand::WriteAccent("145,65,172"),
                PlasmaCommand::ColorScheme(vec!["--accent-color", "#9141ac"]),
            ])
        );
        for desktop in [
            DesktopEnvironment::Hyprland,
            DesktopEnvironment::Other,
            DesktopEnvironment::Headless,
        ] {
            assert!(
                adapter(desktop, &theme)
                    .unwrap_err()
                    .to_string()
                    .contains("UnsupportedCapability")
            );
            assert_eq!(
                adapter(desktop, &ThemeSettings::default()).unwrap(),
                Adapter::NoChange
            );
        }
        assert!(!choices(DesktopEnvironment::KdePlasma).contains("system default"));
    }

    #[test]
    fn plasma_never_invokes_gnome_or_accepts_arbitrary_values() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let tool = temp.path().join("plasma");
        let log = temp.path().join("args");
        fs::write(
            &tool,
            format!("#!/bin/sh\nprintf '%s\\n' \"$@\" >> '{}'\n", log.display()),
        )
        .unwrap();
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o700)).unwrap();
        let theme = ThemeSettings {
            accent_color: Some(AccentColor::Blue),
            color_scheme: Some(ColorScheme::Light),
        };
        apply(
            DesktopEnvironment::KdePlasma,
            &theme,
            Path::new("/nonexistent-gsettings"),
            &tool,
            &tool,
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(log).unwrap(),
            "BreezeLight\n--file\nkdeglobals\n--group\nGeneral\n--key\nAccentColor\n53,132,228\n--accent-color\n#3584e4\n"
        );
        assert!(
            serde_json::from_str::<ThemeSettings>(
                r#"{"accent_color":"blue;reboot","color_scheme":"dark"}"#
            )
            .is_err()
        );
        assert!(
            serde_json::from_str::<ThemeSettings>(
                r#"{"accent_color":"blue","file":"/etc/passwd"}"#
            )
            .is_err()
        );
    }
}

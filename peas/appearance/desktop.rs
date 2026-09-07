//! Bounded session data and capabilities, never arbitrary environment data.
use crate::{ColorScheme, ThemeSettings, ValidationError};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopEnvironment {
    Gnome,
    KdePlasma,
    Hyprland,
    Xfce,
    Lxqt,
    Other,
    Headless,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct AppearanceCapabilities {
    pub light_dark: bool,
    pub system_default: bool,
    pub accent_color: bool,
    pub wallpaper: bool,
    pub hyprland_controls: bool,
}

impl DesktopEnvironment {
    pub fn detect(values: [Option<&str>; 3], hyprland: bool, graphical: bool) -> Self {
        for value in values.into_iter().flatten() {
            if value.len() > 256 || value.chars().any(char::is_control) {
                continue;
            }
            for token in value.split(':') {
                let known = match token.to_ascii_lowercase().as_str() {
                    "gnome" | "gnome-classic" | "gnome-wayland" | "gnome-xorg" => Self::Gnome,
                    "kde" | "plasma" | "plasmawayland" | "plasmax11" | "plasma6" => Self::KdePlasma,
                    "hyprland" => Self::Hyprland,
                    "xfce" | "xfce4" => Self::Xfce,
                    "lxqt" => Self::Lxqt,
                    _ => continue,
                };
                return known;
            }
        }
        if hyprland {
            Self::Hyprland
        } else if graphical {
            Self::Other
        } else {
            Self::Headless
        }
    }

    pub fn capabilities(self) -> AppearanceCapabilities {
        AppearanceCapabilities {
            light_dark: matches!(self, Self::Gnome | Self::KdePlasma),
            system_default: self == Self::Gnome,
            accent_color: matches!(self, Self::Gnome | Self::KdePlasma),
            wallpaper: false, // No validated wallpaper catalogue is exposed yet.
            hyprland_controls: self == Self::Hyprland,
        }
    }

    pub fn validate_appearance(self, theme: &ThemeSettings) -> Result<(), ValidationError> {
        let capabilities = self.capabilities();
        if (theme.accent_color.is_some() && !capabilities.accent_color)
            || (theme.color_scheme.is_some() && !capabilities.light_dark)
            || (theme.color_scheme == Some(ColorScheme::System) && !capabilities.system_default)
        {
            return Err(ValidationError::UnsupportedCapability(format!(
                "this appearance change is not supported on {self}; ask for available appearance choices"
            )));
        }
        Ok(())
    }
}

impl fmt::Display for DesktopEnvironment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Gnome => "GNOME",
            Self::KdePlasma => "KDE Plasma",
            Self::Hyprland => "Hyprland",
            Self::Xfce => "XFCE",
            Self::Lxqt => "LXQt",
            Self::Other => "this desktop",
            Self::Headless => "a headless session",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection_is_bounded_normalized_and_uses_session_fallbacks() {
        for (name, expected) in [
            ("ubuntu:GNOME", DesktopEnvironment::Gnome),
            ("GNOME-Classic", DesktopEnvironment::Gnome),
            ("KDE", DesktopEnvironment::KdePlasma),
            ("plasmawayland", DesktopEnvironment::KdePlasma),
            ("Hyprland", DesktopEnvironment::Hyprland),
            ("XFCE", DesktopEnvironment::Xfce),
            ("LXQt", DesktopEnvironment::Lxqt),
            ("gnome;reboot", DesktopEnvironment::Other),
            ("unknown", DesktopEnvironment::Other),
        ] {
            for values in [
                [Some(name), None, None],
                [None, Some(name), None],
                [None, None, Some(name)],
            ] {
                assert_eq!(DesktopEnvironment::detect(values, false, true), expected);
            }
        }
        assert_eq!(
            DesktopEnvironment::detect([Some("GNOME\nsecret"), None, None], false, true),
            DesktopEnvironment::Other
        );
        assert_eq!(
            DesktopEnvironment::detect([Some(&"g".repeat(257)), None, None], false, true),
            DesktopEnvironment::Other
        );
        assert_eq!(
            DesktopEnvironment::detect([None; 3], false, false),
            DesktopEnvironment::Headless
        );
        assert_eq!(
            DesktopEnvironment::detect([None; 3], true, true),
            DesktopEnvironment::Hyprland
        );
        // An explicit current desktop wins over a stale compositor signature.
        assert_eq!(
            DesktopEnvironment::detect([Some("KDE"), None, None], true, true),
            DesktopEnvironment::KdePlasma
        );
    }

    #[test]
    fn unsupported_appearance_is_explicit() {
        let mut theme = ThemeSettings {
            color_scheme: Some(ColorScheme::Dark),
            accent_color: None,
        };
        assert!(
            DesktopEnvironment::Gnome
                .validate_appearance(&theme)
                .is_ok()
        );
        assert!(
            DesktopEnvironment::KdePlasma
                .validate_appearance(&theme)
                .is_ok()
        );
        for desktop in [
            DesktopEnvironment::Hyprland,
            DesktopEnvironment::Xfce,
            DesktopEnvironment::Lxqt,
            DesktopEnvironment::Other,
            DesktopEnvironment::Headless,
        ] {
            assert!(matches!(
                desktop.validate_appearance(&theme),
                Err(ValidationError::UnsupportedCapability(_))
            ));
        }
        theme.color_scheme = Some(ColorScheme::System);
        assert!(
            DesktopEnvironment::Gnome
                .validate_appearance(&theme)
                .is_ok()
        );
        assert!(
            DesktopEnvironment::KdePlasma
                .validate_appearance(&theme)
                .is_err()
        );
    }
}

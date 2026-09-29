//! hyprland pea: existing closed types and validation.
use crate::ValidationError;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HyprlandSetting {
    GapsInner,
    GapsOuter,
    BorderSize,
    CornerRadius,
    Animations,
    Blur,
    ActiveOpacity,
    InactiveOpacity,
    NaturalScroll,
    Layout,
}

impl HyprlandSetting {
    pub fn parse(value: &str) -> Result<Self, ValidationError> {
        match value {
            "gaps_inner" => Ok(Self::GapsInner),
            "gaps_outer" => Ok(Self::GapsOuter),
            "border_size" => Ok(Self::BorderSize),
            "corner_radius" => Ok(Self::CornerRadius),
            "animations" => Ok(Self::Animations),
            "blur" => Ok(Self::Blur),
            "active_opacity" => Ok(Self::ActiveOpacity),
            "inactive_opacity" => Ok(Self::InactiveOpacity),
            "natural_scroll" => Ok(Self::NaturalScroll),
            "layout" => Ok(Self::Layout),
            _ => Err(ValidationError::InvalidRequest(format!(
                "unsupported Hyprland setting `{value}`"
            ))),
        }
    }

    pub fn option_path(self) -> &'static str {
        match self {
            Self::GapsInner => "general.gaps_in",
            Self::GapsOuter => "general.gaps_out",
            Self::BorderSize => "general.border_size",
            Self::CornerRadius => "decoration.rounding",
            Self::Animations => "animations.enabled",
            Self::Blur => "decoration.blur.enabled",
            Self::ActiveOpacity => "decoration.active_opacity",
            Self::InactiveOpacity => "decoration.inactive_opacity",
            Self::NaturalScroll => "input.touchpad.natural_scroll",
            Self::Layout => "general.layout",
        }
    }

    pub fn legacy_option_path(self) -> String {
        self.option_path().replace('.', ":")
    }

    pub fn normalize_value(self, value: &str) -> Result<String, ValidationError> {
        let value = value.trim().to_ascii_lowercase();
        match self {
            Self::GapsInner | Self::GapsOuter => normalize_integer(&value, 0, 100),
            Self::BorderSize => normalize_integer(&value, 0, 20),
            Self::CornerRadius => normalize_integer(&value, 0, 100),
            Self::Animations | Self::Blur | Self::NaturalScroll => match value.as_str() {
                "true" | "on" | "enabled" | "enable" => Ok("true".into()),
                "false" | "off" | "disabled" | "disable" => Ok("false".into()),
                _ => Err(ValidationError::InvalidRequest(format!(
                    "{} expects on or off",
                    self.option_path()
                ))),
            },
            Self::ActiveOpacity | Self::InactiveOpacity => {
                let number: f64 = value.parse().map_err(|_| {
                    ValidationError::InvalidRequest(format!(
                        "{} expects a number from 0 to 1",
                        self.option_path()
                    ))
                })?;
                if !number.is_finite() || !(0.0..=1.0).contains(&number) {
                    return Err(ValidationError::InvalidRequest(format!(
                        "{} expects a number from 0 to 1",
                        self.option_path()
                    )));
                }
                let rendered = format!("{number:.3}");
                Ok(rendered
                    .trim_end_matches('0')
                    .trim_end_matches('.')
                    .to_owned())
            }
            Self::Layout => match value.as_str() {
                "dwindle" | "master" | "scrolling" | "monocle" => Ok(value),
                _ => Err(ValidationError::InvalidRequest(
                    "Hyprland layout must be dwindle, master, scrolling, or monocle".into(),
                )),
            },
        }
    }

    pub fn lua_value(self, normalized: &str) -> String {
        if matches!(self, Self::Layout) {
            format!("\"{normalized}\"")
        } else {
            normalized.to_owned()
        }
    }
}

impl fmt::Display for HyprlandSetting {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.option_path())
    }
}

fn normalize_integer(value: &str, minimum: i32, maximum: i32) -> Result<String, ValidationError> {
    let number: i32 = value.parse().map_err(|_| {
        ValidationError::InvalidRequest(format!(
            "Hyprland setting expects an integer from {minimum} to {maximum}"
        ))
    })?;
    if !(minimum..=maximum).contains(&number) {
        return Err(ValidationError::InvalidRequest(format!(
            "Hyprland setting expects an integer from {minimum} to {maximum}"
        )));
    }
    Ok(number.to_string())
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HyprlandSettingChange {
    pub setting: HyprlandSetting,
    pub value: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HyprlandDispatch {
    SwitchWorkspace,
    MoveWindowToWorkspace,
    FocusDirection,
    ToggleFloating,
    ToggleFullscreen,
}

impl HyprlandDispatch {
    pub fn parse(value: &str) -> Result<Self, ValidationError> {
        match value {
            "switch_workspace" => Ok(Self::SwitchWorkspace),
            "move_window_to_workspace" => Ok(Self::MoveWindowToWorkspace),
            "focus_direction" => Ok(Self::FocusDirection),
            "toggle_floating" => Ok(Self::ToggleFloating),
            "toggle_fullscreen" => Ok(Self::ToggleFullscreen),
            _ => Err(ValidationError::InvalidRequest(format!(
                "unsupported Hyprland action `{value}`"
            ))),
        }
    }

    pub fn normalize_argument(
        self,
        argument: Option<&str>,
    ) -> Result<Option<String>, ValidationError> {
        match self {
            Self::SwitchWorkspace | Self::MoveWindowToWorkspace => {
                let workspace: u8 = argument
                    .ok_or_else(|| {
                        ValidationError::InvalidRequest("workspace number is required".into())
                    })?
                    .parse()
                    .map_err(|_| {
                        ValidationError::InvalidRequest(
                            "workspace must be a number from 1 to 99".into(),
                        )
                    })?;
                if !(1..=99).contains(&workspace) {
                    return Err(ValidationError::InvalidRequest(
                        "workspace must be a number from 1 to 99".into(),
                    ));
                }
                Ok(Some(workspace.to_string()))
            }
            Self::FocusDirection => {
                let direction = match argument.unwrap_or_default().to_ascii_lowercase().as_str() {
                    "left" | "l" => "l",
                    "right" | "r" => "r",
                    "up" | "u" => "u",
                    "down" | "d" => "d",
                    _ => {
                        return Err(ValidationError::InvalidRequest(
                            "focus direction must be left, right, up, or down".into(),
                        ));
                    }
                };
                Ok(Some(direction.into()))
            }
            Self::ToggleFloating | Self::ToggleFullscreen => Ok(None),
        }
    }
}

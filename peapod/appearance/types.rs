//! appearance pea: existing closed types and validation.
use crate::ValidationError;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccentColor {
    Blue,
    Teal,
    Green,
    Yellow,
    Orange,
    Red,
    Pink,
    Purple,
    Slate,
}

impl AccentColor {
    pub fn parse(value: &str) -> Result<Self, ValidationError> {
        match value {
            "blue" => Ok(Self::Blue),
            "teal" => Ok(Self::Teal),
            "green" => Ok(Self::Green),
            "yellow" => Ok(Self::Yellow),
            "orange" => Ok(Self::Orange),
            "red" => Ok(Self::Red),
            "pink" => Ok(Self::Pink),
            "purple" => Ok(Self::Purple),
            "slate" => Ok(Self::Slate),
            _ => Err(ValidationError::InvalidRequest(format!(
                "unsupported accent colour `{value}`"
            ))),
        }
    }
}

impl fmt::Display for AccentColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Blue => "blue",
            Self::Teal => "teal",
            Self::Green => "green",
            Self::Yellow => "yellow",
            Self::Orange => "orange",
            Self::Red => "red",
            Self::Pink => "pink",
            Self::Purple => "purple",
            Self::Slate => "slate",
        })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorScheme {
    System,
    Light,
    Dark,
}

impl ColorScheme {
    pub fn parse(value: &str) -> Result<Self, ValidationError> {
        match value {
            "system" => Ok(Self::System),
            "light" => Ok(Self::Light),
            "dark" => Ok(Self::Dark),
            _ => Err(ValidationError::InvalidRequest(format!(
                "unsupported colour scheme `{value}`"
            ))),
        }
    }

    pub fn gsettings_value(self) -> &'static str {
        match self {
            Self::System => "default",
            Self::Light => "prefer-light",
            Self::Dark => "prefer-dark",
        }
    }
}

impl fmt::Display for ColorScheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        })
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeSettings {
    pub accent_color: Option<AccentColor>,
    pub color_scheme: Option<ColorScheme>,
}

impl ThemeSettings {
    pub fn is_empty(&self) -> bool {
        self.accent_color.is_none() && self.color_scheme.is_none()
    }

    pub fn merged(&self, change: &Self) -> Self {
        Self {
            accent_color: change.accent_color.or(self.accent_color),
            color_scheme: change.color_scheme.or(self.color_scheme),
        }
    }
}

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;
use thiserror::Error;

mod desktop;
pub use desktop::{AppearanceCapabilities, DesktopEnvironment};

pub const MAX_QUERY_BYTES: usize = 160;
pub const MAX_ATTRIBUTE_BYTES: usize = 180;
pub const MAX_CANDIDATES: usize = 12;
pub const MAX_SSID_BYTES: usize = 32;
pub const MAX_EVENT_TITLE_BYTES: usize = 160;
pub const LOCAL_DATETIME_BYTES: usize = 19;
pub const MAX_APPIMAGE_BYTES: u64 = 1024 * 1024 * 1024;
pub const APPIMAGE_POLICY_PATH: &str = "/etc/peasy/appimage-policy.json";

/// `null` permits reviewed installs; a map enforces administrator-approved hashes.
/// Missing policy files fail closed, including during mixed-version upgrades.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(transparent)]
pub struct AppImagePolicy(pub Option<std::collections::BTreeMap<String, Vec<String>>>);

impl Default for AppImagePolicy {
    fn default() -> Self {
        Self(Some(Default::default()))
    }
}

impl AppImagePolicy {
    pub fn load(path: &Path) -> Result<Self, std::io::Error> {
        use std::io::Read;
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error),
        };
        let mut bytes = Vec::new();
        file.take(65537).read_to_end(&mut bytes)?;
        if bytes.len() > 65536 {
            return Err(std::io::Error::other("AppImage policy is too large"));
        }
        serde_json::from_slice(&bytes).map_err(std::io::Error::other)
    }

    pub fn allows_repository(&self, repository: &str) -> bool {
        self.0.as_ref().is_none_or(|trusted| {
            trusted
                .get(&repository.to_ascii_lowercase())
                .is_some_and(|hashes| !hashes.is_empty())
        })
    }

    pub fn is_disabled(&self) -> bool {
        self.0.as_ref().is_some_and(|trusted| trusted.is_empty())
    }

    pub fn allows(&self, package: &AppImagePackage) -> bool {
        package.validate().is_ok()
            && self.0.as_ref().is_none_or(|trusted| {
                trusted
                    .get(&package.repository.to_ascii_lowercase())
                    .is_some_and(|hashes| hashes.contains(&package.hash))
            })
    }
}
const MANAGED_STATE_PREFIX: &str = "# peasy-state-json: ";

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("UnsupportedCapability: {0}")]
    UnsupportedCapability(String),
    #[error("the value is empty")]
    Empty,
    #[error("the value is too long")]
    TooLong,
    #[error("invalid package attribute `{0}`")]
    InvalidAttribute(String),
    #[error("invalid request: {0}")]
    InvalidRequest(String),
}

pub fn validate_query(value: &str) -> Result<&str, ValidationError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ValidationError::Empty);
    }
    if value.len() > MAX_QUERY_BYTES || value.chars().any(char::is_control) {
        return Err(ValidationError::TooLong);
    }
    Ok(value)
}

fn normalize_model_message(mut value: String) -> Result<String, ValidationError> {
    if value
        .chars()
        .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\t'))
    {
        return Err(ValidationError::InvalidRequest(
            "agent response contains invalid control characters".into(),
        ));
    }
    if value.len() > 400 {
        let mut end = 400;
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        value.truncate(end);
    }
    Ok(value)
}

fn validate_github_repository(value: &str) -> Result<String, ValidationError> {
    let value = value.trim().trim_end_matches(".git");
    let Some((owner, repository)) = value.split_once('/') else {
        return Err(ValidationError::InvalidRequest(
            "GitHub repository must be owner/name".into(),
        ));
    };
    let valid_part = |part: &str| {
        !part.is_empty()
            && part.len() <= 100
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    };
    if repository.contains('/') || !valid_part(owner) || !valid_part(repository) {
        return Err(ValidationError::InvalidRequest(
            "invalid GitHub repository".into(),
        ));
    }
    Ok(format!(
        "{}/{}",
        owner.to_ascii_lowercase(),
        repository.to_ascii_lowercase()
    ))
}

pub fn validate_attribute(value: &str) -> Result<&str, ValidationError> {
    if value.is_empty() {
        return Err(ValidationError::Empty);
    }
    if value.len() > MAX_ATTRIBUTE_BYTES {
        return Err(ValidationError::TooLong);
    }
    let valid = value.split('.').all(|part| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'+' | b'-'))
    });
    if !valid {
        return Err(ValidationError::InvalidAttribute(value.to_owned()));
    }
    Ok(value)
}

pub fn validate_ssid(value: &str) -> Result<&str, ValidationError> {
    if value.is_empty() {
        return Err(ValidationError::Empty);
    }
    if value.len() > MAX_SSID_BYTES || value.chars().any(char::is_control) {
        return Err(ValidationError::InvalidRequest(
            "invalid Wi-Fi network name".into(),
        ));
    }
    Ok(value)
}

pub fn validate_event_title(value: &str) -> Result<&str, ValidationError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ValidationError::Empty);
    }
    if value.len() > MAX_EVENT_TITLE_BYTES || value.chars().any(char::is_control) {
        return Err(ValidationError::InvalidRequest(
            "invalid calendar event title".into(),
        ));
    }
    Ok(value)
}

pub fn validate_local_datetime(value: &str) -> Result<&str, ValidationError> {
    let bytes = value.as_bytes();
    if bytes.len() != LOCAL_DATETIME_BYTES
        || !bytes.iter().enumerate().all(|(index, byte)| match index {
            4 | 7 => *byte == b'-',
            10 => *byte == b'T',
            13 | 16 => *byte == b':',
            _ => byte.is_ascii_digit(),
        })
    {
        return Err(ValidationError::InvalidRequest(
            "calendar start must be a local YYYY-MM-DDTHH:MM:SS value".into(),
        ));
    }
    let number = |range: std::ops::Range<usize>| {
        value[range]
            .parse::<u32>()
            .map_err(|_| ValidationError::InvalidRequest("invalid calendar date".into()))
    };
    let year = number(0..4)?;
    let month = number(5..7)?;
    let day = number(8..10)?;
    let hour = number(11..13)?;
    let minute = number(14..16)?;
    let second = number(17..19)?;
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => 0,
    };
    if day == 0 || day > max_day || hour > 23 || minute > 59 || second > 59 {
        return Err(ValidationError::InvalidRequest(
            "calendar start is not a real local date and time".into(),
        ));
    }
    Ok(value)
}

/// Escape a string for use as a literal Nix search regular expression.
pub fn regex_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(
            ch,
            '.' | '+' | '*' | '?' | '(' | ')' | '|' | '[' | ']' | '{' | '}' | '^' | '$' | '\\'
        ) {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PackageCandidate {
    pub attribute: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub version: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestedVersion {
    Latest,
    Exact(String),
}

impl RequestedVersion {
    pub fn parse(value: &str) -> Result<Self, ValidationError> {
        let value = value.trim();
        if value.eq_ignore_ascii_case("latest") {
            return Ok(Self::Latest);
        }
        if value.is_empty()
            || value.len() > 64
            || !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'+' | b'-' | b'_')
            })
        {
            return Err(ValidationError::InvalidRequest(
                "invalid requested package version".into(),
            ));
        }
        Ok(Self::Exact(value.to_owned()))
    }

    pub fn matches(&self, candidate: &str) -> bool {
        match self {
            Self::Latest => true,
            Self::Exact(requested) => {
                requested.trim_start_matches(['v', 'V']) == candidate.trim_start_matches(['v', 'V'])
            }
        }
    }
}

impl fmt::Display for RequestedVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Latest => f.write_str("latest stable"),
            Self::Exact(version) => f.write_str(version),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AppImageArchitecture {
    X86_64,
    Aarch64,
}

impl fmt::Display for AppImageArchitecture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::X86_64 => "x86_64",
            Self::Aarch64 => "aarch64",
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AppImagePackage {
    pub id: String,
    pub display_name: String,
    pub repository: String,
    pub version: String,
    pub release_tag: String,
    pub asset_name: String,
    pub url: String,
    pub hash: String,
    pub architecture: AppImageArchitecture,
    pub size: u64,
}

impl AppImagePackage {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_attribute(&self.id)?;
        if !self.id.starts_with("appimage.") {
            return Err(ValidationError::InvalidRequest(
                "external package identifier must use the appimage namespace".into(),
            ));
        }
        validate_display_text(&self.display_name, 120, "AppImage display name")?;
        validate_display_text(&self.version, 64, "AppImage version")?;
        validate_display_text(&self.release_tag, 128, "AppImage release tag")?;
        validate_display_text(&self.asset_name, 240, "AppImage asset name")?;
        if !self.asset_name.to_ascii_lowercase().ends_with(".appimage") {
            return Err(ValidationError::InvalidRequest(
                "external release asset is not an AppImage".into(),
            ));
        }
        if self.size == 0 || self.size > MAX_APPIMAGE_BYTES {
            return Err(ValidationError::InvalidRequest(
                "external AppImage size is outside Peasy's allowed range".into(),
            ));
        }
        let mut repository = self.repository.split('/');
        let owner = repository.next().unwrap_or_default();
        let name = repository.next().unwrap_or_default();
        if repository.next().is_some() || !valid_github_slug(owner) || !valid_github_slug(name) {
            return Err(ValidationError::InvalidRequest(
                "invalid GitHub repository identifier".into(),
            ));
        }
        let expected_id = format!(
            "appimage.{}.{}",
            owner.to_ascii_lowercase(),
            name.to_ascii_lowercase()
        );
        if self.id != expected_id {
            return Err(ValidationError::InvalidRequest(
                "external package identifier does not match its repository".into(),
            ));
        }
        let path = self
            .url
            .strip_prefix("https://github.com/")
            .ok_or_else(|| {
                ValidationError::InvalidRequest(
                    "external AppImages must use a GitHub HTTPS release URL".into(),
                )
            })?;
        if self.url.len() > 2048
            || self.url.contains(['?', '#', '\\', '\n', '\r', '\0'])
            || !path.to_ascii_lowercase().starts_with(&format!(
                "{}/{}/releases/download/",
                owner.to_ascii_lowercase(),
                name.to_ascii_lowercase()
            ))
        {
            return Err(ValidationError::InvalidRequest(
                "external AppImage URL does not match its GitHub repository".into(),
            ));
        }
        let digest = self.hash.strip_prefix("sha256-").unwrap_or_default();
        if digest.len() != 44
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
        {
            return Err(ValidationError::InvalidRequest(
                "external AppImage must have a valid SHA-256 SRI hash".into(),
            ));
        }
        Ok(())
    }

    pub fn pname(&self) -> String {
        self.repository
            .rsplit('/')
            .next()
            .unwrap_or("external-appimage")
            .to_ascii_lowercase()
    }
}

fn valid_github_slug(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn validate_display_text(value: &str, maximum: usize, label: &str) -> Result<(), ValidationError> {
    if value.is_empty()
        || value.len() > maximum
        || value
            .chars()
            .any(|character| character.is_control() || matches!(character, '\'' | '"'))
    {
        return Err(ValidationError::InvalidRequest(format!("invalid {label}")));
    }
    Ok(())
}

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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ModelAction {
    SearchPackage {
        query: String,
        version: Option<RequestedVersion>,
    },
    SearchAppImage {
        query: String,
        version: Option<RequestedVersion>,
        repository: Option<String>,
    },
    CheckPackage {
        query: String,
    },
    ListThemes,
    ListWifi,
    HyprlandStatus,
    InstallPackage {
        package: String,
        message: Option<String>,
    },
    RemovePackage {
        package: String,
    },
    SetTheme {
        theme: ThemeSettings,
    },
    SetHyprlandSetting {
        change: HyprlandSettingChange,
    },
    HyprlandDispatch {
        dispatch: HyprlandDispatch,
        argument: Option<String>,
    },
    ConnectWifi {
        ssid: String,
    },
    ConnectBluetooth {
        device: String,
    },
    CreateCalendarEvent {
        title: String,
        start_local: String,
        duration_minutes: u16,
    },
    Explain {
        message: String,
    },
    Cancel,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelEnvelope {
    pub action: String,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub package: Option<String>,
    #[serde(default)]
    pub package_version: Option<String>,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub theme_color: Option<String>,
    #[serde(default)]
    pub theme_mode: Option<String>,
    #[serde(default)]
    pub ssid: Option<String>,
    #[serde(default)]
    pub device: Option<String>,
    #[serde(default)]
    pub event_title: Option<String>,
    #[serde(default)]
    pub event_start: Option<String>,
    #[serde(default)]
    pub duration_minutes: Option<u16>,
    #[serde(default)]
    pub hyprland_setting: Option<String>,
    #[serde(default)]
    pub hyprland_value: Option<String>,
    #[serde(default)]
    pub hyprland_dispatch: Option<String>,
    #[serde(default)]
    pub hyprland_argument: Option<String>,
}

impl TryFrom<ModelEnvelope> for ModelAction {
    type Error = ValidationError;

    fn try_from(value: ModelEnvelope) -> Result<Self, Self::Error> {
        match value.action.as_str() {
            "search_package" | "search_appimage" | "check_package" => {
                let repository = if value.action == "search_appimage" {
                    value
                        .repository
                        .map(|repository| validate_github_repository(&repository))
                        .transpose()?
                } else {
                    None
                };
                let query = match value.query {
                    Some(query) => validate_query(&query)?.to_owned(),
                    None if repository.is_some() => repository
                        .as_deref()
                        .and_then(|repository| repository.rsplit('/').next())
                        .expect("validated GitHub repository has a name")
                        .to_owned(),
                    None => {
                        return Err(ValidationError::InvalidRequest(
                            "package search requires query".into(),
                        ));
                    }
                };
                if matches!(value.action.as_str(), "search_package" | "search_appimage") {
                    let version = value
                        .package_version
                        .as_deref()
                        .map(RequestedVersion::parse)
                        .transpose()?;
                    if value.action == "search_appimage" {
                        Ok(Self::SearchAppImage {
                            query,
                            version,
                            repository,
                        })
                    } else {
                        Ok(Self::SearchPackage { query, version })
                    }
                } else {
                    Ok(Self::CheckPackage { query })
                }
            }
            "list_themes" => Ok(Self::ListThemes),
            "list_wifi" => Ok(Self::ListWifi),
            "hyprland_status" => Ok(Self::HyprlandStatus),
            "install_package" | "remove_package" => {
                let package = value.package.ok_or_else(|| {
                    ValidationError::InvalidRequest("package action requires package".into())
                })?;
                validate_attribute(&package)?;
                if value.action == "install_package" {
                    let message = value
                        .message
                        .filter(|message| !message.trim().is_empty())
                        .map(normalize_model_message)
                        .transpose()?;
                    Ok(Self::InstallPackage { package, message })
                } else {
                    Ok(Self::RemovePackage { package })
                }
            }
            "set_theme" => {
                let theme = ThemeSettings {
                    accent_color: value
                        .theme_color
                        .as_deref()
                        .map(AccentColor::parse)
                        .transpose()?,
                    color_scheme: value
                        .theme_mode
                        .as_deref()
                        .map(ColorScheme::parse)
                        .transpose()?,
                };
                if theme.is_empty() {
                    return Err(ValidationError::InvalidRequest(
                        "set_theme requires theme_color or theme_mode".into(),
                    ));
                }
                Ok(Self::SetTheme { theme })
            }
            "set_hyprland_setting" => {
                let setting = HyprlandSetting::parse(
                    value.hyprland_setting.as_deref().ok_or_else(|| {
                        ValidationError::InvalidRequest(
                            "set_hyprland_setting requires hyprland_setting".into(),
                        )
                    })?,
                )?;
                let normalized = setting.normalize_value(
                    value.hyprland_value.as_deref().ok_or_else(|| {
                        ValidationError::InvalidRequest(
                            "set_hyprland_setting requires hyprland_value".into(),
                        )
                    })?,
                )?;
                Ok(Self::SetHyprlandSetting {
                    change: HyprlandSettingChange {
                        setting,
                        value: normalized,
                    },
                })
            }
            "hyprland_dispatch" => {
                let dispatch = HyprlandDispatch::parse(
                    value.hyprland_dispatch.as_deref().ok_or_else(|| {
                        ValidationError::InvalidRequest(
                            "hyprland_dispatch requires a typed action".into(),
                        )
                    })?,
                )?;
                let argument = dispatch.normalize_argument(value.hyprland_argument.as_deref())?;
                Ok(Self::HyprlandDispatch { dispatch, argument })
            }
            "connect_wifi" => {
                let ssid = value.ssid.ok_or_else(|| {
                    ValidationError::InvalidRequest("connect_wifi requires ssid".into())
                })?;
                validate_ssid(&ssid)?;
                Ok(Self::ConnectWifi { ssid })
            }
            "connect_bluetooth" => {
                let device = value.device.ok_or_else(|| {
                    ValidationError::InvalidRequest("connect_bluetooth requires device".into())
                })?;
                Ok(Self::ConnectBluetooth {
                    device: validate_query(&device)?.to_owned(),
                })
            }
            "create_calendar_event" => {
                let title = value.event_title.ok_or_else(|| {
                    ValidationError::InvalidRequest(
                        "create_calendar_event requires event_title".into(),
                    )
                })?;
                let start_local = value.event_start.ok_or_else(|| {
                    ValidationError::InvalidRequest(
                        "create_calendar_event requires event_start".into(),
                    )
                })?;
                let duration_minutes = value.duration_minutes.ok_or_else(|| {
                    ValidationError::InvalidRequest(
                        "create_calendar_event requires duration_minutes".into(),
                    )
                })?;
                validate_event_title(&title)?;
                validate_local_datetime(&start_local)?;
                if !(5..=1440).contains(&duration_minutes) {
                    return Err(ValidationError::InvalidRequest(
                        "calendar duration must be between 5 minutes and 24 hours".into(),
                    ));
                }
                Ok(Self::CreateCalendarEvent {
                    title,
                    start_local,
                    duration_minutes,
                })
            }
            "explain" => {
                let message = normalize_model_message(value.message.unwrap_or_default())?;
                Ok(Self::Explain { message })
            }
            "cancel" => Ok(Self::Cancel),
            other => Err(ValidationError::InvalidRequest(format!(
                "unknown model action `{other}`"
            ))),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EngineInput {
    pub action: ModelAction,
    pub candidates: Vec<PackageCandidate>,
    pub installed: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "decision", content = "value", rename_all = "snake_case")]
pub enum EngineDecision {
    Search {
        query: String,
        version: Option<RequestedVersion>,
    },
    SearchAppImage {
        query: String,
        version: Option<RequestedVersion>,
        repository: Option<String>,
    },
    CheckPackage(String),
    ListThemes,
    ListWifi,
    HyprlandStatus,
    Install {
        package: String,
        message: Option<String>,
    },
    Remove(String),
    SetTheme(ThemeSettings),
    SetHyprlandSetting(HyprlandSettingChange),
    HyprlandDispatch {
        dispatch: HyprlandDispatch,
        argument: Option<String>,
    },
    ConnectWifi(String),
    ConnectBluetooth(String),
    CreateCalendarEvent {
        title: String,
        start_local: String,
        duration_minutes: u16,
    },
    Explain(String),
    Cancel,
    Reject(String),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageOperation {
    Install,
    Remove,
}

impl fmt::Display for PackageOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Install => "install",
            Self::Remove => "remove",
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "request", rename_all = "snake_case", deny_unknown_fields)]
pub enum IpcRequest {
    SearchPackages { query: String },
    GetPackages,
    GetTheme,
    GetManagedModule,
    ProposeInstall { package: String },
    ProposeAppImageInstall { package: AppImagePackage },
    ProposeRemove { package: String },
    ProposeTheme { theme: ThemeSettings },
    Apply { proposal: String },
    Status,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Proposal {
    pub id: String,
    pub title: String,
    pub change: ProposalChange,
    pub diff: Vec<DiffLine>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "change", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProposalChange {
    Package {
        operation: PackageOperation,
        package: String,
        display_name: String,
    },
    Theme {
        theme: ThemeSettings,
    },
    AppImage {
        operation: PackageOperation,
        package: AppImagePackage,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffKind {
    Context,
    Add,
    Remove,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DiffLine {
    pub kind: DiffKind,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ApplyResult {
    pub configuration_valid: bool,
    pub build_successful: bool,
    pub activated: bool,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "response", rename_all = "snake_case", deny_unknown_fields)]
pub enum IpcResponse {
    SearchResults { candidates: Vec<PackageCandidate> },
    Packages { packages: Vec<String> },
    Theme { theme: ThemeSettings },
    ManagedModule { module: String },
    Proposal { proposal: Box<Proposal> },
    Applied { result: ApplyResult },
    Status { ready: bool, applying: bool },
    Error { message: String },
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackageState {
    pub packages: Vec<String>,
    #[serde(default)]
    pub appimages: Vec<AppImagePackage>,
    #[serde(default)]
    pub theme: ThemeSettings,
}

impl PackageState {
    pub fn normalize(&mut self) -> Result<(), ValidationError> {
        let mut unique = BTreeSet::new();
        for package in &self.packages {
            validate_attribute(package)?;
            unique.insert(package.clone());
        }
        self.packages = unique.into_iter().collect();
        let mut appimage_ids = BTreeSet::new();
        for package in &self.appimages {
            package.validate()?;
            if !appimage_ids.insert(package.id.clone()) {
                return Err(ValidationError::InvalidRequest(format!(
                    "duplicate external package `{}`",
                    package.id
                )));
            }
        }
        self.appimages.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(())
    }

    pub fn with_change(
        &self,
        operation: PackageOperation,
        package: &str,
    ) -> Result<Self, ValidationError> {
        validate_attribute(package)?;
        let mut packages: BTreeSet<String> = self.packages.iter().cloned().collect();
        match operation {
            PackageOperation::Install => {
                packages.insert(package.to_owned());
            }
            PackageOperation::Remove => {
                packages.remove(package);
            }
        }
        Ok(Self {
            packages: packages.into_iter().collect(),
            appimages: self.appimages.clone(),
            theme: self.theme.clone(),
        })
    }

    pub fn with_appimage_install(
        &self,
        package: &AppImagePackage,
    ) -> Result<Self, ValidationError> {
        package.validate()?;
        let mut appimages = self.appimages.clone();
        if let Some(existing) = appimages.iter_mut().find(|item| item.id == package.id) {
            *existing = package.clone();
        } else {
            appimages.push(package.clone());
        }
        let mut state = Self {
            packages: self.packages.clone(),
            appimages,
            theme: self.theme.clone(),
        };
        state.normalize()?;
        Ok(state)
    }

    pub fn with_appimage_remove(&self, id: &str) -> Result<Self, ValidationError> {
        validate_attribute(id)?;
        let mut state = Self {
            packages: self.packages.clone(),
            appimages: self
                .appimages
                .iter()
                .filter(|package| package.id != id)
                .cloned()
                .collect(),
            theme: self.theme.clone(),
        };
        state.normalize()?;
        Ok(state)
    }

    pub fn with_theme(&self, change: &ThemeSettings) -> Result<Self, ValidationError> {
        if change.is_empty() {
            return Err(ValidationError::InvalidRequest(
                "theme change must set a colour or mode".into(),
            ));
        }
        Ok(Self {
            packages: self.packages.clone(),
            appimages: self.appimages.clone(),
            theme: self.theme.merged(change),
        })
    }
}

pub fn render_packages_module(state: &PackageState) -> Result<String, ValidationError> {
    render_packages_module_version(state, false)
}

// Retain the exact old renderer solely for strict migration checks. Both formats
// must match trusted output byte-for-byte; never accept arbitrary Nix beside a
// valid embedded state record. All new writes use the desktop-aware format.
fn render_packages_module_version(
    state: &PackageState,
    legacy_gnome: bool,
) -> Result<String, ValidationError> {
    let mut state = state.clone();
    state.normalize()?;
    for package in &state.packages {
        validate_attribute(package)?;
    }
    for package in &state.appimages {
        package.validate()?;
    }
    let package_lines = state
        .packages
        .iter()
        .map(|package| format!("      \"{package}\""))
        .collect::<Vec<_>>()
        .join("\n");
    let mut appearance = String::new();
    if !state.theme.is_empty() {
        appearance.push_str(if legacy_gnome {
            "\n  programs.dconf.enable = true;\n  programs.dconf.profiles.user.databases = [\n    {\n      settings.\"org/gnome/desktop/interface\" = {\n"
        } else {
            "\n  programs.dconf.enable = lib.mkIf (config.services.desktopManager.gnome.enable or false) true;\n  programs.dconf.profiles.user.databases = lib.mkIf (config.services.desktopManager.gnome.enable or false) [\n    {\n      settings.\"org/gnome/desktop/interface\" = {\n"
        });
        if let Some(color) = state.theme.accent_color {
            appearance.push_str(&format!("        accent-color = \"{color}\";\n"));
        }
        if let Some(scheme) = state.theme.color_scheme {
            appearance.push_str(&format!(
                "        color-scheme = \"{}\";\n",
                scheme.gsettings_value()
            ));
        }
        // Theme keys remain normal desktop preferences. The unprivileged
        // client writes these same closed enum values to the current session
        // after the reviewed generation activates.
        appearance.push_str("      };\n    }\n  ];\n");
    }
    let state_json = serde_json::to_string(&state).expect("serializing validated Peasy state");
    let theme_json = serde_json::to_string(&state.theme).expect("serializing validated theme");
    appearance.push_str(&format!(
        "\n  environment.etc.\"peasy/state.json\".text = {state_json_nix};\n  environment.etc.\"peasy/theme.json\".text = {theme_json_nix};\n",
        state_json_nix = nix_string(&state_json),
        theme_json_nix = nix_string(&theme_json),
    ));
    let appimages = render_appimage_bindings(&state.appimages, "  ")?;
    let arguments = if legacy_gnome {
        "{ lib, pkgs, ... }"
    } else {
        "{ config, lib, pkgs, ... }"
    };
    Ok(format!(
        "# Generated by Peasy. Do not edit.\n{MANAGED_STATE_PREFIX}{state_json}\n{arguments}:\nlet\n  peasyExternalAppImages = [\n{appimages}  ];\nin\n{{\n  environment.systemPackages = (map\n    (attribute: lib.getAttrFromPath (lib.splitString \".\" attribute) pkgs)\n    [\n{package_lines}\n    ]) ++ peasyExternalAppImages;\n{appearance}}}\n"
    ))
}

pub fn parse_packages_module(source: &str) -> Result<PackageState, ValidationError> {
    let encoded = source
        .lines()
        .find_map(|line| line.strip_prefix(MANAGED_STATE_PREFIX))
        .ok_or_else(|| {
            ValidationError::InvalidRequest(
                "Peasy managed module has no embedded state record".into(),
            )
        })?;
    let mut state: PackageState = serde_json::from_str(encoded).map_err(|_| {
        ValidationError::InvalidRequest("Peasy managed module state is invalid".into())
    })?;
    state.normalize()?;
    if render_packages_module(&state)? != source
        && render_packages_module_version(&state, true)? != source
    {
        return Err(ValidationError::InvalidRequest(
            "Peasy managed module was modified outside Peasy".into(),
        ));
    }
    Ok(state)
}

fn render_appimage_bindings(
    packages: &[AppImagePackage],
    indent: &str,
) -> Result<String, ValidationError> {
    let mut rendered = String::new();
    for package in packages {
        package.validate()?;
        let pname = nix_string(&package.pname());
        let version = nix_string(&package.version);
        let display_name = nix_string(&package.display_name);
        let url = nix_string(&package.url);
        let hash = nix_string(&package.hash);
        rendered.push_str(&format!(
            "{indent}  (let\n{indent}    wrapped = pkgs.appimageTools.wrapType2 {{\n{indent}      pname = {pname};\n{indent}      version = {version};\n{indent}      src = pkgs.fetchurl {{\n{indent}        url = {url};\n{indent}        hash = {hash};\n{indent}      }};\n{indent}    }};\n{indent}    desktop = pkgs.makeDesktopItem {{\n{indent}      name = {pname};\n{indent}      desktopName = {display_name};\n{indent}      exec = {pname};\n{indent}      icon = \"application-x-executable\";\n{indent}      categories = [ \"Network\" ];\n{indent}    }};\n{indent}  in pkgs.symlinkJoin {{\n{indent}    name = {pname};\n{indent}    paths = [ wrapped desktop ];\n{indent}  }})\n"
        ));
    }
    Ok(rendered)
}

pub fn nix_string(value: &str) -> String {
    // JSON quoting alone leaves Nix interpolation executable. Escape it even
    // in otherwise validated metadata and embedded state JSON.
    serde_json::to_string(value)
        .expect("serializing validated Nix string")
        .replace("${", "\\${")
}

pub fn module_diff(
    before: &PackageState,
    after: &PackageState,
) -> Result<Vec<DiffLine>, ValidationError> {
    let before_rendered = render_packages_module(before)?;
    let after_rendered = render_packages_module(after)?;
    let before_lines = before_rendered
        .lines()
        .filter(|line| {
            !line.starts_with(MANAGED_STATE_PREFIX)
                && !line.contains("environment.etc.\"peasy/state.json\"")
        })
        .collect::<Vec<_>>();
    let after_lines = after_rendered
        .lines()
        .filter(|line| {
            !line.starts_with(MANAGED_STATE_PREFIX)
                && !line.contains("environment.etc.\"peasy/state.json\"")
        })
        .collect::<Vec<_>>();
    let mut prefix = 0;
    while prefix < before_lines.len()
        && prefix < after_lines.len()
        && before_lines[prefix] == after_lines[prefix]
    {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < before_lines.len().saturating_sub(prefix)
        && suffix < after_lines.len().saturating_sub(prefix)
        && before_lines[before_lines.len() - 1 - suffix]
            == after_lines[after_lines.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let context_start = prefix.saturating_sub(2);
    let before_end = before_lines.len() - suffix;
    let after_end = after_lines.len() - suffix;
    let mut diff = before_lines[context_start..prefix]
        .iter()
        .map(|line| DiffLine {
            kind: DiffKind::Context,
            text: (*line).to_owned(),
        })
        .collect::<Vec<_>>();
    diff.extend(
        before_lines[prefix..before_end]
            .iter()
            .map(|line| DiffLine {
                kind: DiffKind::Remove,
                text: (*line).to_owned(),
            }),
    );
    diff.extend(after_lines[prefix..after_end].iter().map(|line| DiffLine {
        kind: DiffKind::Add,
        text: (*line).to_owned(),
    }));
    let context_end = (after_end + 2).min(after_lines.len());
    diff.extend(
        after_lines[after_end..context_end]
            .iter()
            .map(|line| DiffLine {
                kind: DiffKind::Context,
                text: (*line).to_owned(),
            }),
    );
    Ok(diff)
}

pub fn render_system_expression(
    nixpkgs_path: &Path,
    host_configuration: &Path,
    system: &str,
) -> Result<String, ValidationError> {
    if !nixpkgs_path.starts_with("/nix/store") {
        return Err(ValidationError::InvalidRequest(
            "Nixpkgs must reside in the Nix store".into(),
        ));
    }
    for (name, path) in [
        ("Nixpkgs", nixpkgs_path),
        ("host configuration", host_configuration),
    ] {
        let value = path.to_string_lossy();
        if !path.is_absolute() || value.contains(['\n', '\r', '\0']) {
            return Err(ValidationError::InvalidRequest(format!(
                "{name} must be an absolute path"
            )));
        }
    }
    if system.is_empty()
        || !system
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(ValidationError::InvalidRequest(
            "invalid administrator-configured system".into(),
        ));
    }

    let nixpkgs = nix_string(&nixpkgs_path.to_string_lossy());
    let host = nix_string(&host_configuration.to_string_lossy());
    let system = serde_json::to_string(system).expect("system JSON");
    Ok(format!(
        "# Generated by Peasy. Do not edit.\nlet\n  nixpkgs = builtins.toPath {nixpkgs};\n  evaluated = import (nixpkgs + \"/nixos/lib/eval-config.nix\") {{\n    system = {system};\n    modules = [ (builtins.toPath {host}) ];\n  }};\nin\nevaluated.config.system.build.toplevel\n"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nix_interpolation_is_inert_in_metadata_and_embedded_json() {
        let hostile = "${builtins.readFile /etc/passwd}";
        assert_eq!(
            nix_string(hostile),
            "\"\\${builtins.readFile /etc/passwd}\""
        );
        let mut package = appimage();
        package.display_name = hostile.into();
        let state = PackageState::default()
            .with_appimage_install(&package)
            .unwrap();
        let source = render_packages_module(&state).unwrap();
        assert_eq!(parse_packages_module(&source).unwrap(), state);
        for line in source.lines().filter(|line| !line.starts_with('#')) {
            assert!(!line.replace("\\${", "").contains("${"));
        }
    }

    #[test]
    fn external_trust_requires_both_repository_and_exact_digest() {
        let mut package = appimage();
        let policy = AppImagePolicy(Some(std::collections::BTreeMap::from([(
            package.repository.clone(),
            vec![package.hash.clone()],
        )])));
        assert!(!AppImagePolicy::default().allows(&package));
        assert!(policy.allows(&package));
        package.hash = "sha256-BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB=".into();
        assert!(!policy.allows(&package));
    }

    #[test]
    fn reviewed_appimages_need_no_preapproved_hash_but_still_require_valid_records() {
        let policy: AppImagePolicy = serde_json::from_str("null").unwrap();
        let mut package = appimage();
        assert!(!policy.is_disabled());
        assert!(policy.allows_repository(&package.repository));
        assert!(policy.allows(&package));
        package.hash = "not-a-hash".into();
        assert!(!policy.allows(&package));
        package = appimage();
        package.url = "https://example.com/unrelated.AppImage".into();
        assert!(!policy.allows(&package));
        let disabled: AppImagePolicy = serde_json::from_str("{}").unwrap();
        assert!(disabled.is_disabled());
        assert!(!disabled.allows_repository("example/nostr-chat"));
        assert!(!disabled.allows(&appimage()));
        assert!(serde_json::from_str::<AppImagePolicy>("true").is_err());
    }

    fn appimage() -> AppImagePackage {
        AppImagePackage {
            id: "appimage.example.nostr-chat".into(),
            display_name: "Nostr Chat".into(),
            repository: "example/nostr-chat".into(),
            version: "1.2.0".into(),
            release_tag: "v1.2.0".into(),
            asset_name: "nostr-chat-1.2.0-x86_64.AppImage".into(),
            url: "https://github.com/example/nostr-chat/releases/download/v1.2.0/nostr-chat-1.2.0-x86_64.AppImage".into(),
            hash: "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".into(),
            architecture: AppImageArchitecture::X86_64,
            size: 42_000_000,
        }
    }

    #[test]
    fn rejects_command_and_traversal_attributes() {
        for value in [
            "telegram; rm -rf /",
            "../../home/user/private.txt",
            "$(curl example.com)",
            "hello..world",
            "a/b",
        ] {
            assert!(validate_attribute(value).is_err(), "accepted {value:?}");
        }
    }

    #[test]
    fn rejects_malicious_model_action() {
        let malicious = r#"{"action":"shell","command":"cat /home/user/.ssh/id_ed25519"}"#;
        assert!(serde_json::from_str::<ModelEnvelope>(malicious).is_err());
        let terminal_escape =
            r#"{"action":"explain","query":null,"package":null,"message":"\u001b]52;clipboard"}"#;
        let envelope = serde_json::from_str::<ModelEnvelope>(terminal_escape).unwrap();
        assert!(ModelAction::try_from(envelope).is_err());
    }

    #[test]
    fn package_versions_are_closed_and_exact() {
        assert_eq!(
            RequestedVersion::parse("latest").unwrap(),
            RequestedVersion::Latest
        );
        let exact = RequestedVersion::parse("1.2").unwrap();
        assert!(exact.matches("v1.2"));
        assert!(!exact.matches("v1.2.1"));
        assert!(RequestedVersion::parse("1.2;curl example.com").is_err());

        let action: ModelAction = serde_json::from_str::<ModelEnvelope>(
            r#"{"action":"search_package","query":"nostr chat","package_version":"1.2"}"#,
        )
        .unwrap()
        .try_into()
        .unwrap();
        assert_eq!(
            action,
            ModelAction::SearchPackage {
                query: "nostr chat".into(),
                version: Some(RequestedVersion::Exact("1.2".into())),
            }
        );

        let repository_action: ModelAction = serde_json::from_str::<ModelEnvelope>(
            r#"{"action":"search_appimage","query":null,"repository":"lnbits/nostr-chat"}"#,
        )
        .unwrap()
        .try_into()
        .unwrap();
        assert_eq!(
            repository_action,
            ModelAction::SearchAppImage {
                query: "nostr-chat".into(),
                version: None,
                repository: Some("lnbits/nostr-chat".into()),
            }
        );
    }

    #[test]
    fn appimages_are_pinned_data_and_cannot_inject_nix() {
        let package = appimage();
        package.validate().unwrap();
        let state = PackageState::default()
            .with_appimage_install(&package)
            .unwrap();
        let rendered = render_packages_module(&state).unwrap();
        assert!(rendered.contains("pkgs.appimageTools.wrapType2"));
        assert!(rendered.contains("pkgs.fetchurl"));
        assert!(rendered.contains(&package.url));
        assert!(rendered.contains(&package.hash));
        assert!(rendered.contains("pkgs.makeDesktopItem"));
        assert!(rendered.contains("peasyExternalAppImages"));

        let mut wrong_host = package.clone();
        wrong_host.url = "https://evil.example/nostr-chat.AppImage".into();
        assert!(wrong_host.validate().is_err());
        let mut wrong_repository = package.clone();
        wrong_repository.url =
            "https://github.com/impostor/nostr-chat/releases/download/v1.2.0/nostr-chat.AppImage"
                .into();
        assert!(wrong_repository.validate().is_err());
        let mut injection = package;
        injection.asset_name = "bad.AppImage\"; builtins.readFile /etc/shadow".into();
        assert!(injection.validate().is_err());
    }

    #[test]
    fn ignores_irrelevant_declared_structured_output_fields() {
        let search: ModelAction = serde_json::from_str::<ModelEnvelope>(
            r#"{
                "action":"search_package",
                "query":"opera browser",
                "package":null,
                "message":"Searching for Opera"
            }"#,
        )
        .unwrap()
        .try_into()
        .unwrap();
        assert_eq!(
            search,
            ModelAction::SearchPackage {
                query: "opera browser".to_owned(),
                version: None,
            }
        );

        let install: ModelAction = serde_json::from_str::<ModelEnvelope>(
            r#"{
                "action":"install_package",
                "query":"opera browser",
                "package":"opera",
                "message":"Install Opera"
            }"#,
        )
        .unwrap()
        .try_into()
        .unwrap();
        assert_eq!(
            install,
            ModelAction::InstallPackage {
                package: "opera".to_owned(),
                message: Some("Install Opera".to_owned()),
            }
        );
    }

    #[test]
    fn state_is_sorted_and_rendered_as_data() {
        let mut state = PackageState {
            packages: vec!["vlc".into(), "telegram-desktop".into(), "vlc".into()],
            appimages: Vec::new(),
            theme: ThemeSettings::default(),
        };
        state.normalize().unwrap();
        assert_eq!(state.packages, ["telegram-desktop", "vlc"]);
        let rendered = render_packages_module(&state).unwrap();
        assert!(rendered.contains("\"telegram-desktop\""));
        assert!(!rendered.contains("with pkgs"));
        assert_eq!(parse_packages_module(&rendered).unwrap(), state);
    }

    #[test]
    fn legacy_managed_modules_migrate_without_accepting_modified_nix() {
        for theme in [
            ThemeSettings::default(),
            ThemeSettings {
                accent_color: Some(AccentColor::Green),
                color_scheme: Some(ColorScheme::Dark),
            },
        ] {
            let state = PackageState {
                packages: vec!["hello".into()],
                appimages: vec![],
                theme,
            };
            let old = render_packages_module_version(&state, true).unwrap();
            assert!(old.contains("\n{ lib, pkgs, ... }:\n"));
            assert_eq!(parse_packages_module(&old).unwrap(), state);
            let upgraded = render_packages_module(&parse_packages_module(&old).unwrap()).unwrap();
            assert!(upgraded.contains("\n{ config, lib, pkgs, ... }:\n"));
            assert_eq!(parse_packages_module(&upgraded).unwrap(), state);
            assert!(parse_packages_module(&(old + "\n# external edit\n")).is_err());
            assert!(
                parse_packages_module(&upgraded.replace(
                    "environment.systemPackages",
                    "system.activationScripts.inject.text"
                ))
                .is_err()
            );
        }
    }

    #[test]
    fn theme_values_are_closed_and_render_for_live_dconf_sync() {
        let envelope = serde_json::from_str::<ModelEnvelope>(
            r#"{
                "action":"set_theme",
                "query":null,
                "package":null,
                "message":"Blue theme",
                "theme_color":"blue",
                "theme_mode":"dark"
            }"#,
        )
        .unwrap();
        let ModelAction::SetTheme { theme } = ModelAction::try_from(envelope).unwrap() else {
            panic!("expected theme action");
        };
        let state = PackageState::default().with_theme(&theme).unwrap();
        let rendered = render_packages_module(&state).unwrap();
        assert!(rendered.contains("accent-color = \"blue\";"));
        assert!(rendered.contains("color-scheme = \"prefer-dark\";"));
        assert!(!rendered.contains("locks ="));
        assert!(rendered.contains("environment.etc.\"peasy/theme.json\""));
        assert!(rendered.contains(r#"{"accent_color":"blue","color_scheme":"dark"}"#));

        let invalid = r#"{
            "action":"set_theme",
            "theme_color":"chartreuse",
            "theme_mode":null
        }"#;
        let envelope = serde_json::from_str::<ModelEnvelope>(invalid).unwrap();
        assert!(ModelAction::try_from(envelope).is_err());

        assert!(rendered.contains("environment.etc.\"peasy/state.json\""));
    }

    #[test]
    fn module_diff_marks_exact_removed_and_added_lines() {
        let before = PackageState::default();
        let after = before
            .with_theme(&ThemeSettings {
                accent_color: Some(AccentColor::Blue),
                color_scheme: None,
            })
            .unwrap();
        let diff = module_diff(&before, &after).unwrap();
        assert!(diff.iter().any(|line| {
            line.kind == DiffKind::Add && line.text.contains("accent-color = \"blue\"")
        }));
        let changed = after
            .with_theme(&ThemeSettings {
                accent_color: Some(AccentColor::Purple),
                color_scheme: None,
            })
            .unwrap();
        let changed_diff = module_diff(&after, &changed).unwrap();
        assert!(changed_diff.iter().any(|line| {
            line.kind == DiffKind::Remove && line.text.contains("accent-color = \"blue\"")
        }));
        assert!(changed_diff.iter().any(|line| {
            line.kind == DiffKind::Add && line.text.contains("accent-color = \"purple\"")
        }));
    }

    #[test]
    fn validates_read_only_and_live_model_actions() {
        for (json, expected) in [
            (r#"{"action":"list_themes"}"#, ModelAction::ListThemes),
            (r#"{"action":"list_wifi"}"#, ModelAction::ListWifi),
            (
                r#"{"action":"check_package","query":"obs"}"#,
                ModelAction::CheckPackage {
                    query: "obs".into(),
                },
            ),
            (
                r#"{"action":"connect_wifi","ssid":"CoolCafe"}"#,
                ModelAction::ConnectWifi {
                    ssid: "CoolCafe".into(),
                },
            ),
            (
                r#"{"action":"connect_bluetooth","device":"Beats headphones"}"#,
                ModelAction::ConnectBluetooth {
                    device: "Beats headphones".into(),
                },
            ),
            (
                r#"{"action":"create_calendar_event","event_title":"Walk with Dad","event_start":"2026-09-27T10:00:00","duration_minutes":60}"#,
                ModelAction::CreateCalendarEvent {
                    title: "Walk with Dad".into(),
                    start_local: "2026-09-27T10:00:00".into(),
                    duration_minutes: 60,
                },
            ),
        ] {
            let envelope = serde_json::from_str::<ModelEnvelope>(json).unwrap();
            assert_eq!(ModelAction::try_from(envelope).unwrap(), expected);
        }

        for invalid in [
            r#"{"action":"connect_wifi","ssid":"bad\nssid"}"#,
            r#"{"action":"create_calendar_event","event_title":"Walk","event_start":"2026-02-30T10:00:00","duration_minutes":60}"#,
            r#"{"action":"create_calendar_event","event_title":"Walk","event_start":"2026-09-27T25:00:00","duration_minutes":60}"#,
            r#"{"action":"create_calendar_event","event_title":"Walk","event_start":"2026-09-27T10:00:00","duration_minutes":1}"#,
        ] {
            let envelope = serde_json::from_str::<ModelEnvelope>(invalid).unwrap();
            assert!(
                ModelAction::try_from(envelope).is_err(),
                "accepted {invalid}"
            );
        }
    }

    #[test]
    fn hyprland_actions_are_closed_and_values_are_normalized() {
        let setting = serde_json::from_str::<ModelEnvelope>(
            r#"{"action":"set_hyprland_setting","hyprland_setting":"gaps_outer","hyprland_value":"12"}"#,
        )
        .unwrap();
        assert_eq!(
            ModelAction::try_from(setting).unwrap(),
            ModelAction::SetHyprlandSetting {
                change: HyprlandSettingChange {
                    setting: HyprlandSetting::GapsOuter,
                    value: "12".into(),
                }
            }
        );

        let dispatch = serde_json::from_str::<ModelEnvelope>(
            r#"{"action":"hyprland_dispatch","hyprland_dispatch":"focus_direction","hyprland_argument":"left"}"#,
        )
        .unwrap();
        assert_eq!(
            ModelAction::try_from(dispatch).unwrap(),
            ModelAction::HyprlandDispatch {
                dispatch: HyprlandDispatch::FocusDirection,
                argument: Some("l".into()),
            }
        );

        for invalid in [
            r#"{"action":"set_hyprland_setting","hyprland_setting":"plugin","hyprland_value":"load /tmp/evil.so"}"#,
            r#"{"action":"set_hyprland_setting","hyprland_setting":"layout","hyprland_value":"dwindle\"); os.execute(\"sh\""}"#,
            r#"{"action":"hyprland_dispatch","hyprland_dispatch":"exec","hyprland_argument":"sh"}"#,
            r#"{"action":"hyprland_dispatch","hyprland_dispatch":"switch_workspace","hyprland_argument":"1; exec sh"}"#,
            r#"{"action":"set_hyprland_setting","hyprland_setting":"active_opacity","hyprland_value":"NaN"}"#,
        ] {
            let envelope = serde_json::from_str::<ModelEnvelope>(invalid).unwrap();
            assert!(
                ModelAction::try_from(envelope).is_err(),
                "accepted {invalid}"
            );
        }
    }

    #[test]
    fn old_package_only_state_migrates_with_empty_optional_state() {
        let state: PackageState = serde_json::from_str(r#"{"packages":["vlc"]}"#).unwrap();
        assert_eq!(state.packages, ["vlc"]);
        assert!(state.appimages.is_empty());
        assert_eq!(state.theme, ThemeSettings::default());
    }

    #[test]
    fn system_expression_uses_only_trusted_absolute_paths() {
        let rendered = render_system_expression(
            Path::new("/nix/store/00000000000000000000000000-nixpkgs"),
            Path::new("/etc/nixos/configuration.nix"),
            "x86_64-linux",
        )
        .unwrap();
        assert!(rendered.contains("nixos/lib/eval-config.nix"));
        assert!(rendered.contains("/etc/nixos/configuration.nix"));
        assert!(!rendered.contains("--flake"));

        assert!(
            render_system_expression(
                Path::new("/nix/store/00000000000000000000000000-nixpkgs"),
                Path::new("../../home/user/configuration.nix"),
                "x86_64-linux",
            )
            .is_err()
        );
    }
}

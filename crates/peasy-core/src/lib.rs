#[cfg(not(target_arch = "wasm32"))]
pub mod cancellation;
#[cfg(not(target_arch = "wasm32"))]
pub mod process;
#[cfg(not(target_arch = "wasm32"))]
pub mod progress;

#[path = "../../../peas/packages/types.rs"]
mod packages;
pub use packages::{
    MAX_ATTRIBUTE_BYTES, MAX_CANDIDATES, PackageCandidate, PackageOperation, RequestedVersion,
    regex_escape, validate_attribute,
};
#[path = "../../../peas/appimages/types.rs"]
mod appimages;
pub use appimages::{
    APPIMAGE_POLICY_PATH, AppImageArchitecture, AppImagePackage, AppImagePolicy, MAX_APPIMAGE_BYTES,
};
#[path = "../../../peas/appearance/types.rs"]
mod appearance;
pub use appearance::{AccentColor, ColorScheme, ThemeSettings};
#[path = "../../../peas/hyprland/types.rs"]
mod hyprland;
pub use hyprland::{HyprlandDispatch, HyprlandSetting, HyprlandSettingChange};
#[path = "../../../peas/wifi/types.rs"]
mod wifi;
pub use wifi::{MAX_SSID_BYTES, validate_ssid};
#[path = "../../../peas/calendar/types.rs"]
mod calendar;
use appimages::{render_appimage_bindings, validate_github_repository};
pub use calendar::{
    LOCAL_DATETIME_BYTES, MAX_EVENT_TITLE_BYTES, validate_event_title, validate_local_datetime,
};

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;
use thiserror::Error;

#[cfg(test)]
#[path = "../../../peas/tests/contracts.rs"]
mod pea_contracts;

#[path = "../../../peas/appearance/desktop.rs"]
mod desktop;
pub use desktop::{AppearanceCapabilities, DesktopEnvironment};

pub const MAX_QUERY_BYTES: usize = 160;

#[path = "../../../peas/system_configuration/types.rs"]
mod system_configuration;
pub use system_configuration::{ManagedSetup, SYSTEM_ENABLE_OPTIONS, SYSTEM_GROUPS, SystemSetup};

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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        setup: Option<SystemSetup>,
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
    pub setup: Option<SystemSetup>,
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
        if value.setup.is_some() && value.action != "install_package" {
            return Err(ValidationError::InvalidRequest(
                "setup requires install_package".into(),
            ));
        }
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
                    if let Some(setup) = &value.setup {
                        setup.validate()?;
                    }
                    Ok(Self::InstallPackage {
                        package,
                        message,
                        setup: value.setup,
                    })
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        setup: Option<SystemSetup>,
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

/// Kept stable so clients can recognize the existing protocol's restart response.
pub const IPC_RESTARTING_MESSAGE: &str = "Peasy is updating. Retry the request shortly.";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "request", rename_all = "snake_case", deny_unknown_fields)]
pub enum IpcRequest {
    SearchPackages { query: String },
    GetPackages,
    GetTheme,
    GetManagedModule,
    ProposeInstall { package: String },
    ProposeSetup { package: String, setup: SystemSetup },
    ProposeAppImageInstall { package: AppImagePackage },
    ProposeRemove { package: String },
    ProposeTheme { theme: ThemeSettings },
    Apply { proposal: String },
    ApplyWithProgress { proposal: String },
    Inspect,
    ProposeRecovery,
    Cancel { proposal: String },
    Status,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Proposal {
    pub id: String,
    pub title: String,
    pub change: ProposalChange,
    pub diff: Vec<DiffLine>,
    #[serde(default)]
    pub packages: Vec<PackageIdentity>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "change", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProposalChange {
    Recovery {
        generation: String,
    },
    Setup {
        operation: PackageOperation,
        setup: ManagedSetup,
    },
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
    Cancelled { activation_started: bool },
    Status { ready: bool, applying: bool },
    Error { message: String },
    Progress { stage: OperationStage },
    Inspection { status: Box<ServiceStatus> },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackageIdentity {
    pub attribute: String,
    pub name: String,
    pub version: String,
    pub drv_path: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationStage {
    Authorizing,
    Validating,
    Downloading,
    Building,
    Activating,
    Completed,
}
impl OperationStage {
    pub fn message(self) -> &'static str {
        match self {
            Self::Authorizing => "Waiting for administrator authentication…",
            Self::Validating => "Checking the reviewed packages and configuration…",
            Self::Downloading => "Downloading packages…",
            Self::Building => "Building packages and the system generation…",
            Self::Activating => "Activating the system generation…",
            Self::Completed => "Change completed.",
        }
    }
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RecoveryInfo {
    pub message: String,
    #[serde(default)]
    pub intended_change: Vec<DiffLine>,
    pub intended_packages: Vec<String>,
    pub active_generation: Option<String>,
    pub previous_generation: Option<String>,
    pub needs_attention: bool,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServiceStatus {
    pub version: String,
    pub protocol: u32,
    pub executable: String,
    pub nixpkgs: String,
    pub restart_pending: bool,
    pub applying: bool,
    pub recovery: Option<RecoveryInfo>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackageState {
    pub packages: Vec<String>,
    #[serde(default)]
    pub appimages: Vec<AppImagePackage>,
    #[serde(default)]
    pub theme: ThemeSettings,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub setups: Vec<ManagedSetup>,
}

impl PackageState {
    pub fn normalize(&mut self) -> Result<(), ValidationError> {
        let mut identities = BTreeSet::new();
        for setup in &mut self.setups {
            setup.normalize()?;
            if !identities.insert(setup.package.clone()) {
                return Err(ValidationError::InvalidRequest(
                    "duplicate managed setup".into(),
                ));
            }
        }
        self.setups.sort_by(|a, b| a.package.cmp(&b.package));
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
            setups: self.setups.clone(),
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
            setups: self.setups.clone(),
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
            setups: self.setups.clone(),
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
            setups: self.setups.clone(),
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
        .effective_packages()
        .iter()
        .map(|package| format!("      \"{package}\""))
        .collect::<Vec<_>>()
        .join("\n");
    let mut appearance = system_configuration::render(&state.setups);
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
        && (!state.setups.is_empty() || render_packages_module_version(&state, true)? != source)
    {
        return Err(ValidationError::InvalidRequest(
            "Peasy managed module was modified outside Peasy".into(),
        ));
    }
    Ok(state)
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
                setup: None,
            }
        );
    }

    #[test]
    fn state_is_sorted_and_rendered_as_data() {
        let mut state = PackageState {
            packages: vec!["vlc".into(), "telegram-desktop".into(), "vlc".into()],
            setups: Vec::new(),
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
                setups: Vec::new(),
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

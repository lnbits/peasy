//! Versioned, data-only pea packages. Host operations remain closed Rust types.
use crate::{
    ModelAction, NetworkScope, PackageState, ValidationError, model_response_schema, nix_string,
    validate_network_id,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
pub const HOST_API: u32 = 1;
pub const POLICY_PATH: &str = "/etc/peasy/pea-policy.json";
pub const MAX_PACK_BYTES: usize = 64 * 1024;
pub const PERMISSIONS: &[&str] = &[
    "network.read",
    "network.session",
    "network.system",
    "packages",
    "appearance",
    "wifi",
    "bluetooth",
    "calendar",
    "hyprland",
];
fn invalid(s: &str) -> ValidationError {
    ValidationError::InvalidRequest(s.into())
}
fn bounded(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && !s.chars().any(|c| c.is_control() && c != '\n')
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeaManifest {
    pub id: String,
    pub version: String,
    pub host_api: u32,
    pub capabilities: Vec<String>,
    pub permissions: Vec<String>,
    pub instructions: String,
    pub response_schema: Value,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeaPin {
    pub id: String,
    pub version: String,
    pub revision: String,
    pub hash: String,
    pub host_api: u32,
    pub permissions: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogueEntry {
    pub package: PeaMetadata,
    pub capabilities: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeaMetadata {
    pub id: String,
    pub version: String,
    pub host_api: u32,
    pub permissions: Vec<String>,
    pub hash: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeaCatalogue {
    pub format: u32,
    pub peas: Vec<CatalogueEntry>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeaPolicy {
    pub allow_official: bool,
    pub allowed_permissions: Vec<String>,
}
impl PeaPolicy {
    pub fn allows(&self, pin: &PeaPin) -> bool {
        self.allow_official
            && pin.validate().is_ok()
            && pin
                .permissions
                .iter()
                .all(|p| self.allowed_permissions.contains(p))
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load(path: &std::path::Path) -> std::io::Result<Self> {
        use std::io::Read;
        let f = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e),
        };
        let mut bytes = vec![];
        f.take(8193).read_to_end(&mut bytes)?;
        if bytes.len() > 8192 {
            return Err(std::io::Error::other("pea policy too large"));
        }
        serde_json::from_slice(&bytes).map_err(std::io::Error::other)
    }
}
fn validate_permissions(permissions: &[String]) -> Result<(), ValidationError> {
    if permissions.is_empty()
        || permissions.len() > PERMISSIONS.len()
        || permissions
            .iter()
            .any(|p| !PERMISSIONS.contains(&p.as_str()))
        || permissions.iter().collect::<BTreeSet<_>>().len() != permissions.len()
    {
        return Err(invalid(
            "unsupported or duplicate pea permissions; a host update may be required",
        ));
    }
    Ok(())
}
pub fn schema_for_permissions(permissions: &[String]) -> Value {
    let mut schema = model_response_schema();
    let mut actions = vec!["explain", "cancel"];
    for p in permissions {
        actions.extend(match p.as_str() {
            "network.read" => vec!["inspect_network"],
            "network.session" | "network.system" => vec!["configure_network"],
            "packages" => vec![
                "search_package",
                "search_appimage",
                "check_package",
                "install_package",
                "remove_package",
            ],
            "appearance" => vec!["list_themes", "set_theme"],
            "wifi" => vec!["list_wifi", "connect_wifi"],
            "bluetooth" => vec!["connect_bluetooth"],
            "calendar" => vec!["create_calendar_event"],
            "hyprland" => vec![
                "hyprland_status",
                "set_hyprland_setting",
                "hyprland_dispatch",
            ],
            _ => vec![],
        });
    }
    actions.sort();
    actions.dedup();
    schema["properties"]["action"]["enum"] = serde_json::json!(actions);
    schema
}
impl PeaManifest {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_network_id(&self.id)?;
        validate_permissions(&self.permissions)?;
        if self.host_api != HOST_API {
            return Err(invalid("pea requires a different host API; update Peasy"));
        }
        if !bounded(&self.version, 32)
            || !bounded(&self.instructions, 12000)
            || self.capabilities.is_empty()
            || self.capabilities.len() > 16
            || self.capabilities.iter().any(|c| !bounded(c, 160))
        {
            return Err(invalid("invalid pea metadata"));
        }
        if self.response_schema != schema_for_permissions(&self.permissions) {
            return Err(invalid(
                "pea schema must match the installed host API and declared permissions",
            ));
        }
        Ok(())
    }
    pub fn permits(&self, action: &ModelAction) -> bool {
        let permission = match action {
            ModelAction::InspectNetwork => "network.read",
            ModelAction::ConfigureNetwork { plan } => {
                if plan.scope == NetworkScope::System {
                    "network.system"
                } else {
                    "network.session"
                }
            }
            ModelAction::SearchPackage { .. }
            | ModelAction::SearchAppImage { .. }
            | ModelAction::CheckPackage { .. }
            | ModelAction::InstallPackage { .. }
            | ModelAction::RemovePackage { .. } => "packages",
            ModelAction::ListThemes | ModelAction::SetTheme { .. } => "appearance",
            ModelAction::ListWifi | ModelAction::ConnectWifi { .. } => "wifi",
            ModelAction::ConnectBluetooth { .. } => "bluetooth",
            ModelAction::CreateCalendarEvent { .. } => "calendar",
            ModelAction::HyprlandStatus
            | ModelAction::SetHyprlandSetting { .. }
            | ModelAction::HyprlandDispatch { .. } => "hyprland",
            ModelAction::Explain { .. } | ModelAction::Cancel => return true,
            _ => return false,
        };
        self.permissions.iter().any(|p| p == permission)
    }
}
impl PeaPin {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_network_id(&self.id)?;
        validate_permissions(&self.permissions)?;
        if self.host_api != HOST_API
            || !bounded(&self.version, 32)
            || self.revision.len() != 40
            || !self.revision.bytes().all(|b| b.is_ascii_hexdigit())
            || self.hash.len() != 64
            || !self.hash.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(invalid("invalid or incompatible pea pin"));
        }
        Ok(())
    }
    pub fn url(&self) -> String {
        format!(
            "https://raw.githubusercontent.com/lnbits/peasy/{}/peas/{}/pea.json",
            self.revision, self.id
        )
    }
    pub fn matches(&self, m: &PeaManifest) -> bool {
        self.id == m.id
            && self.version == m.version
            && self.host_api == m.host_api
            && self.permissions == m.permissions
    }
}
impl PeaCatalogue {
    pub fn validate(&self, revision: &str) -> Result<(), ValidationError> {
        if self.format != 1 || self.peas.len() > 128 {
            return Err(invalid("unsupported pea catalogue"));
        }
        let mut ids = BTreeSet::new();
        for e in &self.peas {
            let p = &e.package;
            validate_network_id(&p.id)?;
            if !ids.insert(&p.id)
                || e.capabilities.is_empty()
                || e.capabilities.len() > 16
                || e.capabilities.iter().any(|c| !bounded(c, 160))
                || !bounded(&p.version, 32)
                || p.hash.len() != 64
                || !p.hash.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err(invalid("invalid pea catalogue entry"));
            }
        }
        if revision.len() != 40 || !revision.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid("invalid catalogue revision"));
        }
        Ok(())
    }
}
impl PackageState {
    pub fn with_pea(&self, pin: &PeaPin, enable: bool) -> Result<Self, ValidationError> {
        pin.validate()?;
        let mut after = self.clone();
        if !enable && !after.peas.contains(pin) {
            return Err(invalid("pea is no longer at its reviewed pin"));
        }
        after.peas.retain(|p| p.id != pin.id);
        if enable {
            after.peas.push(pin.clone());
        }
        after.normalize()?;
        Ok(after)
    }
}
pub(crate) fn render(pins: &[PeaPin]) -> String {
    let mut out = String::new();
    for p in pins {
        out.push_str(&format!(
            "  environment.etc.{}.source = pkgs.fetchurl {{ url = {}; sha256 = {}; }};\n",
            nix_string(&format!("peasy/peas/{}.json", p.id)),
            nix_string(&p.url()),
            nix_string(&p.hash)
        ));
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cannot_expand_the_host_or_hide_permissions_in_a_schema() {
        let mut m = PeaManifest {
            id: "example".into(),
            version: "1".into(),
            host_api: HOST_API,
            capabilities: vec!["Networking".into()],
            permissions: vec!["network.read".into()],
            instructions: "Inspect network resources".into(),
            response_schema: schema_for_permissions(&["network.read".into()]),
        };
        assert!(m.validate().is_ok());
        m.response_schema["additionalProperties"] = Value::Bool(true);
        assert!(m.validate().is_err());
        m.response_schema = schema_for_permissions(&m.permissions);
        m.host_api = 99;
        assert!(m.validate().is_err());
    }
    #[test]
    fn policy_is_fail_closed_and_pins_are_data_only() {
        let p = PeaPin {
            id: "example".into(),
            version: "1".into(),
            revision: "a".repeat(40),
            hash: "b".repeat(64),
            host_api: 1,
            permissions: vec!["network.read".into()],
        };
        assert!(!PeaPolicy::default().allows(&p));
        let state = PackageState::default().with_pea(&p, true).unwrap();
        let text = crate::render_packages_module(&state).unwrap();
        assert_eq!(crate::parse_packages_module(&text).unwrap(), state);
        assert!(text.contains("pkgs.fetchurl"));
        assert!(!text.contains("builtins.getFlake"));
        assert_eq!(state.with_pea(&p, false).unwrap(), PackageState::default());
    }
}

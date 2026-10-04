//! Versioned, data-only pea packages. Host operations remain closed Rust types.
use crate::{
    ModelAction, NetworkScope, PackageState, ValidationError, model_response_schema, nix_string,
    validate_network_id,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
pub const HOST_API: u32 = 6;
pub const POLICY_PATH: &str = "/etc/peasy/pea-policy.json";
pub const MAX_PACK_BYTES: usize = 64 * 1024;
pub const PERMISSIONS: &[&str] = &[
    "applications.read",
    "applications.write",
    "diagnostics.read",
    "services.read",
    "services.write",
    "storage.read",
    "storage.write",
    "nix_maintenance.read",
    "nix_maintenance.write",
    "users.read",
    "users.write",
    "firewall.read",
    "firewall.write",
    "printing.read",
    "printing.write",
    "displays.read",
    "displays.write",
    "audio.read",
    "audio.write",
    "power.read",
    "power.write",
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
    schema_with_permissions(model_response_schema(), permissions)
}
fn schema_with_permissions(mut schema: Value, permissions: &[String]) -> Value {
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
            p if p.ends_with(".read") => vec!["inspect_resources"],
            p if p.ends_with(".write") => vec!["change_resources"],
            _ => vec![],
        });
    }
    actions.sort();
    actions.dedup();
    schema["properties"]["action"]["enum"] = serde_json::json!(actions);
    let reads = permissions
        .iter()
        .filter_map(|p| p.strip_suffix(".read"))
        .filter(|p| *p != "network")
        .collect::<Vec<_>>();
    if reads.is_empty() {
        schema["properties"]["resource_query"] = serde_json::json!({"type":"null"});
    } else {
        schema["properties"]["resource_query"]["properties"]["domain"]["enum"] =
            serde_json::json!(reads);
    }
    let variants = schema["properties"]["resource_change"]["anyOf"]
        .as_array_mut()
        .expect("resource union");
    variants.retain(|v| {
        let Some(operation) = v["properties"]["operation"]["enum"][0].as_str() else {
            return true;
        };
        let domain = match operation {
            "service" | "service_enabled" => "services",
            "disk" | "persistent_mount" => "storage",
            "user_create" | "user_disabled" | "user_groups" => "users",
            "nix_optimise" | "nix_garbage_collect" | "nix_delete_generations" => "nix_maintenance",
            "power_profile" | "power_settings" => "power",
            "printer" => "printing",
            "display" => "displays",
            "audio" => "audio",
            "firewall" => "firewall",
            "open_application" => "applications",
            _ => return false,
        };
        permissions.contains(&format!("{domain}.write"))
    });
    if variants.len() == 1 {
        schema["properties"]["resource_change"] = serde_json::json!({"type":"null"});
    }
    schema
}
const LEGACY_ENABLE_OPTIONS: &[&str] = &[
    "virtualisation.libvirtd.enable",
    "programs.virt-manager.enable",
    "services.printing.enable",
    "hardware.sane.enable",
    "hardware.bluetooth.enable",
];
const LEGACY_GROUPS: &[&str] = &["libvirtd", "lp", "scanner"];
const LEGACY_MESSAGE_CHARS: usize = 400;

// Immutable API 1/2 pins retain exactly their original, narrower schema.
// Do not derive legacy enums from the expanding current catalogue.
fn schema_for_api(permissions: &[String], api: u32) -> Value {
    let mut schema = schema_for_permissions(permissions);
    if api == 5 {
        schema = schema_with_permissions(
            serde_json::from_str(include_str!("../../../peapod/tests/api5-model-schema.json"))
                .expect("frozen API 5 schema"),
            permissions,
        );
    }
    if api == 4 {
        schema = schema_with_permissions(
            serde_json::from_str(include_str!("../../../peapod/tests/api4-model-schema.json"))
                .expect("frozen API 4 schema"),
            permissions,
        );
    }
    if api < 4 {
        let actions = schema["properties"]["action"]["enum"].clone();
        schema = serde_json::from_str(include_str!("../../../peapod/tests/api3-model-schema.json"))
            .expect("frozen API 3 schema");
        schema["properties"]["action"]["enum"] = actions;
    }
    if api < 3 {
        schema["properties"]["message"]["maxLength"] = serde_json::json!(LEGACY_MESSAGE_CHARS);
        schema["properties"]["setup"]["properties"]["enable"]["items"]["enum"] =
            serde_json::json!(LEGACY_ENABLE_OPTIONS);
        schema["properties"]["setup"]["properties"]["groups"]["items"]["enum"] =
            serde_json::json!(LEGACY_GROUPS);
    }
    if api == 1 {
        schema["properties"]["setup"]["properties"]
            .as_object_mut()
            .expect("setup properties")
            .remove("postgresql");
        schema["properties"]["setup"]["required"]
            .as_array_mut()
            .expect("setup required fields")
            .retain(|field| field != "postgresql");
    }
    schema
}

impl PeaManifest {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_network_id(&self.id)?;
        validate_permissions(&self.permissions)?;
        if self.host_api < 6
            && self
                .permissions
                .iter()
                .any(|p| p.starts_with("applications."))
        {
            return Err(invalid("application launching requires host API 6"));
        }
        if self.host_api < 4
            && self
                .permissions
                .iter()
                .any(|p| p.contains('.') && !p.starts_with("network."))
        {
            return Err(invalid("resource permissions require host API 4"));
        }
        if !matches!(self.host_api, 1 | 2 | 3 | 4 | 5 | HOST_API) {
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
        let expected_schema = schema_for_api(&self.permissions, self.host_api);
        if self.response_schema != expected_schema {
            return Err(invalid(
                "pea schema must match the installed host API and declared permissions",
            ));
        }
        Ok(())
    }
    pub fn permits(&self, action: &ModelAction) -> bool {
        // The manifest schema is immutable, but providers can ignore it. Enforce
        // its version-specific differences on native actions as well.
        if self.validate().is_err() {
            return false;
        }
        if self.host_api < 3 {
            let message = match action {
                ModelAction::Explain { message } => Some(message),
                ModelAction::InstallPackage { message, .. } => message.as_ref(),
                _ => None,
            };
            if message.is_some_and(|m| m.chars().count() > LEGACY_MESSAGE_CHARS) {
                return false;
            }
            if let ModelAction::InstallPackage {
                setup: Some(setup), ..
            } = action
                && (setup
                    .enable
                    .iter()
                    .any(|o| !LEGACY_ENABLE_OPTIONS.contains(&o.as_str()))
                    || setup
                        .groups
                        .iter()
                        .any(|g| !LEGACY_GROUPS.contains(&g.as_str()))
                    || (self.host_api == 1 && setup.postgresql.is_some()))
            {
                return false;
            }
        }
        if let ModelAction::InspectResources { query } = action {
            if self.host_api < 6 && query.domain == crate::ResourceDomain::Applications {
                return false;
            }
            return self.host_api >= 4
                && query.validate().is_ok()
                && self
                    .permissions
                    .contains(&format!("{}.read", query.domain.id()));
        }
        if let ModelAction::ChangeResources { change } = action {
            if change.settings_only() {
                return false;
            }
            if self.host_api < 6 && change.domain() == crate::ResourceDomain::Applications {
                return false;
            }
            if self.host_api < 5
                && match change {
                    crate::ResourceChange::Display { x, y, .. } => *x < 0 || *y < 0,
                    crate::ResourceChange::PowerSettings { lid, idle_minutes } => {
                        lid.is_none() || idle_minutes.is_none()
                    }
                    _ => false,
                }
            {
                return false;
            }
            return self.host_api >= 4
                && change.validate().is_ok()
                && self
                    .permissions
                    .contains(&format!("{}.write", change.domain().id()));
        }
        let permission = match action {
            ModelAction::InspectNetwork => "network.read",
            ModelAction::ConfigureNetwork { plan } => {
                if plan.scope == NetworkScope::System {
                    // The deferred activation is itself a session operation.
                    if plan.activate.is_some()
                        && !self.permissions.iter().any(|p| p == "network.session")
                    {
                        return false;
                    }
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
        if self.host_api < 6
            && self
                .permissions
                .iter()
                .any(|p| p.starts_with("applications."))
        {
            return Err(invalid("application launching requires host API 6"));
        }
        if self.host_api < 4
            && self
                .permissions
                .iter()
                .any(|p| p.contains('.') && !p.starts_with("network."))
        {
            return Err(invalid("resource permissions require host API 4"));
        }
        if !matches!(self.host_api, 1 | 2 | 3 | 4 | 5 | HOST_API)
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
            "https://raw.githubusercontent.com/lnbits/peasy/{}/peapod/{}/pea.json",
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
            "  environment.etc.{}.source = pkgs.fetchurl {{ url = {}; sha256 = {}; curlOptsList = [ \"--max-filesize\" \"65536\" \"--max-time\" \"30\" \"--max-redirs\" \"0\" ]; }};\n",
            nix_string(&format!("peasy/peas/{}.json", p.id)),
            nix_string(&p.url()),
            nix_string(&p.hash)
        ));
    }
    out
}
// Exact previous renderer, only for migrating already-recorded pins. The next
// managed write uses bounded fetches; arbitrary edits still fail closed.
pub(crate) fn legacy_render(pins: &[PeaPin]) -> String {
    render(pins).replace(" curlOptsList = [ \"--max-filesize\" \"65536\" \"--max-time\" \"30\" \"--max-redirs\" \"0\" ];", "")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn api5_pins_cannot_acquire_application_launch_authority() {
        let mut manifest = manifest_for_api(5);
        let action = ModelAction::ChangeResources {
            change: crate::ResourceChange::OpenApplication {
                desktop_id: "org.telegram.desktop.desktop".into(),
            },
        };
        assert!(!manifest.permits(&action));
        manifest.permissions = vec!["applications.write".into()];
        manifest.response_schema = schema_for_api(&manifest.permissions, 5);
        assert!(manifest.validate().is_err());
        assert!(!manifest.permits(&action));
        manifest.host_api = HOST_API;
        manifest.response_schema = schema_for_api(&manifest.permissions, HOST_API);
        manifest.validate().unwrap();
        assert!(manifest.permits(&action));
    }
    #[test]
    fn api4_pins_keep_original_power_and_display_limits() {
        for (id, change) in [
            (
                "power",
                crate::ResourceChange::PowerSettings {
                    lid: None,
                    idle_minutes: Some(30),
                },
            ),
            (
                "displays",
                crate::ResourceChange::Display {
                    connector: "DP-1".into(),
                    mode: "1920x1080@60".into(),
                    scale_percent: 100,
                    x: -1920,
                    y: 0,
                    primary: false,
                },
            ),
        ] {
            let mut manifest = manifest_for_api(4);
            manifest.permissions = vec![format!("{id}.write")];
            manifest.response_schema = schema_for_api(&manifest.permissions, 4);
            manifest.validate().unwrap();
            let action = ModelAction::ChangeResources { change };
            assert!(!manifest.permits(&action));
            manifest.host_api = HOST_API;
            manifest.response_schema = schema_for_api(&manifest.permissions, HOST_API);
            assert!(manifest.permits(&action));
        }
    }
    fn manifest_for_api(api: u32) -> PeaManifest {
        let mut manifest: PeaManifest =
            serde_json::from_str(include_str!("../../../peapod/tests/api2-packages.json")).unwrap();
        manifest.host_api = api;
        manifest.response_schema = schema_for_api(&manifest.permissions, api);
        manifest.validate().unwrap();
        manifest
    }

    #[test]
    fn legacy_peas_cannot_gain_new_setup_options_or_groups() {
        for (enable, groups) in [
            (
                vec!["virtualisation.docker.enable".into()],
                vec!["docker".into()],
            ),
            (vec!["programs.direnv.enable".into()], vec![]),
            (vec![], vec!["video".into()]),
        ] {
            let setup = crate::SystemSetup {
                packages: vec![],
                enable,
                groups,
                postgresql: None,
            };
            setup.validate().unwrap(); // Valid host actions are not necessarily valid pea actions.
            let action = ModelAction::InstallPackage {
                package: "hello".into(),
                message: None,
                setup: Some(setup),
            };
            assert!(!manifest_for_api(1).permits(&action));
            assert!(!manifest_for_api(2).permits(&action));
            assert!(manifest_for_api(3).permits(&action));
        }
        let action = ModelAction::InstallPackage {
            package: "virt-manager".into(),
            message: None,
            setup: Some(crate::SystemSetup {
                packages: vec![],
                enable: vec!["virtualisation.libvirtd.enable".into()],
                groups: vec!["libvirtd".into()],
                postgresql: None,
            }),
        };
        for api in 1..=3 {
            assert!(manifest_for_api(api).permits(&action));
        }
    }

    #[test]
    fn postgres_and_message_limits_follow_the_declared_api() {
        let action = ModelAction::InstallPackage {
            package: "postgresql_17".into(),
            message: None,
            setup: Some(crate::SystemSetup {
                packages: vec![],
                enable: vec![],
                groups: vec![],
                postgresql: Some(crate::PostgresqlSetup {
                    package: "postgresql_17".into(),
                    caller_database: true,
                }),
            }),
        };
        assert!(!manifest_for_api(1).permits(&action));
        assert!(manifest_for_api(2).permits(&action));
        assert!(manifest_for_api(3).permits(&action));
        for length in [400, 401] {
            for action in [
                ModelAction::Explain {
                    message: "é".repeat(length),
                },
                ModelAction::InstallPackage {
                    package: "hello".into(),
                    setup: None,
                    message: Some("é".repeat(length)),
                },
            ] {
                assert_eq!(manifest_for_api(1).permits(&action), length == 400);
                assert_eq!(manifest_for_api(2).permits(&action), length == 400);
                assert!(manifest_for_api(3).permits(&action));
            }
        }
        let mut invalid = manifest_for_api(2);
        invalid.host_api = 99;
        assert!(!invalid.permits(&ModelAction::Cancel));
    }

    #[test]
    fn original_api_one_pins_remain_compatible_without_widening_permissions() {
        let mut manifest: PeaManifest =
            serde_json::from_str(include_str!("../../../peapod/tests/api2-packages.json")).unwrap();
        manifest.validate().unwrap();
        manifest.host_api = 1;
        manifest.response_schema["properties"]["setup"]["properties"]
            .as_object_mut()
            .unwrap()
            .remove("postgresql");
        manifest.response_schema["properties"]["setup"]["required"]
            .as_array_mut()
            .unwrap()
            .retain(|field| field != "postgresql");
        manifest.validate().unwrap();
        manifest.response_schema["properties"]["action"]["enum"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!("configure_network"));
        assert!(manifest.validate().is_err());
    }
    #[test]
    fn expanded_catalogue_requires_api_three() {
        let mut old: PeaManifest =
            serde_json::from_str(include_str!("../../../peapod/tests/api2-packages.json")).unwrap();
        old.validate().unwrap();
        old.response_schema["properties"]["setup"]["properties"]["enable"]["items"]["enum"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!("virtualisation.docker.enable"));
        assert!(old.validate().is_err());
        let mut current: PeaManifest =
            serde_json::from_str(include_str!("../../../peapod/packages/pea.json")).unwrap();
        current.validate().unwrap();
        current.host_api = 2;
        assert!(current.validate().is_err());
    }

    #[test]
    fn persistent_activation_requires_both_network_permissions() {
        let mut manifest: PeaManifest =
            serde_json::from_str(include_str!("../../../peapod/networking/pea.json")).unwrap();
        let mut plan: crate::NetworkPlan =
            serde_json::from_str(include_str!("../../../peapod/networking/example.json")).unwrap();
        manifest.permissions = vec!["network.system".into()];
        manifest.response_schema = schema_for_api(&manifest.permissions, manifest.host_api);
        assert!(!manifest.permits(&ModelAction::ConfigureNetwork { plan: plan.clone() }));
        manifest.permissions.push("network.session".into());
        manifest.response_schema = schema_for_api(&manifest.permissions, manifest.host_api);
        assert!(manifest.permits(&ModelAction::ConfigureNetwork { plan: plan.clone() }));
        manifest.permissions.retain(|p| p != "network.session");
        manifest.response_schema = schema_for_api(&manifest.permissions, manifest.host_api);
        plan.activate = None;
        assert!(manifest.permits(&ModelAction::ConfigureNetwork { plan }));
    }
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
        let old = text.replace(&render(&state.peas), &legacy_render(&state.peas));
        assert_eq!(crate::parse_packages_module(&old).unwrap(), state);
        assert!(crate::parse_packages_module(&(old + "\n# unexpected edit\n")).is_err());
        assert!(text.contains("pkgs.fetchurl"));
        assert!(!text.contains("builtins.getFlake"));
        assert_eq!(state.with_pea(&p, false).unwrap(), PackageState::default());
    }
}

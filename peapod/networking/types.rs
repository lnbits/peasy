//! Network host API v1: resources and declarative values, never commands or Nix.
use crate::{PackageState, ValidationError, nix_string, validate_ssid};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::net::Ipv4Addr;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkKind {
    Ethernet,
    Wifi,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WifiMode {
    Infrastructure,
    Ap,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Ipv4Method {
    Auto,
    Manual,
    Shared,
    Disabled,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkScope {
    Session,
    System,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkProfile {
    pub id: String,
    pub interface: String,
    pub kind: NetworkKind,
    pub wifi_mode: Option<WifiMode>,
    pub ssid: Option<String>,
    pub ipv4: Ipv4Method,
    pub addresses: Vec<String>,
    pub gateway: Option<String>,
    pub dns: Vec<String>,
    pub autoconnect: bool,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkPlan {
    pub scope: NetworkScope,
    pub profiles: Vec<NetworkProfile>,
    pub remove: Vec<String>,
    /// A discovered UUID, or the id of the new session profile.
    pub activate: Option<String>,
    pub deactivate: Option<String>,
}
fn invalid(message: &str) -> ValidationError {
    ValidationError::InvalidRequest(message.into())
}
pub fn validate_network_id(id: &str) -> Result<(), ValidationError> {
    if id.is_empty()
        || id.len() > 48
        || !id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_'))
    {
        return Err(invalid("invalid network profile id"));
    }
    Ok(())
}
pub fn validate_interface(id: &str) -> Result<(), ValidationError> {
    if id.is_empty()
        || id.len() > 15
        || id.starts_with('-')
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
    {
        return Err(invalid("invalid network interface"));
    }
    Ok(())
}
pub fn validate_network_uuid(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}
impl NetworkProfile {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_network_id(&self.id)?;
        validate_interface(&self.interface)?;
        match self.kind {
            NetworkKind::Ethernet if self.wifi_mode.is_some() || self.ssid.is_some() => {
                return Err(invalid("Ethernet cannot have Wi-Fi settings"));
            }
            NetworkKind::Wifi => {
                validate_ssid(
                    self.ssid
                        .as_deref()
                        .ok_or_else(|| invalid("Wi-Fi needs an SSID"))?,
                )?;
                if self.wifi_mode.is_none() {
                    return Err(invalid("Wi-Fi needs a mode"));
                }
            }
            _ => {}
        }
        if self.addresses.len() > 4 || self.dns.len() > 4 {
            return Err(invalid("too many network addresses"));
        }
        for address in &self.addresses {
            let (ip, prefix) = address
                .split_once('/')
                .ok_or_else(|| invalid("address needs an IPv4 prefix"))?;
            ip.parse::<Ipv4Addr>()
                .map_err(|_| invalid("invalid IPv4 address"))?;
            if prefix
                .parse::<u8>()
                .ok()
                .filter(|n| *n > 0 && *n <= 32)
                .is_none()
            {
                return Err(invalid("invalid IPv4 prefix"));
            }
        }
        for ip in self.dns.iter().chain(self.gateway.iter()) {
            ip.parse::<Ipv4Addr>()
                .map_err(|_| invalid("invalid IPv4 address"))?;
        }
        if (self.ipv4 == Ipv4Method::Manual && self.addresses.is_empty())
            || (matches!(self.ipv4, Ipv4Method::Auto | Ipv4Method::Disabled)
                && !self.addresses.is_empty())
            || (self.ipv4 != Ipv4Method::Manual && self.gateway.is_some())
            || (matches!(self.ipv4, Ipv4Method::Disabled | Ipv4Method::Shared)
                && !self.dns.is_empty())
        {
            return Err(invalid("inconsistent IPv4 settings"));
        }
        Ok(())
    }
    pub fn connection_id(&self) -> String {
        format!("peasy-{}", self.id)
    }
    /// Stable UUID in a Peasy-reserved namespace; IDs longer than 16 bytes use
    /// two independent FNV streams. Collisions are checked against discovered profiles.
    pub fn uuid(&self) -> String {
        let hash = |seed: u64| {
            self.id
                .bytes()
                .fold(seed, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100000001b3))
        };
        let hex = format!(
            "{:016x}{:016x}",
            hash(0xcbf29ce484222325),
            hash(0x84222325cbf29ce4)
        );
        format!(
            "{}-{}-5{}-a{}-{}",
            &hex[..8],
            &hex[8..12],
            &hex[13..16],
            &hex[17..20],
            &hex[20..]
        )
    }
    pub fn properties(&self) -> Vec<(&'static str, String)> {
        let mut out = vec![
            ("connection.id", self.connection_id()),
            ("connection.uuid", self.uuid()),
            ("connection.interface-name", self.interface.clone()),
            (
                "connection.type",
                match self.kind {
                    NetworkKind::Ethernet => "802-3-ethernet",
                    NetworkKind::Wifi => "802-11-wireless",
                }
                .into(),
            ),
            ("connection.autoconnect", self.autoconnect.to_string()),
            (
                "ipv4.method",
                match self.ipv4 {
                    Ipv4Method::Auto => "auto",
                    Ipv4Method::Manual => "manual",
                    Ipv4Method::Shared => "shared",
                    Ipv4Method::Disabled => "disabled",
                }
                .into(),
            ),
            ("ipv6.method", "disabled".into()),
        ];
        if !self.addresses.is_empty() {
            out.push(("ipv4.addresses", self.addresses.join(",")));
        }
        if let Some(gateway) = &self.gateway {
            out.push(("ipv4.gateway", gateway.clone()));
        }
        if !self.dns.is_empty() {
            out.push(("ipv4.dns", self.dns.join(",")));
            out.push(("ipv4.ignore-auto-dns", "true".into()));
        }
        if let Some(ssid) = &self.ssid {
            out.extend([
                ("802-11-wireless.ssid", ssid.clone()),
                (
                    "802-11-wireless.mode",
                    if self.wifi_mode == Some(WifiMode::Ap) {
                        "ap"
                    } else {
                        "infrastructure"
                    }
                    .into(),
                ),
                ("802-11-wireless-security.key-mgmt", "wpa-psk".into()),
                ("802-11-wireless-security.proto", "rsn".into()),
                ("802-11-wireless-security.psk-flags", "2".into()),
            ]);
        }
        out
    }
}
impl NetworkPlan {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.profiles.len() > 8 || self.remove.len() > 8 {
            return Err(invalid("network plan is too large"));
        }
        let mut ids = BTreeSet::new();
        let mut interfaces = BTreeSet::new();
        for p in &self.profiles {
            p.validate()?;
            if !ids.insert(&p.id) || !interfaces.insert(&p.interface) {
                return Err(invalid("duplicate network resource"));
            }
        }
        for id in &self.remove {
            validate_network_id(id)?;
            if !ids.insert(id) {
                return Err(invalid("duplicate network resource"));
            }
        }
        match self.scope {
            NetworkScope::System => {
                if self.deactivate.is_some()
                    || ids.is_empty()
                    || self
                        .activate
                        .as_ref()
                        .is_some_and(|id| !self.profiles.iter().any(|p| &p.id == id))
                {
                    return Err(invalid(
                        "persistent plans may activate one of their declared profiles after the system review",
                    ));
                }
            }
            NetworkScope::Session => {
                if !self.remove.is_empty()
                    || self.profiles.len() > 1
                    || self.activate.is_some() == self.deactivate.is_some()
                {
                    return Err(invalid(
                        "session plans perform one activation or deactivation",
                    ));
                }
                if let Some(p) = self.profiles.first() {
                    if self.activate.as_deref() != Some(&p.id) || p.autoconnect {
                        return Err(invalid(
                            "temporary profile must be activated explicitly without autoconnect",
                        ));
                    }
                } else if !self
                    .activate
                    .iter()
                    .chain(self.deactivate.iter())
                    .all(|id| validate_network_uuid(id))
                {
                    return Err(invalid("activation requires a discovered connection UUID"));
                }
            }
        }
        Ok(())
    }
    pub fn password_required(&self) -> bool {
        self.activate.is_some() && self.profiles.iter().any(|p| p.kind == NetworkKind::Wifi)
    }
}
impl PackageState {
    pub fn with_network(&self, plan: &NetworkPlan) -> Result<Self, ValidationError> {
        plan.validate()?;
        if plan.scope != NetworkScope::System {
            return Err(invalid("only system profiles are declarative"));
        }
        let mut after = self.clone();
        for id in &plan.remove {
            if !after.networks.iter().any(|p| &p.id == id) {
                return Err(invalid("Peasy does not own that network profile"));
            }
            after.networks.retain(|p| &p.id != id);
        }
        for profile in &plan.profiles {
            after.networks.retain(|p| p.id != profile.id);
            after.networks.push(profile.clone());
        }
        after.normalize()?;
        Ok(after)
    }
}
pub(crate) fn render(profiles: &[NetworkProfile]) -> String {
    if profiles.is_empty() {
        return String::new();
    }
    let mut out = String::from("\n  networking.networkmanager.enable = true;\n");
    for p in profiles {
        out.push_str(&format!(
            "  environment.etc.{}.text = lib.generators.toINI {{}} {{\n",
            nix_string(&format!(
                "NetworkManager/system-connections/{}.nmconnection",
                p.connection_id()
            ))
        ));
        out.push_str(&format!("    connection = {{ id = {}; uuid = {}; type = {}; interface-name = {}; autoconnect = {}; }};\n", nix_string(&p.connection_id()), nix_string(&p.uuid()), nix_string(match p.kind { NetworkKind::Ethernet => "ethernet", NetworkKind::Wifi => "wifi" }), nix_string(&p.interface), p.autoconnect));
        let method = match p.ipv4 {
            Ipv4Method::Auto => "auto",
            Ipv4Method::Manual => "manual",
            Ipv4Method::Shared => "shared",
            Ipv4Method::Disabled => "disabled",
        };
        out.push_str(&format!("    ipv4 = {{ method = {};", nix_string(method)));
        for (i, a) in p.addresses.iter().enumerate() {
            let value = if i == 0 {
                p.gateway
                    .as_ref()
                    .map(|g| format!("{a},{g}"))
                    .unwrap_or(a.clone())
            } else {
                a.clone()
            };
            out.push_str(&format!(" address{} = {};", i + 1, nix_string(&value)));
        }
        if !p.dns.is_empty() {
            out.push_str(&format!(
                " dns = {}; ignore-auto-dns = true;",
                nix_string(&p.dns.join(";"))
            ));
        }
        out.push_str(" };\n    ipv6.method = \"disabled\";\n");
        if let Some(ssid) = &p.ssid {
            out.push_str(&format!("    wifi = {{ ssid = {}; mode = {}; }};\n    wifi-security = {{ key-mgmt = \"wpa-psk\"; proto = \"rsn\"; psk-flags = 2; }};\n", nix_string(ssid), nix_string(if p.wifi_mode == Some(WifiMode::Ap) { "ap" } else { "infrastructure" })));
        }
        out.push_str("  };\n");
        out.push_str(&format!(
            "  environment.etc.{}.mode = \"0600\";\n",
            nix_string(&format!(
                "NetworkManager/system-connections/{}.nmconnection",
                p.connection_id()
            ))
        ));
    }
    for interface in profiles
        .iter()
        .filter(|p| p.ipv4 == Ipv4Method::Shared)
        .map(|p| &p.interface)
        .collect::<BTreeSet<_>>()
    {
        out.push_str(&format!("  networking.firewall.interfaces.{}.allowedUDPPorts = [ 53 67 ];\n  networking.firewall.interfaces.{}.allowedTCPPorts = [ 53 ];\n", nix_string(interface), nix_string(interface)));
    }
    out
}

use serde_json::{Value, json};
pub fn network_schema() -> Value {
    let nullable = |kind: &str| json!({"type":[kind,"null"]});
    json!({"type":["object","null"],"additionalProperties":false,"properties":{
        "scope":{"type":"string","enum":["session","system"]},
        "profiles":{"type":"array","maxItems":8,"items":{"type":"object","additionalProperties":false,"properties":{
            "id":{"type":"string","maxLength":48},"interface":{"type":"string","maxLength":15},
            "kind":{"type":"string","enum":["ethernet","wifi"]},
            "wifi_mode":{"type":["string","null"],"enum":["infrastructure","ap",null]},"ssid":nullable("string"),
            "ipv4":{"type":"string","enum":["auto","manual","shared","disabled"]},
            "addresses":{"type":"array","maxItems":4,"items":{"type":"string"}},"gateway":nullable("string"),
            "dns":{"type":"array","maxItems":4,"items":{"type":"string"}},"autoconnect":{"type":"boolean"}
        },"required":["id","interface","kind","wifi_mode","ssid","ipv4","addresses","gateway","dns","autoconnect"]}},
        "remove":{"type":"array","maxItems":8,"items":{"type":"string","maxLength":48}},"activate":nullable("string"),"deactivate":nullable("string")
    },"required":["scope","profiles","remove","activate","deactivate"]})
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ethernet() -> NetworkProfile {
        NetworkProfile {
            id: "wired".into(),
            interface: "enp1s0".into(),
            kind: NetworkKind::Ethernet,
            wifi_mode: None,
            ssid: None,
            ipv4: Ipv4Method::Auto,
            addresses: vec![],
            gateway: None,
            dns: vec![],
            autoconnect: true,
        }
    }
    #[test]
    fn rejects_inconsistent_and_executable_values() {
        let mut p = ethernet();
        p.interface = "$(id)".into();
        assert!(p.validate().is_err());
        p = ethernet();
        p.ipv4 = Ipv4Method::Manual;
        assert!(p.validate().is_err());
        p.addresses.push("192.168.1.2/24".into());
        p.gateway = Some("192.168.1.1".into());
        assert!(p.validate().is_ok());
        p.dns.push("8.8.8.8;exec".into());
        assert!(p.validate().is_err());
        assert!(serde_json::from_value::<NetworkProfile>(json!({"command":"nmcli"})).is_err());
    }
    #[test]
    fn persistent_profiles_roundtrip_and_remove_only_owned_state() {
        let p = ethernet();
        assert!(validate_network_uuid(&p.uuid()));
        let mut plan = NetworkPlan {
            scope: NetworkScope::System,
            profiles: vec![p],
            remove: vec![],
            activate: None,
            deactivate: None,
        };
        let state = PackageState::default().with_network(&plan).unwrap();
        let module = crate::render_packages_module(&state).unwrap();
        assert_eq!(crate::parse_packages_module(&module).unwrap(), state);
        assert!(module.contains("0600"));
        assert!(!module.contains("psk ="));
        plan.profiles.clear();
        plan.remove.push("stranger".into());
        assert!(state.with_network(&plan).is_err());
        plan.remove = vec!["wired".into()];
        assert_eq!(state.with_network(&plan).unwrap(), PackageState::default());
    }
    #[test]
    fn session_profiles_cannot_persist_or_mutate_multiple_resources() {
        let mut p = ethernet();
        let mut plan = NetworkPlan {
            scope: NetworkScope::Session,
            profiles: vec![p.clone()],
            remove: vec![],
            activate: Some(p.id.clone()),
            deactivate: None,
        };
        assert!(plan.validate().is_err());
        p.autoconnect = false;
        plan.profiles = vec![p];
        assert!(plan.validate().is_ok());
        plan.deactivate = Some("12345678-1234-1234-1234-123456789abc".into());
        assert!(plan.validate().is_err());
    }
}

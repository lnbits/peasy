//! Closed resource operations shared by the management peas. Never commands.
use crate::{ValidationError, nix_string};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceDomain {
    Diagnostics,
    Services,
    Storage,
    NixMaintenance,
    Users,
    Firewall,
    Printing,
    Displays,
    Audio,
    Power,
}
impl ResourceDomain {
    pub fn id(self) -> &'static str {
        match self {
            Self::Diagnostics => "diagnostics",
            Self::Services => "services",
            Self::Storage => "storage",
            Self::NixMaintenance => "nix_maintenance",
            Self::Users => "users",
            Self::Firewall => "firewall",
            Self::Printing => "printing",
            Self::Displays => "displays",
            Self::Audio => "audio",
            Self::Power => "power",
        }
    }
    pub fn session(self) -> bool {
        matches!(
            self,
            Self::Printing | Self::Displays | Self::Audio | Self::Power | Self::Storage
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceQuery {
    pub domain: ResourceDomain,
    pub target: Option<String>,
}
impl ResourceQuery {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if let Some(target) = &self.target {
            if self.domain == ResourceDomain::Services {
                unit(target)?;
            } else {
                return Err(invalid("only service inspection accepts a target"));
            }
        }
        Ok(())
    }
}

macro_rules! choices {
    ($name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($variant),+ }
        impl $name { pub fn value(self) -> &'static str { match self { $(Self::$variant => $value),+ } } }
    };
}
choices!(ServiceAction { Start => "start", Stop => "stop", Restart => "restart" });
choices!(DiskAction { Mount => "mount", Unmount => "unmount", Format => "format" });
choices!(FileSystem { Ext4 => "ext4", Vfat => "vfat", Exfat => "exfat" });
choices!(AudioAction { Default => "set-default", Volume => "set-volume", Mute => "set-mute", Unmute => "set-mute" });
choices!(PowerProfile { Balanced => "balanced", PowerSaver => "power-saver", Performance => "performance" });
choices!(LidAction { Ignore => "ignore", Suspend => "suspend", Hibernate => "hibernate" });
choices!(PrinterAction { Add => "add", Default => "default", Test => "test" });
choices!(ManagedService { Ssh => "services.openssh.enable", Caddy => "services.caddy.enable", Tailscale => "services.tailscale.enable", Printing => "services.printing.enable", Bluetooth => "hardware.bluetooth.enable", Docker => "virtualisation.docker.enable", Podman => "virtualisation.podman.enable", Libvirtd => "virtualisation.libvirtd.enable" });

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResourceChange {
    Service {
        unit: String,
        action: ServiceAction,
    },
    ServiceEnabled {
        service: ManagedService,
        enabled: bool,
    },
    Disk {
        device: String,
        action: DiskAction,
        filesystem: Option<FileSystem>,
    },
    PersistentMount {
        uuid: String,
        name: String,
        filesystem: FileSystem,
        present: bool,
    },
    Firewall {
        tcp: Vec<u16>,
        udp: Vec<u16>,
        trusted_interfaces: Vec<String>,
    },
    UserCreate {
        name: String,
    },
    UserDisabled {
        name: String,
        disabled: bool,
    },
    UserGroups {
        groups: Vec<String>,
    },
    NixOptimise {},
    NixGarbageCollect {},
    NixDeleteGenerations {
        generations: Vec<u64>,
    },
    Audio {
        id: u32,
        action: AudioAction,
        volume: Option<u8>,
    },
    PowerProfile {
        profile: PowerProfile,
    },
    PowerSettings {
        lid: LidAction,
        idle_minutes: u16,
    },
    Printer {
        name: String,
        action: PrinterAction,
        uri: Option<String>,
    },
    Display {
        connector: String,
        mode: String,
        scale_percent: u16,
        x: u16,
        y: u16,
        primary: bool,
    },
}
fn invalid(message: &str) -> ValidationError {
    ValidationError::InvalidRequest(message.into())
}
pub fn identifier(s: &str, max: usize) -> Result<(), ValidationError> {
    if s.is_empty()
        || s.len() > max
        || !s.as_bytes()[0].is_ascii_alphanumeric()
        || !s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-.".contains(&b))
        || s.contains("..")
    {
        return Err(invalid("invalid resource identifier"));
    }
    Ok(())
}
pub fn unit(s: &str) -> Result<(), ValidationError> {
    identifier(s, 160)?;
    if !s.ends_with(".service") {
        return Err(invalid("an exact .service unit is required"));
    }
    Ok(())
}
pub fn username(s: &str) -> Result<(), ValidationError> {
    if s.is_empty()
        || s.len() > 32
        || !s.as_bytes()[0].is_ascii_lowercase()
        || !s
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_-".contains(&b))
        || ["root", "nobody", "nixbld", "peasy"].contains(&s)
    {
        return Err(invalid("invalid normal user name"));
    }
    Ok(())
}
impl ResourceChange {
    pub fn domain(&self) -> ResourceDomain {
        match self {
            Self::Service { .. } | Self::ServiceEnabled { .. } => ResourceDomain::Services,
            Self::Disk { .. } | Self::PersistentMount { .. } => ResourceDomain::Storage,
            Self::Firewall { .. } => ResourceDomain::Firewall,
            Self::UserCreate { .. } | Self::UserDisabled { .. } | Self::UserGroups { .. } => {
                ResourceDomain::Users
            }
            Self::NixOptimise {}
            | Self::NixGarbageCollect {}
            | Self::NixDeleteGenerations { .. } => ResourceDomain::NixMaintenance,
            Self::Audio { .. } => ResourceDomain::Audio,
            Self::PowerProfile { .. } | Self::PowerSettings { .. } => ResourceDomain::Power,
            Self::Printer { .. } => ResourceDomain::Printing,
            Self::Display { .. } => ResourceDomain::Displays,
        }
    }
    pub fn persistent(&self) -> bool {
        matches!(
            self,
            Self::ServiceEnabled { .. }
                | Self::PersistentMount { .. }
                | Self::Firewall { .. }
                | Self::UserCreate { .. }
                | Self::UserDisabled { .. }
                | Self::UserGroups { .. }
                | Self::PowerSettings { .. }
        )
    }
    pub fn privileged(&self) -> bool {
        self.persistent()
            || matches!(
                self,
                Self::Service { .. }
                    | Self::NixOptimise {}
                    | Self::NixGarbageCollect {}
                    | Self::NixDeleteGenerations { .. }
            )
    }
    pub fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::Service { unit: name, .. } => unit(name)?,
            Self::Disk {
                device,
                action,
                filesystem,
            } => {
                let name = device
                    .strip_prefix("/dev/")
                    .ok_or_else(|| invalid("a discovered block device is required"))?;
                identifier(name, 64)?;
                if (*action == DiskAction::Format) != filesystem.is_some() {
                    return Err(invalid("only formatting requires a filesystem"));
                }
            }
            Self::PersistentMount { uuid, name, .. } => {
                identifier(uuid, 64)?;
                identifier(name, 48)?;
            }
            Self::Firewall {
                tcp,
                udp,
                trusted_interfaces,
            } => {
                if tcp.len() > 64
                    || udp.len() > 64
                    || trusted_interfaces.len() > 8
                    || tcp.contains(&0)
                    || udp.contains(&0)
                {
                    return Err(invalid("firewall limits exceeded"));
                }
                for name in trusted_interfaces {
                    crate::validate_interface(name)?;
                }
            }
            Self::UserCreate { name } | Self::UserDisabled { name, .. } => username(name)?,
            Self::UserGroups { groups } => {
                if groups.len() > 16
                    || groups.iter().any(|g| {
                        ![
                            "audio",
                            "video",
                            "render",
                            "dialout",
                            "lp",
                            "scanner",
                            "networkmanager",
                            "docker",
                            "libvirtd",
                            "wireshark",
                        ]
                        .contains(&g.as_str())
                    })
                {
                    return Err(invalid("unsupported caller group"));
                }
            }
            Self::NixDeleteGenerations { generations } => {
                if generations.is_empty() || generations.len() > 32 || generations.contains(&0) {
                    return Err(invalid("select 1–32 exact generations"));
                }
            }
            Self::Audio { id, action, volume } => {
                if *id == 0
                    || (*action == AudioAction::Volume) != volume.is_some()
                    || volume.is_some_and(|v| v > 100)
                {
                    return Err(invalid("invalid audio target or volume"));
                }
            }
            Self::PowerSettings { idle_minutes, .. } => {
                if *idle_minutes > 1440 {
                    return Err(invalid("idle timeout exceeds one day"));
                }
            }
            Self::Printer { name, action, uri } => {
                identifier(name, 64)?;
                if (*action == PrinterAction::Add) != uri.is_some() {
                    return Err(invalid("only adding a printer requires a discovered URI"));
                }
                if let Some(uri) = uri
                    && (uri.len() > 512
                        || !(uri.starts_with("ipp://") || uri.starts_with("ipps://"))
                        || uri
                            .bytes()
                            .any(|b| b.is_ascii_control() || b.is_ascii_whitespace())
                        || uri.contains(['@', '?', '#']))
                {
                    return Err(invalid(
                        "only credential-free discovered IPP printers are supported",
                    ));
                }
            }
            Self::Display {
                connector,
                mode,
                scale_percent,
                x,
                y,
                ..
            } => {
                identifier(connector, 64)?;
                if mode.is_empty()
                    || mode.len() > 48
                    || !mode
                        .bytes()
                        .all(|b| b.is_ascii_digit() || b"x@.".contains(&b))
                    || !mode.contains('x')
                    || !(50..=300).contains(scale_percent)
                    || *x > 16384
                    || *y > 16384
                {
                    return Err(invalid("invalid display layout"));
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub fn summary(&self) -> String {
        match self {
            Self::Service { unit, action } => format!("{} {}", action.value(), unit),
            Self::ServiceEnabled { service, enabled } => format!(
                "{} {}",
                if *enabled {
                    "Enable"
                } else {
                    "Withdraw Peasy enablement for"
                },
                service.value()
            ),
            Self::Disk {
                device,
                action,
                filesystem,
            } => format!(
                "{} {}{}",
                action.value(),
                device,
                filesystem
                    .map(|f| format!(" as {}", f.value()))
                    .unwrap_or_default()
            ),
            Self::PersistentMount {
                uuid,
                name,
                present,
                ..
            } => format!(
                "{} UUID {} at /mnt/peasy-{}",
                if *present {
                    "Mount"
                } else {
                    "Remove mount for"
                },
                uuid,
                name
            ),
            Self::Firewall {
                tcp,
                udp,
                trusted_interfaces,
            } => format!(
                "Set Peasy firewall ports: TCP {tcp:?}; UDP {udp:?}; trusted interfaces {trusted_interfaces:?}"
            ),
            Self::UserCreate { name } => format!("Create locked local account {name}"),
            Self::UserDisabled { name, disabled } => format!(
                "{} account {name}",
                if *disabled {
                    "Disable login for"
                } else {
                    "Re-enable"
                }
            ),
            Self::UserGroups { groups } => format!(
                "Set your Peasy-managed groups to {}",
                if groups.is_empty() {
                    "none".into()
                } else {
                    groups.join(", ")
                }
            ),
            Self::NixOptimise {} => "Optimise the Nix store".into(),
            Self::NixGarbageCollect {} => "Collect unreferenced Nix store paths".into(),
            Self::NixDeleteGenerations { generations } => format!(
                "Remove NixOS generation references {}",
                generations
                    .iter()
                    .map(u64::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Audio { id, action, volume } => format!(
                "{} audio device {}{}",
                match action {
                    AudioAction::Default => "Set default",
                    AudioAction::Volume => "Set volume for",
                    AudioAction::Mute => "Mute",
                    AudioAction::Unmute => "Unmute",
                },
                id,
                volume.map(|v| format!(" to {v}%")).unwrap_or_default()
            ),
            Self::PowerProfile { profile } => format!("Select {} power profile", profile.value()),
            Self::PowerSettings { lid, idle_minutes } => format!(
                "Lid action: {}; idle suspension: {}",
                lid.value(),
                if *idle_minutes == 0 {
                    "disabled".into()
                } else {
                    format!("after {idle_minutes} minutes")
                }
            ),
            Self::Printer { name, action, uri } => format!(
                "{} printer {}{}",
                action.value(),
                name,
                uri.as_ref().map(|u| format!(" ({u})")).unwrap_or_default()
            ),
            Self::Display {
                connector,
                mode,
                scale_percent,
                x,
                y,
                primary,
            } => format!(
                "Set {connector} to {mode}, {scale_percent}% scale, position {x},{y}{}",
                if *primary { ", primary display" } else { "" }
            ),
        }
    }
    pub fn note(&self) -> &'static str {
        match self {
            Self::Disk {
                action: DiskAction::Format,
                ..
            } => {
                "ERASES ALL DATA on the selected removable filesystem. NixOS rollback cannot recover it. No partition table is changed."
            }
            Self::Disk { .. } => {
                "Changes a removable filesystem's live mount state through UDisks authorization. NixOS rollback does not undo this."
            }
            Self::Service { .. } => {
                "Changes a system service now. Connections and work may be interrupted; service restart policy may start it again. No boot setting changes."
            }
            Self::ServiceEnabled { enabled: false, .. } => {
                "Withdraws Peasy's enablement only. Other configuration may keep the service enabled; service data is retained."
            }
            Self::ServiceEnabled { .. } => {
                "Enables the NixOS service using its defaults. Review network exposure and access; application-specific settings may still be needed."
            }
            Self::PersistentMount { .. } => {
                "Manages /mnt/peasy-NAME by filesystem UUID. Uses nofail and nodev,nosuid. Existing mounts and data are preserved; boot can continue without the device."
            }
            Self::Firewall { .. } => {
                "Replaces Peasy's firewall contribution. Trusted interfaces allow all incoming traffic. Administrator rules remain; removing a Peasy port does not guarantee it is closed."
            }
            Self::UserCreate { .. } => {
                "Creates a normal local account with a locked initial password and no administrator groups. Set its password separately with the system's local account tools. Home data survives rollback."
            }
            Self::UserDisabled { .. } => {
                "Controls login for a Peasy-created account only. Disabling locks password login, sets nologin and denies SSH. Existing sessions are not terminated. Re-enabling does not set a password."
            }
            Self::UserGroups { .. } => {
                "Replaces Peasy's supplementary groups for the authenticated caller. Docker and libvirtd grant powerful host access. Administrator and package-setup groups remain. Log out and back in."
            }
            Self::NixOptimise {} => {
                "Deduplicates identical Nix store files. Can use substantial disk I/O; does not remove generations."
            }
            Self::NixGarbageCollect {} => {
                "Deletes unreferenced Nix store paths. Existing generation roots are retained. Collected paths may need downloading or building again."
            }
            Self::NixDeleteGenerations { .. } => {
                "Permanently removes these rollback references. Current and booted generations and at least one other generation are protected. Store contents are not garbage-collected automatically."
            }
            Self::Audio { .. } => {
                "Changes WirePlumber audio state. WirePlumber may remember defaults or volume; NixOS rollback does not restore them."
            }
            Self::PowerProfile { .. } => {
                "Selects a live power-profiles-daemon profile, subject to hardware support and desktop authorization."
            }
            Self::PowerSettings { .. } => {
                "Changes system logind idle and lid policy. Desktop inhibitors may override it; zero disables idle suspension. Hibernate requires a working host hibernation setup."
            }
            Self::Printer {
                action: PrinterAction::Add,
                ..
            } => {
                "Creates a driverless CUPS queue for the discovered IPP printer. CUPS authorization applies. Queue state is not restored by NixOS rollback."
            }
            Self::Printer {
                action: PrinterAction::Test,
                ..
            } => "Sends one fixed test page to this printer; consumes paper and ink.",
            Self::Printer { .. } => "Changes the current user's CUPS default printer.",
            Self::Display { .. } => {
                "Changes the live display layout. Desktop persistence varies. Keep the desktop's display settings available to restore an unsuitable layout."
            }
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceState {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<ManagedService>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mounts: Vec<ManagedMount>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub firewall: Option<FirewallState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub users: Vec<ManagedUser>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<CallerGroups>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power: Option<PowerSettings>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedMount {
    pub uuid: String,
    pub name: String,
    pub filesystem: FileSystem,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FirewallState {
    pub tcp: Vec<u16>,
    pub udp: Vec<u16>,
    pub trusted_interfaces: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedUser {
    pub name: String,
    pub uid: u32,
    pub disabled: bool,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CallerGroups {
    pub name: String,
    pub uid: u32,
    pub groups: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PowerSettings {
    pub lid: LidAction,
    pub idle_minutes: u16,
}
impl ResourceState {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.services.len() > 8
            || self.mounts.len() > 16
            || self.users.len() > 16
            || self.groups.len() > 16
        {
            return Err(invalid("too many managed resources"));
        }
        let mut keys = std::collections::BTreeSet::new();
        for service in &self.services {
            if !keys.insert(format!("service:{}", service.value())) {
                return Err(invalid("duplicate service"));
            }
        }
        for m in &self.mounts {
            ResourceChange::PersistentMount {
                uuid: m.uuid.clone(),
                name: m.name.clone(),
                filesystem: m.filesystem,
                present: true,
            }
            .validate()?;
            if !keys.insert(format!("mount:{}", m.name)) || !keys.insert(format!("uuid:{}", m.uuid))
            {
                return Err(invalid("duplicate mount"));
            }
        }
        for u in &self.users {
            username(&u.name)?;
            if u.uid < 1000 || u.uid == 65534 {
                return Err(invalid("invalid managed user UID"));
            }
            if !keys.insert(format!("user:{}", u.name)) || !keys.insert(format!("uid:{}", u.uid)) {
                return Err(invalid("duplicate user"));
            }
        }
        for g in &self.groups {
            username(&g.name)?;
            if g.uid < 1000 || g.uid == 65534 || !keys.insert(format!("group:{}", g.name)) {
                return Err(invalid("invalid caller binding"));
            }
            ResourceChange::UserGroups {
                groups: g.groups.clone(),
            }
            .validate()?;
        }
        if let Some(f) = &self.firewall {
            ResourceChange::Firewall {
                tcp: f.tcp.clone(),
                udp: f.udp.clone(),
                trusted_interfaces: f.trusted_interfaces.clone(),
            }
            .validate()?;
        }
        if let Some(p) = &self.power {
            ResourceChange::PowerSettings {
                lid: p.lid,
                idle_minutes: p.idle_minutes,
            }
            .validate()?;
        }
        Ok(())
    }
    pub fn changed(
        &self,
        change: &ResourceChange,
        caller: Option<(&str, u32)>,
    ) -> Result<Self, ValidationError> {
        change.validate()?;
        let mut next = self.clone();
        match change {
            ResourceChange::ServiceEnabled { service, enabled } => {
                next.services.retain(|s| s != service);
                if *enabled {
                    next.services.push(*service);
                }
                next.services.sort_by_key(|s| s.value());
            }
            ResourceChange::PersistentMount {
                uuid,
                name,
                filesystem,
                present,
            } => {
                next.mounts.retain(|m| &m.name != name);
                if *present {
                    next.mounts.push(ManagedMount {
                        uuid: uuid.clone(),
                        name: name.clone(),
                        filesystem: *filesystem,
                    });
                }
                next.mounts.sort_by(|a, b| a.name.cmp(&b.name));
            }
            ResourceChange::Firewall {
                tcp,
                udp,
                trusted_interfaces,
            } => {
                let mut f = FirewallState {
                    tcp: tcp.clone(),
                    udp: udp.clone(),
                    trusted_interfaces: trusted_interfaces.clone(),
                };
                f.tcp.sort();
                f.tcp.dedup();
                f.udp.sort();
                f.udp.dedup();
                f.trusted_interfaces.sort();
                f.trusted_interfaces.dedup();
                next.firewall = Some(f);
            }
            ResourceChange::UserCreate { name } => {
                if next.users.iter().any(|u| &u.name == name) {
                    return Err(invalid("account is already managed"));
                }
                next.users.push(ManagedUser {
                    name: name.clone(),
                    uid: caller
                        .ok_or_else(|| invalid("assigned user identity required"))?
                        .1,
                    disabled: false,
                });
                next.users.sort_by(|a, b| a.name.cmp(&b.name));
            }
            ResourceChange::UserDisabled { name, disabled } => {
                next.users
                    .iter_mut()
                    .find(|u| &u.name == name)
                    .ok_or_else(|| invalid("only Peasy-created accounts can be disabled"))?
                    .disabled = *disabled;
            }
            ResourceChange::UserGroups { groups } => {
                let (name, uid) = caller.ok_or_else(|| invalid("caller binding required"))?;
                next.groups.retain(|g| g.name != name);
                let mut groups = groups.clone();
                groups.sort();
                groups.dedup();
                if !groups.is_empty() {
                    next.groups.push(CallerGroups {
                        name: name.into(),
                        uid,
                        groups,
                    });
                }
                next.groups.sort_by(|a, b| a.name.cmp(&b.name));
            }
            ResourceChange::PowerSettings { lid, idle_minutes } => {
                next.power = Some(PowerSettings {
                    lid: *lid,
                    idle_minutes: *idle_minutes,
                });
            }
            _ => return Err(invalid("operation is not declarative")),
        }
        next.validate()?;
        Ok(next)
    }
    pub fn render(&self) -> Result<String, ValidationError> {
        self.validate()?;
        let mut s = String::new();
        for service in &self.services {
            s.push_str(&format!("  {} = true;\n", service.value()));
        }
        for m in &self.mounts {
            s.push_str(&format!("  fileSystems.{} = {{ device = {}; fsType = {}; options = [ \"nofail\" \"nodev\" \"nosuid\" ]; }};\n",nix_string(&format!("/mnt/peasy-{}",m.name)),nix_string(&format!("/dev/disk/by-uuid/{}",m.uuid)),nix_string(m.filesystem.value())));
        }
        if let Some(f) = &self.firewall {
            s.push_str(&format!("  networking.firewall.enable = true;\n  networking.firewall.allowedTCPPorts = [ {} ];\n  networking.firewall.allowedUDPPorts = [ {} ];\n  networking.firewall.trustedInterfaces = [ {} ];\n",f.tcp.iter().map(u16::to_string).collect::<Vec<_>>().join(" "),f.udp.iter().map(u16::to_string).collect::<Vec<_>>().join(" "),f.trusted_interfaces.iter().map(|i|nix_string(i)).collect::<Vec<_>>().join(" ")));
        }
        for u in &self.users {
            s.push_str(&format!("  users.users.{} = {{ isNormalUser = true; uid = {}; initialHashedPassword = \"!\";{} }};\n",nix_string(&u.name),u.uid,if u.disabled {" hashedPassword = \"!\"; shell = pkgs.shadow + \"/bin/nologin\";"} else {""}));
        }
        let disabled = self
            .users
            .iter()
            .filter(|u| u.disabled)
            .map(|u| nix_string(&u.name))
            .collect::<Vec<_>>();
        if !disabled.is_empty() {
            s.push_str(&format!(
                "  services.openssh.settings.DenyUsers = [ {} ];\n",
                disabled.join(" ")
            ));
        }
        for g in &self.groups {
            s.push_str(&format!(
                "  users.users.{}.extraGroups = [ {} ];\n",
                nix_string(&g.name),
                g.groups
                    .iter()
                    .map(|s| nix_string(s))
                    .collect::<Vec<_>>()
                    .join(" ")
            ));
        }
        if !self.groups.is_empty() {
            s.push_str("  assertions = [\n");
            for g in &self.groups {
                s.push_str(&format!("    {{ assertion = (config.users.users.{name}.isNormalUser or false) && (config.users.users.{name}.uid == null || config.users.users.{name}.uid == {uid}); message = \"Peasy caller identity changed\"; }}\n",name=nix_string(&g.name),uid=g.uid));
            }
            s.push_str("  ];\n");
        }
        if let Some(p) = &self.power {
            s.push_str(&format!("  services.logind.settings.Login = {{ HandleLidSwitch = {}; IdleAction = {}; IdleActionSec = {}; }};\n",nix_string(p.lid.value()),nix_string(if p.idle_minutes==0 {"ignore"} else {"suspend"}),nix_string(&format!("{}min",p.idle_minutes))));
        }
        Ok(s)
    }
}

pub fn query_schema() -> Value {
    json!({"type":["object","null"],"additionalProperties":false,"properties":{"domain":{"type":"string","enum":["diagnostics","services","storage","nix_maintenance","users","firewall","printing","displays","audio","power"]},"target":{"type":["string","null"],"maxLength":160}},"required":["domain","target"]})
}
pub fn change_schema() -> Value {
    let string = |max| json!({"type":"string","maxLength":max});
    let enumeration = |values: Vec<&str>| json!({"type":"string","enum":values});
    let integer = |min, max| json!({"type":"integer","minimum":min,"maximum":max});
    let strings =
        |max| json!({"type":"array","maxItems":max,"items":{"type":"string","maxLength":64}});
    let bool_schema = json!({"type":"boolean"});
    let nullable_string = |max| json!({"type":["string","null"],"maxLength":max});
    let mut variants = vec![json!({"type":"null"})];
    let mut add = |operation: &str, fields: Vec<(&str, Value)>| {
        let mut props = serde_json::Map::new();
        props.insert(
            "operation".into(),
            json!({"type":"string","enum":[operation]}),
        );
        let mut required = vec!["operation"];
        for (name, schema) in fields {
            props.insert(name.into(), schema);
            required.push(name);
        }
        variants.push(json!({"type":"object","additionalProperties":false,"properties":props,"required":required}));
    };
    add(
        "service",
        vec![
            ("unit", string(160)),
            ("action", enumeration(vec!["start", "stop", "restart"])),
        ],
    );
    add(
        "service_enabled",
        vec![
            (
                "service",
                enumeration(vec![
                    "ssh",
                    "caddy",
                    "tailscale",
                    "printing",
                    "bluetooth",
                    "docker",
                    "podman",
                    "libvirtd",
                ]),
            ),
            ("enabled", bool_schema.clone()),
        ],
    );
    add(
        "disk",
        vec![
            ("device", string(69)),
            ("action", enumeration(vec!["mount", "unmount", "format"])),
            (
                "filesystem",
                json!({"type":["string","null"],"enum":["ext4","vfat","exfat",null]}),
            ),
        ],
    );
    add(
        "persistent_mount",
        vec![
            ("uuid", string(64)),
            ("name", string(48)),
            ("filesystem", enumeration(vec!["ext4", "vfat", "exfat"])),
            ("present", bool_schema.clone()),
        ],
    );
    let ports = json!({"type":"array","maxItems":64,"items":{"type":"integer","minimum":1,"maximum":65535}});
    add(
        "firewall",
        vec![
            ("tcp", ports.clone()),
            ("udp", ports),
            ("trusted_interfaces", strings(8)),
        ],
    );
    add("user_create", vec![("name", string(32))]);
    add(
        "user_disabled",
        vec![("name", string(32)), ("disabled", bool_schema.clone())],
    );
    add("user_groups", vec![("groups", strings(16))]);
    add("nix_optimise", vec![]);
    add("nix_garbage_collect", vec![]);
    add(
        "nix_delete_generations",
        vec![(
            "generations",
            json!({"type":"array","minItems":1,"maxItems":32,"items":{"type":"integer","minimum":1}}),
        )],
    );
    add(
        "audio",
        vec![
            ("id", integer(1, u32::MAX)),
            (
                "action",
                enumeration(vec!["default", "volume", "mute", "unmute"]),
            ),
            (
                "volume",
                json!({"type":["integer","null"],"minimum":0,"maximum":100}),
            ),
        ],
    );
    add(
        "power_profile",
        vec![(
            "profile",
            enumeration(vec!["balanced", "power_saver", "performance"]),
        )],
    );
    add(
        "power_settings",
        vec![
            ("lid", enumeration(vec!["ignore", "suspend", "hibernate"])),
            ("idle_minutes", integer(0, 1440)),
        ],
    );
    add(
        "printer",
        vec![
            ("name", string(64)),
            ("action", enumeration(vec!["add", "default", "test"])),
            ("uri", nullable_string(512)),
        ],
    );
    add(
        "display",
        vec![
            ("connector", string(64)),
            ("mode", string(48)),
            ("scale_percent", integer(50, 300)),
            ("x", integer(0, 16384)),
            ("y", integer(0, 16384)),
            ("primary", bool_schema),
        ],
    );
    json!({"anyOf":variants})
}

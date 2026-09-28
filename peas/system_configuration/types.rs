//! Generic, reviewed NixOS configuration primitives. No application recipes.
use crate::{PackageState, ValidationError, nix_string, validate_attribute};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

/// Shared by schema, Wasm validation and the privileged renderer.
pub const SYSTEM_ENABLE_OPTIONS: &[(&str, &str)] = &[
    (
        "virtualisation.libvirtd.enable",
        "Enables local VM management. Guest networks and privileged VM operations are available to authorised users.",
    ),
    (
        "programs.virt-manager.enable",
        "Adds desktop integration for Virtual Machine Manager.",
    ),
    (
        "services.printing.enable",
        "Enables CUPS printing. Printer discovery, drivers and selecting a printer may need manual setup.",
    ),
    (
        "hardware.sane.enable",
        "Enables scanner backends and device access. Vendor-specific drivers may need manual setup.",
    ),
    (
        "hardware.bluetooth.enable",
        "Enables the Bluetooth stack. Pair devices separately.",
    ),
    (
        "virtualisation.podman.enable",
        "Enables Podman, rootless containers and local container sockets. No podman group access is granted; rootful containers require administrator access.",
    ),
    (
        "virtualisation.docker.enable",
        "Starts the rootful Docker daemon and local socket. Containers can modify networking/firewall rules; published ports can expose services.",
    ),
    (
        "programs.nix-ld.enable",
        "Provides the loader and base libraries for unpackaged Linux binaries. Additional runtime libraries may still need manual configuration. Start a new login session.",
    ),
    (
        "programs.direnv.enable",
        "Integrates direnv and nix-direnv with supported shells. Start a new shell; approve each trusted project with direnv allow. Project scripts run with your account access.",
    ),
    (
        "programs.zsh.enable",
        "Installs Zsh with system shell integration. Does not change your login shell or personal configuration.",
    ),
    (
        "programs.fish.enable",
        "Installs Fish with system shell integration. Does not change your login shell or personal configuration.",
    ),
    (
        "programs.wireshark.enable",
        "Adds Wireshark capture integration and a privileged dumpcap wrapper. Capture access requires the wireshark group.",
    ),
    (
        "programs.mtr.enable",
        "Adds MTR network diagnostics and its privileged packet-sending wrapper.",
    ),
    (
        "programs.appimage.enable",
        "Adds appimage-run for manually obtained AppImages. Does not download an application or grant it extra device access.",
    ),
    (
        "programs.appimage.binfmt",
        "Allows executable AppImages to run through appimage-run automatically. Requires programs.appimage.enable.",
    ),
    (
        "services.flatpak.enable",
        "Enables Flatpak with a GTK portal fallback. Remotes, apps and per-app sandbox overrides remain manual; log in again. Screen sharing needs the desktop-specific portal.",
    ),
    (
        "programs.dconf.enable",
        "Enables the settings backend used by GTK/GNOME applications.",
    ),
    (
        "services.gnome.gnome-keyring.enable",
        "Enables the GNOME secret-storage service and login integration. A new login and keyring unlock may be needed; does not create or migrate secrets.",
    ),
    (
        "services.gvfs.enable",
        "Enables desktop file-manager integration for remote locations and removable devices. Mount destinations and credentials remain user choices.",
    ),
    (
        "services.udisks2.enable",
        "Enables disk management through the existing desktop authorization policy. Does not format or automatically mount a disk.",
    ),
    (
        "services.upower.enable",
        "Enables desktop battery and power-status reporting.",
    ),
    (
        "services.pcscd.enable",
        "Enables smart-card access. Card software, PINs and enrollment remain manual.",
    ),
    (
        "services.ratbagd.enable",
        "Enables configuration of supported gaming mice through libratbag. Device settings remain user choices.",
    ),
    (
        "hardware.i2c.enable",
        "Loads I2C device support, granting hardware-bus access to the active local seat and the i2c group. Incorrect hardware writes can damage devices.",
    ),
    (
        "hardware.openrazer.enable",
        "Adds OpenRazer kernel drivers and its user service. Requires compatible hardware/kernel; a reboot may be needed.",
    ),
    (
        "hardware.steam-hardware.enable",
        "Adds controller, VR and Steam hardware device rules. Does not install Steam or change graphics drivers.",
    ),
    (
        "programs.gamemode.enable",
        "Enables on-demand GameMode performance tuning and its privileged scheduling helper. Games still need to opt in.",
    ),
    (
        "hardware.graphics.enable",
        "Enables the graphics library stack using the host drivers. Vendor drivers, CUDA and 32-bit libraries require separate setup.",
    ),
];
/// An empty prerequisite denotes a standard NixOS device group.
pub const SYSTEM_GROUPS: &[(&str, &str)] = &[
    ("libvirtd", "virtualisation.libvirtd.enable"),
    ("scanner", "hardware.sane.enable"),
    ("lp", "services.printing.enable"),
    ("lp", "hardware.sane.enable"),
    ("docker", "virtualisation.docker.enable"),
    ("wireshark", "programs.wireshark.enable"),
    ("gamemode", "programs.gamemode.enable"),
    ("i2c", "hardware.i2c.enable"),
    ("openrazer", "hardware.openrazer.enable"),
    ("dialout", ""),
    ("kvm", ""),
    ("render", ""),
    ("video", ""),
];

/// Versioned attributes prevent a channel update from changing the server major.
pub const POSTGRESQL_PACKAGES: &[&str] = &[
    "postgresql_14",
    "postgresql_15",
    "postgresql_16",
    "postgresql_17",
    "postgresql_18",
];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PostgresqlSetup {
    pub package: String,
    /// Create a database and peer-authenticated role for the socket caller only.
    pub caller_database: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SystemSetup {
    pub packages: Vec<String>,
    pub enable: Vec<String>,
    /// Always the authenticated caller; no AI-controlled account field.
    pub groups: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub postgresql: Option<PostgresqlSetup>,
}

impl SystemSetup {
    pub fn validate(&self) -> Result<(), ValidationError> {
        let invalid = |message: &str| ValidationError::InvalidRequest(message.into());
        if self.packages.len() > 8 || self.enable.len() > 8 || self.groups.len() > 4 {
            return Err(invalid("system setup exceeds its bounded change limit"));
        }
        if self.packages.is_empty()
            && self.enable.is_empty()
            && self.groups.is_empty()
            && self.postgresql.is_none()
        {
            return Err(invalid("empty system setup; use a plain package install"));
        }
        if let Some(postgresql) = &self.postgresql
            && !POSTGRESQL_PACKAGES.contains(&postgresql.package.as_str())
        {
            return Err(invalid(
                "PostgreSQL requires a reviewed versioned server package",
            ));
        }
        for package in &self.packages {
            validate_attribute(package)?;
        }
        for option in &self.enable {
            if !SYSTEM_ENABLE_OPTIONS
                .iter()
                .any(|(allowed, _)| option == allowed)
            {
                return Err(invalid("system setting is not in the reviewed catalogue"));
            }
        }
        if self.enable.iter().any(|o| o == "programs.appimage.binfmt")
            && !self.enable.iter().any(|o| o == "programs.appimage.enable")
        {
            return Err(invalid(
                "AppImage execution registration requires appimage-run integration",
            ));
        }
        for group in &self.groups {
            if !SYSTEM_GROUPS.iter().any(|(allowed, required)| {
                group == allowed
                    && (required.is_empty() || self.enable.iter().any(|option| option == required))
            }) {
                return Err(invalid(
                    "group is not allowed or its supporting service is missing",
                ));
            }
        }
        Ok(())
    }

    pub fn needs_account(&self) -> bool {
        !self.groups.is_empty() || self.postgresql.as_ref().is_some_and(|p| p.caller_database)
    }

    pub fn validate_for_package(&self, package: &str) -> Result<(), ValidationError> {
        self.validate()?;
        if POSTGRESQL_PACKAGES.contains(&package)
            && self
                .postgresql
                .as_ref()
                .is_some_and(|p| p.package != package)
        {
            return Err(ValidationError::InvalidRequest(
                "PostgreSQL server version must match the selected package".into(),
            ));
        }
        Ok(())
    }

    fn normalize(&mut self) -> Result<(), ValidationError> {
        self.validate()?;
        for values in [&mut self.packages, &mut self.enable, &mut self.groups] {
            values.sort();
            values.dedup();
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedSetup {
    pub package: String,
    pub settings: SystemSetup,
    /// Resolved by the daemon, never accepted in a model plan or IPC request.
    pub user: Option<String>,
    pub uid: Option<u32>,
}

impl ManagedSetup {
    pub fn normalize(&mut self) -> Result<(), ValidationError> {
        validate_attribute(&self.package)?;
        self.settings.normalize()?;
        self.settings.validate_for_package(&self.package)?;
        let valid_user = self.user.as_ref().is_some_and(|user| {
            !user.is_empty()
                && user.len() <= 64
                && user != "root"
                && user
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        });
        if (self.settings.needs_account()
            && (!valid_user || !self.uid.is_some_and(|uid| uid >= 1000 && uid != 65534)))
            || (!self.settings.needs_account() && (self.user.is_some() || self.uid.is_some()))
        {
            return Err(ValidationError::InvalidRequest(
                "invalid setup account binding".into(),
            ));
        }
        if self
            .settings
            .postgresql
            .as_ref()
            .is_some_and(|p| p.caller_database)
            && !self.user.as_ref().is_some_and(|user| {
                user.len() <= 63
                    && user != "postgres"
                    && !user.starts_with("pg_")
                    && user
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
                    && user
                        .as_bytes()
                        .first()
                        .is_some_and(|b| b.is_ascii_lowercase() || *b == b'_')
            })
        {
            return Err(ValidationError::InvalidRequest(
                "caller name cannot be used as a PostgreSQL database role".into(),
            ));
        }
        Ok(())
    }

    pub fn package_attributes(&self) -> Vec<String> {
        let mut attributes = self.settings.packages.clone();
        attributes.push(self.package.clone());
        if let Some(postgresql) = &self.settings.postgresql {
            attributes.push(postgresql.package.clone());
        }
        attributes.sort();
        attributes.dedup();
        attributes
    }
}

impl PackageState {
    pub fn setup_dependents(&self, package: &str) -> Vec<&str> {
        self.setups
            .iter()
            .filter(|setup| {
                setup.package == package
                    || setup.settings.packages.iter().any(|item| item == package)
                    || setup
                        .settings
                        .postgresql
                        .as_ref()
                        .is_some_and(|p| p.package == package)
            })
            .map(|setup| setup.package.as_str())
            .collect()
    }

    pub fn with_setup(&self, mut setup: ManagedSetup) -> Result<Self, ValidationError> {
        setup.normalize()?;
        let mut state = self.clone();
        if let Some(postgresql) = &setup.settings.postgresql
            && self
                .setups
                .iter()
                .filter_map(|s| s.settings.postgresql.as_ref())
                .any(|existing| existing.package != postgresql.package)
        {
            return Err(ValidationError::InvalidRequest(
                "PostgreSQL major-version changes require a separate database migration".into(),
            ));
        }
        // Upgrade an existing Peasy-only package install into a single owned
        // setup, so uninstall does not leave a hidden standalone copy behind.
        state.packages.retain(|package| package != &setup.package);
        state.setups.retain(|item| item.package != setup.package);
        state.setups.push(setup);
        state.normalize()?;
        Ok(state)
    }

    /// Withdraw only this setup's contribution; keep shared and standalone items.
    pub fn without_setup(&self, package: &str) -> Result<Self, ValidationError> {
        let mut state = self.clone();
        state.setups.retain(|item| item.package != package);
        state.normalize()?;
        Ok(state)
    }

    pub fn effective_packages(&self) -> BTreeSet<String> {
        self.packages
            .iter()
            .cloned()
            .chain(self.setups.iter().flat_map(|setup| {
                std::iter::once(setup.package.clone()).chain(setup.settings.packages.clone())
            }))
            .collect()
    }
}

pub(super) fn render(setups: &[ManagedSetup]) -> String {
    let mut lines = BTreeSet::new();
    let mut assertions = BTreeSet::new();
    for setup in setups {
        for option in &setup.settings.enable {
            lines.insert(format!("  {option} = true;\n"));
        }
        if setup
            .settings
            .enable
            .iter()
            .any(|o| o == "services.flatpak.enable")
        {
            // A portable fallback without replacing desktop-specific portal choices.
            lines.insert("  xdg.portal.enable = true;\n".into());
            lines.insert("  xdg.portal.extraPortals = [ pkgs.xdg-desktop-portal-gtk ];\n".into());
            lines
                .insert("  xdg.portal.config.common.default = lib.mkDefault [ \"gtk\" ];\n".into());
        }
        if let Some(postgresql) = &setup.settings.postgresql {
            lines.insert("  services.postgresql.enable = true;\n".into());
            lines.insert(format!(
                "  services.postgresql.package = pkgs.{};\n",
                postgresql.package
            ));
            lines.insert("  services.postgresql.enableTCPIP = false;\n".into());
            assertions.insert("    { assertion = config.services.postgresql.settings.listen_addresses == \"localhost\"; message = \"Peasy PostgreSQL setup requires local-only connections\"; }\n".into());
            assertions.insert("    { assertion = config.services.postgresql.enable && config.services.postgresql.settings.port == 5432 && config.services.postgresql.settings.unix_socket_directories == \"/run/postgresql\"; message = \"Peasy PostgreSQL setup requires the reviewed local service endpoint\"; }\n".into());
            lines.insert(
                "  services.postgresql.settings.unix_socket_directories = \"/run/postgresql\";\n"
                    .into(),
            );
            assertions.insert(format!("    {{ assertion = config.services.postgresql.dataDir == \"/var/lib/postgresql/{}\"; message = \"Peasy PostgreSQL setup requires the standard versioned data directory\"; }}\n", postgresql.package.strip_prefix("postgresql_").expect("validated PostgreSQL package")));
            assertions.insert(format!("    {{ assertion = config.services.postgresql.package.drvPath == pkgs.{}.drvPath; message = \"Peasy PostgreSQL server package differs from the reviewed version\"; }}\n", postgresql.package));
        }
        if let Some(user) = &setup.user {
            assertions.insert(format!("    {{ assertion = config.users.users.{}.isNormalUser or false; message = \"Peasy setup requires an existing normal user account\"; }}\n", nix_string(user)));
            assertions.insert(format!("    {{ assertion = config.users.users.{user}.uid == null || config.users.users.{user}.uid == {uid}; message = \"Peasy setup account UID changed; review a new setup\"; }}\n", user = nix_string(user), uid = setup.uid.expect("validated group account UID")));
            // One assignment per user below, avoiding duplicate Nix attributes.
            let groups: BTreeSet<_> = setups
                .iter()
                .filter(|s| s.user.as_ref() == Some(user))
                .flat_map(|s| s.settings.groups.iter())
                .collect();
            if !groups.is_empty() {
                lines.insert(format!(
                    "  users.users.{}.extraGroups = [ {} ];\n",
                    nix_string(user),
                    groups
                        .into_iter()
                        .map(|g| nix_string(g))
                        .collect::<Vec<_>>()
                        .join(" ")
                ));
            }
        }
    }
    let database_users: BTreeSet<_> = setups
        .iter()
        .filter(|s| {
            s.settings
                .postgresql
                .as_ref()
                .is_some_and(|p| p.caller_database)
        })
        .filter_map(|s| s.user.as_ref())
        .collect();
    if !database_users.is_empty() {
        lines.insert(format!(
            "  services.postgresql.ensureDatabases = [ {} ];\n",
            database_users
                .iter()
                .map(|u| nix_string(u))
                .collect::<Vec<_>>()
                .join(" ")
        ));
        lines.insert(format!(
            "  services.postgresql.ensureUsers = [ {} ];\n",
            database_users
                .iter()
                .map(|u| format!("{{ name = {}; ensureDBOwnership = true; }}", nix_string(u)))
                .collect::<Vec<_>>()
                .join(" ")
        ));
    }
    let mut rendered: String = lines.into_iter().collect();
    if !assertions.is_empty() {
        rendered.push_str("  assertions = [\n");
        rendered.extend(assertions);
        rendered.push_str("  ];\n");
    }
    rendered
}

use serde_json::{Value, json};
pub fn setup_schema() -> Value {
    json!({
        "type": ["object", "null"], "additionalProperties": false,
        "properties": {
            "packages": {"type": "array", "maxItems": 8, "items": {"type": "string", "maxLength": crate::MAX_ATTRIBUTE_BYTES}},
            "enable": {"type": "array", "maxItems": 8, "items": {"type": "string", "enum": SYSTEM_ENABLE_OPTIONS.iter().map(|(option, _)| *option).collect::<Vec<_>>()}},
            "groups": {"type": "array", "maxItems": 4, "items": {"type": "string", "enum": SYSTEM_GROUPS.iter().map(|(group, _)| *group).collect::<std::collections::BTreeSet<_>>()}},
            "postgresql": {"type": ["object", "null"], "additionalProperties": false,
                "properties": {
                    "package": {"type": "string", "enum": POSTGRESQL_PACKAGES},
                    "caller_database": {"type": "boolean"}
                }, "required": ["package", "caller_database"]}
        },
        "required": ["packages", "enable", "groups", "postgresql"]
    })
}

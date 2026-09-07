//! Generic, reviewed NixOS configuration primitives. No application recipes.
use crate::{PackageState, ValidationError, nix_string, validate_attribute};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

/// Shared by schema, Wasm validation and the privileged renderer.
pub const SYSTEM_ENABLE_OPTIONS: &[&str] = &[
    "virtualisation.libvirtd.enable",
    "programs.virt-manager.enable",
    "services.printing.enable",
    "hardware.sane.enable",
    "hardware.bluetooth.enable",
];
pub const SYSTEM_GROUPS: &[(&str, &str)] = &[
    ("libvirtd", "virtualisation.libvirtd.enable"),
    ("scanner", "hardware.sane.enable"),
    ("lp", "services.printing.enable"),
    ("lp", "hardware.sane.enable"),
];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SystemSetup {
    pub packages: Vec<String>,
    pub enable: Vec<String>,
    /// Always the authenticated caller; no AI-controlled account field.
    pub groups: Vec<String>,
}

impl SystemSetup {
    pub fn validate(&self) -> Result<(), ValidationError> {
        let invalid = |message: &str| ValidationError::InvalidRequest(message.into());
        if self.packages.len() > 8 || self.enable.len() > 8 || self.groups.len() > 4 {
            return Err(invalid("system setup exceeds its bounded change limit"));
        }
        if self.packages.is_empty() && self.enable.is_empty() && self.groups.is_empty() {
            return Err(invalid("empty system setup; use a plain package install"));
        }
        for package in &self.packages {
            validate_attribute(package)?;
        }
        for option in &self.enable {
            if !SYSTEM_ENABLE_OPTIONS.contains(&option.as_str()) {
                return Err(invalid("system setting is not in the reviewed catalogue"));
            }
        }
        for group in &self.groups {
            if !SYSTEM_GROUPS.iter().any(|(allowed, required)| {
                group == allowed && self.enable.iter().any(|option| option == required)
            }) {
                return Err(invalid(
                    "group is not allowed or its supporting service is missing",
                ));
            }
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
        let valid_user = self.user.as_ref().is_some_and(|user| {
            !user.is_empty()
                && user.len() <= 64
                && user != "root"
                && user
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        });
        if (!self.settings.groups.is_empty()
            && (!valid_user || !self.uid.is_some_and(|uid| uid >= 1000 && uid != 65534)))
            || (self.settings.groups.is_empty() && (self.user.is_some() || self.uid.is_some()))
        {
            return Err(ValidationError::InvalidRequest(
                "invalid setup account binding".into(),
            ));
        }
        Ok(())
    }
}

impl PackageState {
    pub fn setup_dependents(&self, package: &str) -> Vec<&str> {
        self.setups
            .iter()
            .filter(|setup| {
                setup.package == package
                    || setup.settings.packages.iter().any(|item| item == package)
            })
            .map(|setup| setup.package.as_str())
            .collect()
    }

    pub fn with_setup(&self, mut setup: ManagedSetup) -> Result<Self, ValidationError> {
        setup.normalize()?;
        let mut state = self.clone();
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
        if let Some(user) = &setup.user {
            assertions.insert(format!("    {{ assertion = config.users.users.{}.isNormalUser or false; message = \"Peasy setup requires an existing normal user account\"; }}\n", nix_string(user)));
            assertions.insert(format!("    {{ assertion = config.users.users.{user}.uid == null || config.users.users.{user}.uid == {uid}; message = \"Peasy setup account UID changed; review a new setup\"; }}\n", user = nix_string(user), uid = setup.uid.expect("validated group account UID")));
            // One assignment per user below, avoiding duplicate Nix attributes.
            let groups: BTreeSet<_> = setups
                .iter()
                .filter(|s| s.user.as_ref() == Some(user))
                .flat_map(|s| s.settings.groups.iter())
                .collect();
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
    let mut rendered: String = lines.into_iter().collect();
    if !assertions.is_empty() {
        rendered.push_str("  assertions = [\n");
        rendered.extend(assertions);
        rendered.push_str("  ];\n");
    }
    rendered
}

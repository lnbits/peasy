//! Runs inside the existing authenticated daemon transaction; never invokes AI.
use super::{NixBackend, Preview};
use anyhow::{Context, Result, bail};
use nix::unistd::{Uid, User};
use peasy_core::{
    DiffKind, DiffLine, ManagedSetup, PackageOperation, PackageState, ProposalChange, SystemSetup,
    module_diff,
};

impl NixBackend {
    pub fn preview_setup(
        &self,
        package: String,
        settings: SystemSetup,
        uid: u32,
    ) -> Result<Preview> {
        // Validate before NSS lookup or any Nix command. The account comes from
        // SO_PEERCRED, never from the request body or model.
        settings.validate()?;
        let user = if !settings.needs_account() {
            None
        } else {
            if uid < 1000 || uid == 65534 {
                bail!("setup access requires a normal user account");
            }
            Some(
                User::from_uid(Uid::from_raw(uid))?
                    .context("requesting account no longer exists")?
                    .name,
            )
        };
        let bound_uid = user.as_ref().map(|_| uid);
        let mut setup = ManagedSetup {
            package,
            settings,
            user,
            uid: bound_uid,
        };
        setup.normalize()?;
        let before = self.current_state()?;
        let after = before.with_setup(setup.clone())?;
        self.validate_postgresql_host(&before, &setup)?;
        let packages = self.identities(&setup.package_attributes())?;
        if before == after {
            bail!("this system setup is already managed by Peasy");
        }
        let mut diff = module_diff(&before, &after)?;
        setup_notes(&setup, &mut diff);
        Ok(Preview {
            packages,
            before,
            diff,
            title: format!("Set up {} and its system integration", setup.package),
            change: ProposalChange::Setup {
                operation: PackageOperation::Install,
                setup,
            },
        })
    }

    pub(super) fn preview_setup_remove(&self, setup: ManagedSetup) -> Result<Preview> {
        let before = self.current_state()?;
        let after = before.without_setup(&setup.package)?;
        let mut diff = module_diff(&before, &after)?;
        diff.push(DiffLine { kind: DiffKind::Context, text: "Withdraws this setup's contributions only. Shared packages, administrator configuration and user data are retained.".into() });
        if setup.settings.postgresql.is_some() {
            diff.push(DiffLine { kind: DiffKind::Context, text: "PostgreSQL databases, roles and grants are retained. Removing this configuration does not revoke existing database access or undo SQL changes.".into() });
        }
        Ok(Preview {
            packages: vec![],
            before,
            diff,
            title: format!("Remove Peasy setup for {}", setup.package),
            change: ProposalChange::Setup {
                operation: PackageOperation::Remove,
                setup,
            },
        })
    }

    pub(super) fn apply_setup_state(
        &self,
        previous: &PackageState,
        operation: PackageOperation,
        setup: &ManagedSetup,
    ) -> Result<(PackageState, String)> {
        match operation {
            PackageOperation::Install => {
                setup.settings.validate()?;
                self.validate_postgresql_host(previous, setup)?;
                if let Some(name) = &setup.user {
                    let user = User::from_name(name)?.context("setup account no longer exists")?;
                    if Some(user.uid.as_raw()) != setup.uid {
                        bail!("setup account is no longer a normal user");
                    }
                }
                let note = if setup.settings.groups.is_empty() {
                    ""
                } else {
                    " Log out completely and back in to use the new group access."
                };
                let database_note = if setup.settings.postgresql.is_some() {
                    " PostgreSQL data and roles are retained on removal or configuration rollback."
                } else {
                    ""
                };
                Ok((
                    previous.with_setup(setup.clone())?,
                    format!(
                        "{} system setup applied.{note}{database_note}",
                        setup.package
                    ),
                ))
            }
            PackageOperation::Remove => {
                if !previous.setups.contains(setup) {
                    bail!("setup is no longer in the reviewed state");
                }
                let access_note = if setup.settings.groups.is_empty() {
                    ""
                } else {
                    " Log out and back in to refresh group access."
                };
                let database_note = if setup.settings.postgresql.is_some() {
                    " PostgreSQL databases, roles and grants were retained; SQL changes were not undone."
                } else {
                    ""
                };
                Ok((
                    previous.without_setup(&setup.package)?,
                    format!(
                        "Peasy setup for {} removed. Shared and administrator-managed settings and user data were retained.{database_note}{access_note}",
                        setup.package
                    ),
                ))
            }
        }
    }

    fn validate_postgresql_host(
        &self,
        previous: &PackageState,
        setup: &ManagedSetup,
    ) -> Result<()> {
        let Some(postgresql) = &setup.settings.postgresql else {
            return Ok(());
        };
        let _evaluation = self.evaluation_lock.try_lock().map_err(|_| {
            anyhow::anyhow!("A Nix operation is already running; try again shortly")
        })?;
        let expression = format!(
            "let host = {}; p = host.config.services.postgresql; in {{ enabled = p.enable; version = if p.enable then p.package.version else null; }}",
            self.host_expression()?
        );
        let output = self.runner.run(
            &self.config.nix,
            &[
                "eval".into(),
                "--impure".into(),
                "--json".into(),
                "--no-write-lock-file".into(),
                "--expr".into(),
                expression.into(),
            ],
            None,
        )?;
        if !output.status.success() {
            bail!(
                "Could not inspect existing PostgreSQL configuration: {}",
                super::useful_stderr(&output)
            );
        }
        let host: PostgresqlHost = serde_json::from_slice(&output.stdout)
            .context("invalid PostgreSQL host configuration")?;
        validate_postgresql_ownership(previous, postgresql, &host)?;
        // A narrow oneshot reads directory names with DAC_READ_SEARCH; the
        // long-running daemon retains no capability to read database files.
        let probe = self.runner.run(
            &self.config.systemctl,
            &["start".into(), "peasy-postgresql-inspect.service".into()],
            None,
        )?;
        if !probe.status.success() {
            bail!(
                "Could not inspect retained PostgreSQL directories: {}",
                super::useful_stderr(&probe)
            );
        }
        check_retained_postgresql_versions(
            &crate::postgresql_probe::read(&self.config.runtime_dir)?,
            &postgresql.package,
        )?;
        Ok(())
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PostgresqlHost {
    enabled: bool,
    version: Option<String>,
}

fn validate_postgresql_ownership(
    previous: &PackageState,
    requested: &peasy_core::PostgresqlSetup,
    host: &PostgresqlHost,
) -> Result<()> {
    let owned = previous
        .setups
        .iter()
        .any(|s| s.settings.postgresql.is_some());
    if host.enabled && !owned {
        bail!(
            "PostgreSQL is already enabled outside Peasy. Its server and data remain administrator-managed; install tools only or ask the administrator to configure access."
        );
    }
    if host.enabled
        && host.version.as_deref().and_then(|v| v.split('.').next())
            != requested.package.strip_prefix("postgresql_")
    {
        bail!(
            "Existing PostgreSQL major version differs; a separate database migration is required."
        );
    }
    Ok(())
}

fn check_retained_postgresql_versions(versions: &[String], package: &str) -> Result<()> {
    let major = package
        .strip_prefix("postgresql_")
        .expect("validated PostgreSQL package");
    for name in versions {
        if name != major && name.bytes().all(|b| b.is_ascii_digit()) {
            bail!(
                "Retained PostgreSQL {name} data exists. Review a database migration before installing another major version."
            );
        }
        if name == "PG_VERSION" {
            bail!(
                "A legacy PostgreSQL data directory exists; administrator review is required before setup."
            );
        }
    }
    Ok(())
}

fn setup_notes(setup: &ManagedSetup, diff: &mut Vec<DiffLine>) {
    for option in &setup.settings.enable {
        if let Some((_, description)) = peasy_core::SYSTEM_ENABLE_OPTIONS
            .iter()
            .find(|(name, _)| option == name)
        {
            diff.push(DiffLine {
                kind: DiffKind::Context,
                text: description.to_string(),
            });
        }
    }
    for group in &setup.settings.groups {
        let access = match group.as_str() {
            "docker" => "Docker group access is effectively root access to this computer.",
            "wireshark" => {
                "Wireshark group access permits capturing network traffic, potentially including other users' private traffic."
            }
            "dialout" => {
                "Serial-device access permits communicating with and reprogramming connected boards/modems."
            }
            "kvm" => {
                "KVM device access permits hardware-accelerated virtual machines; firmware and host CPU support are still required."
            }
            "render" => {
                "Render-device access permits GPU computation and rendering without an active desktop session."
            }
            "video" => {
                "Video-device access includes graphics and camera devices, even outside the active desktop session."
            }
            "i2c" => {
                "I2C group access permits raw hardware-bus operations outside the active local session."
            }
            "openrazer" => "OpenRazer group access permits controlling supported Razer devices.",
            "gamemode" => {
                "GameMode group access authorizes performance and clock-control operations through the module's policy."
            }
            _ => continue,
        };
        diff.push(DiffLine {
            kind: DiffKind::Context,
            text: access.into(),
        });
    }
    if let Some(postgresql) = &setup.settings.postgresql {
        diff.push(DiffLine { kind: DiffKind::Context, text: format!("PostgreSQL server {} starts now and at boot. Connections are local only; no firewall port is opened. Major-version upgrades require a separate migration.", postgresql.package) });
        if postgresql.caller_database {
            diff.push(DiffLine { kind: DiffKind::Context, text: format!("Create database and peer-authenticated role {} for the requesting user, with ownership of that database and no new superuser grant.", setup.user.as_deref().unwrap_or_default()) });
        }
        diff.push(DiffLine { kind: DiffKind::Context, text: "Database contents, roles and grants survive uninstall and configuration rollback. SQL changes are not rolled back.".into() });
    }
    if !setup.settings.groups.is_empty() {
        diff.push(DiffLine {
            kind: DiffKind::Context,
            text: "Changes user permissions. Log out completely and back in after applying.".into(),
        });
    }
    if setup
        .settings
        .groups
        .iter()
        .any(|group| group == "libvirtd")
    {
        diff.push(DiffLine { kind: DiffKind::Context, text: "Security: libvirtd membership grants powerful system-wide VM management access; only approve this for a trusted account.".into() });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elevated_device_and_container_access_is_visible_in_review() {
        for (group, option, warning) in [
            ("docker", "virtualisation.docker.enable", "effectively root"),
            ("wireshark", "programs.wireshark.enable", "private traffic"),
            ("video", "", "camera"),
            ("i2c", "hardware.i2c.enable", "hardware-bus"),
        ] {
            let mut setup: ManagedSetup =
                serde_json::from_str(include_str!("example.json")).unwrap();
            setup.settings.enable = if option.is_empty() {
                vec![]
            } else {
                vec![option.into()]
            };
            setup.settings.groups = vec![group.into()];
            setup.normalize().unwrap();
            let mut diff = vec![];
            setup_notes(&setup, &mut diff);
            assert!(diff.iter().any(|line| line.text.contains(warning)));
            assert!(diff.iter().any(|line| line.text.contains("Log out")));
        }
    }

    fn check_retained_postgresql_data(root: &std::path::Path, package: &str) -> Result<()> {
        check_retained_postgresql_versions(&crate::postgresql_probe::scan(root)?, package)
    }

    #[test]
    fn postgres_refuses_external_ownership_and_major_version_changes() {
        let setup: ManagedSetup =
            serde_json::from_str(include_str!("postgresql-example.json")).unwrap();
        let requested = setup.settings.postgresql.as_ref().unwrap();
        let empty = PackageState::default();
        let enabled = PostgresqlHost {
            enabled: true,
            version: Some("17.6".into()),
        };
        assert!(validate_postgresql_ownership(&empty, requested, &enabled).is_err());
        let owned = empty.with_setup(setup.clone()).unwrap();
        assert!(validate_postgresql_ownership(&owned, requested, &enabled).is_ok());
        let different = PostgresqlHost {
            enabled: true,
            version: Some("16.10".into()),
        };
        assert!(validate_postgresql_ownership(&owned, requested, &different).is_err());
        let disabled = PostgresqlHost {
            enabled: false,
            version: None,
        };
        assert!(validate_postgresql_ownership(&empty, requested, &disabled).is_ok());
    }

    #[test]
    fn retained_data_blocks_accidental_new_major_without_touching_data() {
        let root = tempfile::tempdir().unwrap();
        assert!(check_retained_postgresql_data(root.path(), "postgresql_17").is_ok());
        let database = root.path().join("17");
        std::fs::create_dir(&database).unwrap();
        std::fs::write(database.join("PG_VERSION"), "17").unwrap();
        assert!(check_retained_postgresql_data(root.path(), "postgresql_17").is_ok());
        assert!(check_retained_postgresql_data(root.path(), "postgresql_18").is_err());
        assert_eq!(
            std::fs::read_to_string(database.join("PG_VERSION")).unwrap(),
            "17"
        );
        std::fs::write(root.path().join("PG_VERSION"), "13").unwrap();
        assert!(check_retained_postgresql_data(root.path(), "postgresql_17").is_err());
    }
}

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
        let user = if settings.groups.is_empty() {
            None
        } else {
            if uid < 1000 || uid == 65534 {
                bail!("group access requires a normal user account");
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
        let mut attributes = setup.settings.packages.clone();
        attributes.push(setup.package.clone());
        let packages = self.identities(&attributes)?;
        let before = self.current_state()?;
        let after = before.with_setup(setup.clone())?;
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
                Ok((
                    previous.with_setup(setup.clone())?,
                    format!("{} system setup applied.{note}", setup.package),
                ))
            }
            PackageOperation::Remove => {
                if !previous.setups.contains(setup) {
                    bail!("setup is no longer in the reviewed state");
                }
                Ok((
                    previous.without_setup(&setup.package)?,
                    format!(
                        "Peasy setup for {} removed. Shared and administrator-managed settings and user data were retained. Log out and back in to refresh group access.",
                        setup.package
                    ),
                ))
            }
        }
    }
}

fn setup_notes(setup: &ManagedSetup, diff: &mut Vec<DiffLine>) {
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

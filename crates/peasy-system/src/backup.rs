use super::{NixBackend, Preview};
use anyhow::{Result, bail};
use peasy_core::{
    DiffKind, DiffLine, PackageState, PortableBackup, ProposalChange, RestoreMode, module_diff,
};

impl NixBackend {
    pub(super) fn verify_restore_peas(
        &self,
        before: &PackageState,
        after: &PackageState,
    ) -> Result<()> {
        for pin in &after.peas {
            if !before.peas.contains(pin) {
                self.verify_backup_pea(pin)?;
            }
        }
        Ok(())
    }

    pub fn preview_restore(&self, backup: PortableBackup, mode: RestoreMode) -> Result<Preview> {
        let before = self.current_state()?;
        let after = backup.restore(&before, mode)?;
        if before == after {
            bail!("The selected backup already matches these Peasy settings; nothing to restore.");
        }
        self.verify_restore_peas(&before, &after)?;
        let packages = self.identities(&backup.packages)?;
        let label = match mode {
            RestoreMode::Merge => "Merge",
            RestoreMode::Replace => "Replace",
        };
        let mut diff = vec![DiffLine {
            kind: DiffKind::Context,
            text: format!(
                "{label} standalone packages, appearance and pea instructions from the backup. Destination hardware, service setups, network profiles and AppImages are preserved. Archived host files are never imported."
            ),
        }];
        for pin in after.peas.iter().filter(|p| !before.peas.contains(p)) {
            diff.push(DiffLine {
                kind: DiffKind::Context,
                text: format!(
                    "Restore official pea {} {}, API {}, permissions: {}. Source: {}. SHA-256: {}.",
                    pin.id,
                    pin.version,
                    pin.host_api,
                    pin.permissions.join(", "),
                    pin.url(),
                    pin.hash
                ),
            });
        }
        for package in &packages {
            diff.push(DiffLine {
                kind: DiffKind::Context,
                text: format!(
                    "Destination package: {} {} ({})",
                    package.attribute, package.version, package.drv_path
                ),
            });
        }
        diff.extend(module_diff(&before, &after)?);
        Ok(Preview {
            before,
            packages,
            change: ProposalChange::Restore { backup, mode },
            title: format!("Restore backup — {label}"),
            diff,
        })
    }
}

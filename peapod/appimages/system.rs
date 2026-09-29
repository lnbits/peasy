//! appimages pea: privileged, validated proposals; activation stays in NixBackend.
use super::{NixBackend, Preview};
use anyhow::{Context, Result, bail};
use peasy_core::{
    AppImagePackage, DiffKind, DiffLine, PackageOperation, ProposalChange, module_diff,
};

impl NixBackend {
    pub fn preview_appimage_install(&self, package: AppImagePackage) -> Result<Preview> {
        package.validate()?;
        self.authorize_appimage(&package)?;
        let before = self.current_state()?;
        let after = before.with_appimage_install(&package)?;
        if before == after {
            bail!("that exact external AppImage is already installed");
        }
        let replacing = before
            .appimages
            .iter()
            .any(|existing| existing.id == package.id);
        let mut diff = appimage_review_details(&package);
        diff.extend(module_diff(&before, &after)?);
        Ok(Preview {
            packages: vec![],
            before,
            change: ProposalChange::AppImage {
                operation: PackageOperation::Install,
                package: package.clone(),
            },
            title: format!(
                "{} external {} {}",
                if replacing { "Update" } else { "Install" },
                package.display_name,
                package.version
            ),
            diff,
        })
    }

    pub(super) fn preview_appimage_remove(&self, package: AppImagePackage) -> Result<Preview> {
        package.validate()?;
        let before = self.current_state()?;
        let after = before.with_appimage_remove(&package.id)?;
        let mut diff = appimage_review_details(&package);
        diff.extend(module_diff(&before, &after)?);
        Ok(Preview {
            packages: vec![],
            before,
            change: ProposalChange::AppImage {
                operation: PackageOperation::Remove,
                package: package.clone(),
            },
            title: format!("Remove external {}", package.display_name),
            diff,
        })
    }

    pub(super) fn authorize_appimage(&self, package: &AppImagePackage) -> Result<()> {
        let policy = peasy_core::AppImagePolicy::load(&self.config.appimage_policy)
            .context("reading administrator AppImage policy")?;
        if !policy.allows(package) {
            bail!(
                "This AppImage release has not been approved by the administrator. Add its independently verified hash to services.peasy.appImages.trustedHashes before installing."
            );
        }
        Ok(())
    }
}

fn appimage_review_details(package: &AppImagePackage) -> Vec<DiffLine> {
    vec![
        DiffLine {
            kind: DiffKind::Context,
            text: "External native application — verify the publisher before applying".into(),
        },
        DiffLine {
            kind: DiffKind::Context,
            text: format!("Repository: https://github.com/{}", package.repository),
        },
        DiffLine {
            kind: DiffKind::Context,
            text: format!(
                "Release: {} ({})",
                package.release_tag, package.architecture
            ),
        },
        DiffLine {
            kind: DiffKind::Context,
            text: format!("Asset: {} ({} bytes)", package.asset_name, package.size),
        },
        DiffLine {
            kind: DiffKind::Context,
            text: format!("Download: {}", package.url),
        },
        DiffLine {
            kind: DiffKind::Context,
            text: format!("SHA-256: {}", package.hash),
        },
    ]
}

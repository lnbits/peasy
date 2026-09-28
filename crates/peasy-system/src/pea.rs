use super::{NixBackend, Preview};
use anyhow::{Result, bail};
use peasy_core::{
    DiffKind, DiffLine, ProposalChange, module_diff,
    pea::{MAX_PACK_BYTES, PeaPin, PeaPolicy},
};
use std::{
    io::Read,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::Path,
};
impl NixBackend {
    pub(super) fn verify_pea(&self, pin: &PeaPin) -> Result<()> {
        self.verify_pea_source(pin, false)
    }
    pub(super) fn verify_backup_pea(&self, pin: &PeaPin) -> Result<()> {
        self.verify_pea_source(pin, true)
    }
    fn verify_pea_source(&self, pin: &PeaPin, restore: bool) -> Result<()> {
        pin.validate()?;
        let policy = PeaPolicy::load(&self.config.pea_policy)?;
        if !policy.allows(pin) {
            bail!("Administrator policy does not allow this official pea or its permissions");
        }
        let _guard = self
            .pea_fetch_lock
            .try_lock()
            .map_err(|_| anyhow::anyhow!("another pea verification is in progress"))?;
        // Stop any interrupted helper before replacing its read-only request.
        // No caller-controlled executable, unit name, URL or command reaches systemd.
        let stop = self.runner.run(
            &self.config.systemctl,
            &["stop".into(), "peasy-pea-fetch.service".into()],
            None,
        )?;
        if !stop.status.success() {
            bail!("could not reset the pea verification helper");
        }
        let staging = self.config.runtime_dir.join("pea-fetch");
        std::fs::create_dir_all(&staging)?;
        std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o700))?;
        let request = staging.join("request.json");
        // Root has no DAC override capability. Replace the previous read-only
        // request through its owned directory after the helper has stopped.
        match std::fs::remove_file(&request) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let request_bytes = if restore {
            serde_json::to_vec(&serde_json::json!({"pin": pin, "restore": true}))?
        } else {
            serde_json::to_vec(pin)?
        };
        std::fs::write(&request, request_bytes)?;
        // The helper receives only this leaf through a read-only bind mount.
        std::fs::set_permissions(&request, std::fs::Permissions::from_mode(0o444))?;
        let verified = self.runner.run(
            &self.config.systemctl,
            &["start".into(), "peasy-pea-fetch.service".into()],
            None,
        )?;
        if !verified.status.success() {
            bail!(
                "official pea source verification failed; discover and review again (see peasy-pea-fetch.service)"
            );
        }
        let mut bytes = vec![];
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
            .open(&self.config.pea_fetch_output)?;
        if !file.metadata()?.is_file() {
            bail!("invalid pea helper output");
        }
        file.take(MAX_PACK_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        crate::pea_fetch::validate_artifact(pin, &bytes)?;
        // Copy the checked bytes into root-owned staging before importing, avoiding
        // a check/use race on the unprivileged helper's output. Nix never downloads
        // unchecked bytes during preview. The final generation holds the store ref.
        let artifact = staging.join("pea.json");
        std::fs::write(&artifact, &bytes)?;
        let output = self.runner.run(
            &self.config.nix,
            &[
                "store".into(),
                "add".into(),
                "--mode".into(),
                "flat".into(),
                "--hash-algo".into(),
                "sha256".into(),
                "--name".into(),
                "pea.json".into(),
                artifact.into_os_string(),
            ],
            None,
        )?;
        if !output.status.success() {
            bail!("could not import verified pea into Nix");
        }
        let store_path = String::from_utf8(output.stdout)?;
        let path = Path::new(store_path.trim());
        if !path.starts_with("/nix/store") || path.components().count() != 4 {
            bail!("invalid pea store path");
        }
        Ok(())
    }
    pub fn preview_pea(&self, pin: PeaPin, enable: bool) -> Result<Preview> {
        pin.validate()?;
        if enable {
            self.verify_pea(&pin)?;
        }
        let before = self.current_state()?;
        let after = before.with_pea(&pin, enable)?;
        if before == after {
            bail!("pea is already at this pin");
        }
        let mut diff = vec![DiffLine {
            kind: DiffKind::Context,
            text: format!(
                "Official data-only pea {} {}, host API {}, permissions: {}. Source: {}. SHA-256: {}. The host retains execution authority.",
                pin.id,
                pin.version,
                pin.host_api,
                pin.permissions.join(", "),
                pin.url(),
                pin.hash
            ),
        }];
        diff.extend(module_diff(&before, &after)?);
        Ok(Preview {
            packages: vec![],
            before,
            change: ProposalChange::Pea {
                pin: pin.clone(),
                enable,
            },
            title: format!(
                "{} pea {}",
                if enable { "Enable" } else { "Disable" },
                pin.id
            ),
            diff,
        })
    }
}

#[cfg(test)]
mod tests {
    use peasy_core::pea::*;
    #[test]
    fn bundled_catalogue_is_closed_and_every_manifest_matches_its_metadata() {
        let catalogue: PeaCatalogue =
            serde_json::from_str(include_str!("../../../peas/catalogue.json")).unwrap();
        catalogue.validate(&"a".repeat(40)).unwrap();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../peas");
        for entry in catalogue.peas {
            let m = entry.package;
            let manifest: PeaManifest =
                serde_json::from_slice(&std::fs::read(root.join(&m.id).join("pea.json")).unwrap())
                    .unwrap();
            manifest.validate().unwrap();
            let pin = PeaPin {
                id: m.id,
                version: m.version,
                hash: m.hash,
                revision: "a".repeat(40),
                host_api: m.host_api,
                permissions: m.permissions,
            };
            assert!(pin.matches(&manifest));
        }
    }
}

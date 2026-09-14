use super::{NixBackend, Preview};
use anyhow::{Context, Result, bail};
use peasy_core::{
    DiffKind, DiffLine, ProposalChange, module_diff,
    pea::{MAX_PACK_BYTES, PeaManifest, PeaPin, PeaPolicy},
};
use std::{io::Read, path::Path};
impl NixBackend {
    pub(super) fn verify_pea(&self, pin: &PeaPin) -> Result<()> {
        pin.validate()?;
        let policy = PeaPolicy::load(&self.config.pea_policy)?;
        if !policy.allows(pin) {
            bail!("Administrator policy does not allow this official pea or its permissions");
        }
        // Fetch data through Nix using the reviewed hash. No remote Nix is evaluated.
        let output = self.runner.run(
            &self.config.nix,
            &[
                "store".into(),
                "prefetch-file".into(),
                "--json".into(),
                "--hash-type".into(),
                "sha256".into(),
                "--expected-hash".into(),
                pin.hash.clone().into(),
                pin.url().into(),
            ],
            None,
        )?;
        if !output.status.success() {
            bail!("Nix could not fetch the pea at its reviewed hash");
        }
        let value: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        let path = Path::new(
            value["storePath"]
                .as_str()
                .context("Nix returned no pea store path")?,
        );
        if !path.starts_with("/nix/store") || path.components().count() != 4 {
            bail!("invalid pea store path");
        }
        let mut bytes = vec![];
        std::fs::File::open(path)?
            .take(MAX_PACK_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_PACK_BYTES {
            bail!("pea package is too large");
        }
        let manifest: PeaManifest = serde_json::from_slice(&bytes)?;
        manifest.validate()?;
        if !pin.matches(&manifest) {
            bail!("pea metadata does not match its reviewed pin");
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

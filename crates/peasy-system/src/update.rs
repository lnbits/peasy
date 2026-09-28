//! Reviewed self-updates reuse the package transaction, authorization and rollback.
use super::{NixBackend, Preview};
use anyhow::{Context, Result, bail};
use peasy_core::{
    DiffKind, DiffLine, PeasyRelease, PeasyUpdateStatus, ProposalChange, module_diff,
};
use std::{
    io::Read,
    os::unix::fs::OpenOptionsExt,
    time::{Duration, Instant},
};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    enabled: bool,
    version: String,
}

impl NixBackend {
    fn update_policy(&self) -> Result<Policy> {
        let mut bytes = Vec::new();
        std::fs::File::open(
            self.config
                .active_system
                .join("etc/peasy/update-policy.json"),
        )
        .context("This installation needs an initial rebuild to enable Peasy updates")?
        .take(4097)
        .read_to_end(&mut bytes)?;
        if bytes.len() > 4096 {
            bail!("invalid Peasy update policy");
        }
        Ok(serde_json::from_slice(&bytes)?)
    }

    fn fetch_update(&self) -> Result<crate::update_check::PublishedRelease> {
        for operation in ["stop", "start"] {
            let result = self.runner.run(
                &self.config.systemctl,
                &[operation.into(), "peasy-update-check.service".into()],
                None,
            )?;
            if !result.status.success() {
                bail!("Could not check GitHub releases; inspect peasy-update-check.service");
            }
        }
        let output = self
            .config
            .runtime_dir
            .parent()
            .context("invalid runtime directory")?
            .join("peasy-update-check/result.json");
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
            .open(output)?;
        if !file.metadata()?.is_file() {
            bail!("invalid update-check output");
        }
        let mut bytes = Vec::new();
        file.take(8193).read_to_end(&mut bytes)?;
        if bytes.len() > 8192 {
            bail!("update-check output too large");
        }
        let report: serde_json::Value = serde_json::from_slice(&bytes)?;
        if report.as_object().is_none_or(|o| o.len() != 1) {
            bail!("invalid update-check report");
        }
        if let Some(error) = report["error"].as_str() {
            bail!("Could not check Peasy releases: {error}");
        }
        let published: crate::update_check::PublishedRelease =
            serde_json::from_value(report["published"].clone())?;
        if let Some(release) = &published.release {
            release.validate()?;
        }
        Ok(published)
    }

    pub fn check_peasy_update(&self, force: bool) -> Result<PeasyUpdateStatus> {
        let policy = self.update_policy()?;
        if !policy.enabled {
            return Ok(PeasyUpdateStatus {
                current_version: policy.version,
                release: None,
                message: "Peasy update checks are disabled by the system administrator.".into(),
            });
        }
        let mut cache = self.update_cache.try_lock().map_err(|_| {
            anyhow::anyhow!("A Peasy release check is already running; try again shortly")
        })?;
        if let Some((created, status)) = cache.as_ref()
            && (!force || created.elapsed() < Duration::from_secs(30))
            && created.elapsed() < Duration::from_secs(6 * 60 * 60)
            && status.current_version == policy.version
        {
            return Ok(status.clone());
        }
        let published = self.fetch_update()?;
        let mut status = PeasyUpdateStatus {
            current_version: policy.version,
            release: None,
            message: published.message,
        };
        // Older releases may predate updater metadata; still report their version correctly.
        if let Some(version) = &published.latest_version {
            let comparison = PeasyRelease {
                format: 1,
                version: version.clone(),
                tag: format!("v{version}"),
                revision: "0".repeat(40),
                sha256: "0".repeat(64),
            };
            if !comparison.newer_than(&status.current_version)? {
                status.message = "Peasy is up to date with the latest stable release.".into();
            }
        }
        if let Some(release) = published.release {
            if release.newer_than(&status.current_version)? {
                status.message = format!("Peasy {} is available.", release.version);
                status.release = Some(release);
            } else {
                status.message = "Peasy is up to date with the latest stable release.".into();
            }
        }
        *cache = Some((Instant::now(), status.clone()));
        Ok(status)
    }

    pub(super) fn verify_peasy_update(&self, release: &PeasyRelease) -> Result<()> {
        release.validate()?;
        let policy = self.update_policy()?;
        if !policy.enabled {
            bail!("Peasy updates are disabled by the system administrator");
        }
        if !release.newer_than(&policy.version)? {
            bail!("This release is not newer than the installed Peasy; check again");
        }
        let mut cache = self.update_cache.try_lock().map_err(|_| {
            anyhow::anyhow!("A Peasy release check is already running; try again shortly")
        })?;
        *cache = None;
        if self.fetch_update()?.release.as_ref() != Some(release) {
            bail!(
                "The published Peasy release changed or its metadata is unavailable; check and review again"
            );
        }
        Ok(())
    }

    pub(super) fn prepare_peasy_source(
        &self,
        release: &PeasyRelease,
        stage: &std::path::Path,
    ) -> Result<()> {
        release.validate()?;
        // fetchTarball itself downloads inside the evaluator on a cold store.
        // Populate its exact fixed-output path through a Nix daemon build first,
        // so the privileged Peasy process never needs network access. Keep a GC
        // root until activation; the generation then retains the source itself.
        let expression = format!(
            "let pkgs = import {} {{ system = {}; config = {{}}; overlays = []; }}; in pkgs.fetchzip {{ name = \"source\"; url = {}; sha256 = {}; extension = \"tar.gz\"; }}",
            peasy_core::nix_string(&self.config.nixpkgs.to_string_lossy()),
            peasy_core::nix_string(&self.config.system),
            peasy_core::nix_string(&release.source_url()),
            peasy_core::nix_string(&release.sha256),
        );
        let result = self.runner.run(
            &self.config.nix,
            &[
                "build".into(),
                "--impure".into(),
                "--no-write-lock-file".into(),
                "--log-format".into(),
                "internal-json".into(),
                "--expr".into(),
                expression.into(),
                "--out-link".into(),
                stage.join("release-source").into_os_string(),
            ],
            Some(stage),
        )?;
        if !result.status.success() {
            bail!(
                "Could not fetch the verified Peasy release source: {}",
                super::useful_stderr(&result)
            );
        }
        Ok(())
    }

    pub fn preview_peasy_update(&self, release: PeasyRelease) -> Result<Preview> {
        self.verify_peasy_update(&release)?;
        let before = self.current_state()?;
        let after = before.with_peasy_release(&release)?;
        if before == after {
            bail!("This Peasy release is already selected");
        }
        let mut diff = vec![
            DiffLine { kind: DiffKind::Context, text: format!("Update Peasy to {} from {}. This replaces Peasy's native application and NixOS module. It may build from source.", release.version, release.release_url()) },
            DiffLine { kind: DiffKind::Context, text: format!("Pinned revision: {}. Source NAR SHA-256: {}.", release.revision, release.sha256) },
            DiffLine { kind: DiffKind::Context, text: "Keeps the host configuration, hardware and flake lock unchanged. Uses the host's existing Nixpkgs. Review NixOS changes and authenticate to apply. Existing generations remain available for rollback. Reopen Peasy after the update.".into() },
        ];
        diff.extend(module_diff(&before, &after)?);
        Ok(Preview {
            before,
            packages: vec![],
            title: format!("Update Peasy to {}", release.version),
            change: ProposalChange::PeasyUpdate { release },
            diff,
        })
    }
}

//! Official Peasy releases are native code, separate from data-only peas.
use crate::{PackageState, ValidationError, nix_string};
use serde::{Deserialize, Serialize};

pub const UPDATE_ASSET: &str = "peasy-update.json";
pub const UPDATE_FORMAT: u32 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeasyRelease {
    pub format: u32,
    pub version: String,
    pub tag: String,
    pub revision: String,
    /// SHA-256 of the unpacked source NAR, independently published by release CI.
    pub sha256: String,
}

impl PeasyRelease {
    pub fn validate(&self) -> Result<(), ValidationError> {
        let invalid =
            || ValidationError::InvalidRequest("invalid official Peasy release pin".into());
        let version = semver::Version::parse(&self.version).map_err(|_| invalid())?;
        if self.format != UPDATE_FORMAT
            || self.version.len() > 64
            || !version.pre.is_empty()
            || !version.build.is_empty()
            || self.tag != format!("v{}", self.version)
            || self.revision.len() != 40
            || !self
                .revision
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            || self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn newer_than(&self, installed: &str) -> Result<bool, ValidationError> {
        self.validate()?;
        let current = semver::Version::parse(installed).map_err(|_| {
            ValidationError::InvalidRequest(
                "installed Peasy version is unknown; update it through the host configuration"
                    .into(),
            )
        })?;
        Ok(semver::Version::parse(&self.version).expect("validated version") > current)
    }
    pub fn source_url(&self) -> String {
        format!(
            "https://codeload.github.com/lnbits/peasy/tar.gz/{}",
            self.revision
        )
    }
    pub fn release_url(&self) -> String {
        format!("https://github.com/lnbits/peasy/releases/tag/{}", self.tag)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeasyUpdateStatus {
    pub current_version: String,
    pub release: Option<PeasyRelease>,
    pub message: String,
}

impl PackageState {
    pub fn with_peasy_release(&self, release: &PeasyRelease) -> Result<Self, ValidationError> {
        release.validate()?;
        let mut after = self.clone();
        after.peasy_release = Some(release.clone());
        after.normalize()?;
        Ok(after)
    }
}

pub(crate) fn render(release: Option<&PeasyRelease>) -> String {
    let Some(release) = release else {
        return String::new();
    };
    format!(
        r#"
  disabledModules = [ {{ key = "peasy-base-module"; }} ];
  imports = let source = builtins.fetchTarball {{ url = {url}; sha256 = {hash}; }}; in [
    (source + "/nix/update-module.nix")
    ({{ config, ... }}: {{
      system.extraDependencies = [ source ];
      assertions = [ {{
      assertion = config.services.peasy.package.version == {version}
        && (config.services.peasy.package.peasySource or "") == toString source;
      message = "Peasy update conflicts with services.peasy.package; remove that override or update through your host configuration.";
    }} ]; }})
  ];
"#,
        url = nix_string(&release.source_url()),
        hash = nix_string(&release.sha256),
        version = nix_string(&release.version)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    pub fn pin() -> PeasyRelease {
        PeasyRelease {
            format: 1,
            version: "0.2.0".into(),
            tag: "v0.2.0".into(),
            revision: "a".repeat(40),
            sha256: "b".repeat(64),
        }
    }
    #[test]
    fn releases_are_stable_versioned_and_closed() {
        let p = pin();
        assert!(p.newer_than("0.1.9").unwrap());
        assert!(!p.newer_than("0.2.0").unwrap());
        assert!(!p.newer_than("0.10.0").unwrap());
        assert!(p.newer_than("0.2.0-rc.1").unwrap());
        for version in ["1.0", "01.2.0", "0.3.0-rc.1", "0.3.0+build", "${abort}"] {
            let mut bad = p.clone();
            bad.version = version.into();
            bad.tag = format!("v{version}");
            assert!(bad.validate().is_err());
        }
        let mut bad = p.clone();
        bad.tag = "main".into();
        assert!(bad.validate().is_err());
        bad = p;
        bad.sha256 = "z".repeat(64);
        assert!(bad.validate().is_err());
    }
    #[test]
    fn ordinary_configuration_changes_preserve_the_selected_release() {
        let state = PackageState::default().with_peasy_release(&pin()).unwrap();
        let state = state
            .with_change(crate::PackageOperation::Install, "hello")
            .unwrap()
            .with_theme(&crate::ThemeSettings {
                accent_color: Some(crate::AccentColor::Blue),
                color_scheme: None,
            })
            .unwrap();
        let setup: crate::ManagedSetup = serde_json::from_str(include_str!(
            "../../../peas/system_configuration/example.json"
        ))
        .unwrap();
        let state = state.with_setup(setup).unwrap();
        assert_eq!(state.peasy_release, Some(pin()));
        assert_eq!(
            crate::parse_packages_module(&crate::render_packages_module(&state).unwrap()).unwrap(),
            state
        );
    }
    #[test]
    fn updates_roundtrip_and_backups_preserve_the_destination_release() {
        let state = PackageState::default().with_peasy_release(&pin()).unwrap();
        let text = crate::render_packages_module(&state).unwrap();
        assert_eq!(crate::parse_packages_module(&text).unwrap(), state);
        assert!(text.contains("peasy-base-module"));
        let backup = crate::PortableBackup::from_state(&state);
        assert!(backup.state().peasy_release.is_none());
        for mode in [crate::RestoreMode::Merge, crate::RestoreMode::Replace] {
            assert_eq!(
                backup.restore(&state, mode).unwrap().peasy_release,
                state.peasy_release
            );
        }
    }
}

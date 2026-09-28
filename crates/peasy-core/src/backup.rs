//! Closed restore data. Backups never supply executable Nix or destination paths.
use crate::{PackageState, ThemeSettings, ValidationError, pea::PeaPin};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RestoreMode {
    Merge,
    Replace,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PortableBackup {
    pub packages: Vec<String>,
    pub theme: ThemeSettings,
    pub peas: Vec<PeaPin>,
}

impl PortableBackup {
    pub fn from_state(state: &PackageState) -> Self {
        Self {
            packages: state.packages.clone(),
            theme: state.theme.clone(),
            peas: state.peas.clone(),
        }
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.packages.len() > 256 {
            return Err(ValidationError::TooLong);
        }
        self.state().normalize()?;
        // Leave framing room within the daemon's 64 KiB IPC request limit.
        if serde_json::to_vec(self)
            .map_err(|_| ValidationError::TooLong)?
            .len()
            > 60 * 1024
        {
            return Err(ValidationError::TooLong);
        }
        Ok(())
    }

    pub fn state(&self) -> PackageState {
        PackageState {
            packages: self.packages.clone(),
            theme: self.theme.clone(),
            peas: self.peas.clone(),
            ..PackageState::default()
        }
    }

    pub fn restore(
        &self,
        before: &PackageState,
        mode: RestoreMode,
    ) -> Result<PackageState, ValidationError> {
        self.validate()?;
        let mut after = before.clone();
        match mode {
            RestoreMode::Replace => {
                after.packages = self.packages.clone();
                after.theme = self.theme.clone();
                after.peas = self.peas.clone();
            }
            RestoreMode::Merge => {
                after.packages.extend(self.packages.clone());
                after.theme = after.theme.merged(&self.theme);
                for pin in &self.peas {
                    if let Some(existing) = after.peas.iter().find(|p| p.id == pin.id) {
                        if existing != pin {
                            return Err(ValidationError::InvalidRequest(format!(
                                "pea {} has a different installed version; choose Replace to restore the backup's selection",
                                pin.id
                            )));
                        }
                    } else {
                        after.peas.push(pin.clone());
                    }
                }
            }
        }
        after.normalize()?;
        if after.packages.len() > 256 {
            return Err(ValidationError::TooLong);
        }
        // Service/account bindings, network interfaces and architecture-specific
        // binaries on the destination survive BOTH modes.
        Ok(after)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AccentColor, ColorScheme};

    #[test]
    fn merge_and_replace_have_explicit_portable_scope() {
        let before = PackageState {
            packages: vec!["hello".into()],
            theme: ThemeSettings {
                accent_color: Some(AccentColor::Blue),
                color_scheme: Some(ColorScheme::Dark),
            },
            ..PackageState::default()
        };
        let backup = PortableBackup {
            packages: vec!["git".into()],
            theme: ThemeSettings {
                accent_color: Some(AccentColor::Green),
                color_scheme: None,
            },
            peas: vec![],
        };
        let merged = backup.restore(&before, RestoreMode::Merge).unwrap();
        assert_eq!(merged.packages, ["git", "hello"]);
        assert_eq!(merged.theme.color_scheme, Some(ColorScheme::Dark));
        assert_eq!(merged.theme.accent_color, Some(AccentColor::Green));
        let replaced = backup.restore(&before, RestoreMode::Replace).unwrap();
        assert_eq!(replaced.packages, ["git"]);
        assert_eq!(replaced.theme.color_scheme, None);
        assert!(
            serde_json::from_value::<PortableBackup>(
                serde_json::json!({"packages":[],"theme":{},"peas":[],"networks":[]})
            )
            .is_err()
        );
    }

    #[test]
    fn malformed_or_excessive_restore_data_is_rejected() {
        let mut backup = PortableBackup::from_state(&PackageState::default());
        backup.packages = vec!["hello".into(); 257];
        assert!(backup.validate().is_err());
        backup.packages = vec!["hello; builtins.abort".into()];
        assert!(backup.validate().is_err());
        let pin = PeaPin {
            id: "appearance".into(),
            version: "1.1.0".into(),
            revision: "a".repeat(40),
            hash: "b".repeat(64),
            host_api: crate::pea::HOST_API,
            permissions: vec!["appearance".into()],
        };
        let before = PackageState {
            peas: vec![pin.clone()],
            ..PackageState::default()
        };
        backup.packages.clear();
        backup.peas = vec![PeaPin {
            revision: "c".repeat(40),
            ..pin
        }];
        assert!(backup.restore(&before, RestoreMode::Merge).is_err());
        assert!(backup.restore(&before, RestoreMode::Replace).is_ok());
    }
}

//! appimages pea: existing closed types and validation.
use crate::{ValidationError, nix_string, validate_attribute};
use serde::{Deserialize, Serialize};
use std::{fmt, path::Path};

pub const MAX_APPIMAGE_BYTES: u64 = 1024 * 1024 * 1024;
pub const APPIMAGE_POLICY_PATH: &str = "/etc/peasy/appimage-policy.json";

/// `null` permits reviewed installs; a map enforces administrator-approved hashes.
/// Missing policy files fail closed, including during mixed-version upgrades.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(transparent)]
pub struct AppImagePolicy(pub Option<std::collections::BTreeMap<String, Vec<String>>>);

impl Default for AppImagePolicy {
    fn default() -> Self {
        Self(Some(Default::default()))
    }
}

impl AppImagePolicy {
    pub fn load(path: &Path) -> Result<Self, std::io::Error> {
        use std::io::Read;
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error),
        };
        let mut bytes = Vec::new();
        file.take(65537).read_to_end(&mut bytes)?;
        if bytes.len() > 65536 {
            return Err(std::io::Error::other("AppImage policy is too large"));
        }
        serde_json::from_slice(&bytes).map_err(std::io::Error::other)
    }

    pub fn allows_repository(&self, repository: &str) -> bool {
        self.0.as_ref().is_none_or(|trusted| {
            trusted
                .get(&repository.to_ascii_lowercase())
                .is_some_and(|hashes| !hashes.is_empty())
        })
    }

    pub fn is_disabled(&self) -> bool {
        self.0.as_ref().is_some_and(|trusted| trusted.is_empty())
    }

    pub fn allows(&self, package: &AppImagePackage) -> bool {
        package.validate().is_ok()
            && self.0.as_ref().is_none_or(|trusted| {
                trusted
                    .get(&package.repository.to_ascii_lowercase())
                    .is_some_and(|hashes| hashes.contains(&package.hash))
            })
    }
}
pub(crate) fn validate_github_repository(value: &str) -> Result<String, ValidationError> {
    let value = value.trim().trim_end_matches(".git");
    let Some((owner, repository)) = value.split_once('/') else {
        return Err(ValidationError::InvalidRequest(
            "GitHub repository must be owner/name".into(),
        ));
    };
    let valid_part = |part: &str| {
        !part.is_empty()
            && part.len() <= 100
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    };
    if repository.contains('/') || !valid_part(owner) || !valid_part(repository) {
        return Err(ValidationError::InvalidRequest(
            "invalid GitHub repository".into(),
        ));
    }
    Ok(format!(
        "{}/{}",
        owner.to_ascii_lowercase(),
        repository.to_ascii_lowercase()
    ))
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AppImageArchitecture {
    X86_64,
    Aarch64,
}

impl fmt::Display for AppImageArchitecture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::X86_64 => "x86_64",
            Self::Aarch64 => "aarch64",
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AppImagePackage {
    pub id: String,
    pub display_name: String,
    pub repository: String,
    pub version: String,
    pub release_tag: String,
    pub asset_name: String,
    pub url: String,
    pub hash: String,
    pub architecture: AppImageArchitecture,
    pub size: u64,
}

impl AppImagePackage {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_attribute(&self.id)?;
        if !self.id.starts_with("appimage.") {
            return Err(ValidationError::InvalidRequest(
                "external package identifier must use the appimage namespace".into(),
            ));
        }
        validate_display_text(&self.display_name, 120, "AppImage display name")?;
        validate_display_text(&self.version, 64, "AppImage version")?;
        validate_display_text(&self.release_tag, 128, "AppImage release tag")?;
        validate_display_text(&self.asset_name, 240, "AppImage asset name")?;
        if !self.asset_name.to_ascii_lowercase().ends_with(".appimage") {
            return Err(ValidationError::InvalidRequest(
                "external release asset is not an AppImage".into(),
            ));
        }
        if self.size == 0 || self.size > MAX_APPIMAGE_BYTES {
            return Err(ValidationError::InvalidRequest(
                "external AppImage size is outside Peasy's allowed range".into(),
            ));
        }
        let mut repository = self.repository.split('/');
        let owner = repository.next().unwrap_or_default();
        let name = repository.next().unwrap_or_default();
        if repository.next().is_some() || !valid_github_slug(owner) || !valid_github_slug(name) {
            return Err(ValidationError::InvalidRequest(
                "invalid GitHub repository identifier".into(),
            ));
        }
        let expected_id = format!(
            "appimage.{}.{}",
            owner.to_ascii_lowercase(),
            name.to_ascii_lowercase()
        );
        if self.id != expected_id {
            return Err(ValidationError::InvalidRequest(
                "external package identifier does not match its repository".into(),
            ));
        }
        let path = self
            .url
            .strip_prefix("https://github.com/")
            .ok_or_else(|| {
                ValidationError::InvalidRequest(
                    "external AppImages must use a GitHub HTTPS release URL".into(),
                )
            })?;
        if self.url.len() > 2048
            || self.url.contains(['?', '#', '\\', '\n', '\r', '\0'])
            || !path.to_ascii_lowercase().starts_with(&format!(
                "{}/{}/releases/download/",
                owner.to_ascii_lowercase(),
                name.to_ascii_lowercase()
            ))
        {
            return Err(ValidationError::InvalidRequest(
                "external AppImage URL does not match its GitHub repository".into(),
            ));
        }
        let digest = self.hash.strip_prefix("sha256-").unwrap_or_default();
        if digest.len() != 44
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
        {
            return Err(ValidationError::InvalidRequest(
                "external AppImage must have a valid SHA-256 SRI hash".into(),
            ));
        }
        Ok(())
    }

    pub fn pname(&self) -> String {
        self.repository
            .rsplit('/')
            .next()
            .unwrap_or("external-appimage")
            .to_ascii_lowercase()
    }
}

fn valid_github_slug(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn validate_display_text(value: &str, maximum: usize, label: &str) -> Result<(), ValidationError> {
    if value.is_empty()
        || value.len() > maximum
        || value
            .chars()
            .any(|character| character.is_control() || matches!(character, '\'' | '"'))
    {
        return Err(ValidationError::InvalidRequest(format!("invalid {label}")));
    }
    Ok(())
}

pub(crate) fn render_appimage_bindings(
    packages: &[AppImagePackage],
    indent: &str,
) -> Result<String, ValidationError> {
    let mut rendered = String::new();
    for package in packages {
        package.validate()?;
        let pname = nix_string(&package.pname());
        let version = nix_string(&package.version);
        let display_name = nix_string(&package.display_name);
        let url = nix_string(&package.url);
        let hash = nix_string(&package.hash);
        rendered.push_str(&format!(
            "{indent}  (let\n{indent}    wrapped = pkgs.appimageTools.wrapType2 {{\n{indent}      pname = {pname};\n{indent}      version = {version};\n{indent}      src = pkgs.fetchurl {{\n{indent}        url = {url};\n{indent}        hash = {hash};\n{indent}      }};\n{indent}    }};\n{indent}    desktop = pkgs.makeDesktopItem {{\n{indent}      name = {pname};\n{indent}      desktopName = {display_name};\n{indent}      exec = {pname};\n{indent}      icon = \"application-x-executable\";\n{indent}      categories = [ \"Network\" ];\n{indent}    }};\n{indent}  in pkgs.symlinkJoin {{\n{indent}    name = {pname};\n{indent}    paths = [ wrapped desktop ];\n{indent}  }})\n"
        ));
    }
    Ok(rendered)
}

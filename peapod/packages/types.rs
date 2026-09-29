//! packages pea: existing closed types and validation.
use crate::ValidationError;
use serde::{Deserialize, Serialize};
use std::fmt;

pub const MAX_ATTRIBUTE_BYTES: usize = 180;
pub const MAX_CANDIDATES: usize = 12;

pub fn validate_attribute(value: &str) -> Result<&str, ValidationError> {
    if value.is_empty() {
        return Err(ValidationError::Empty);
    }
    if value.len() > MAX_ATTRIBUTE_BYTES {
        return Err(ValidationError::TooLong);
    }
    let valid = value.split('.').all(|part| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'+' | b'-'))
    });
    if !valid {
        return Err(ValidationError::InvalidAttribute(value.to_owned()));
    }
    Ok(value)
}
/// Escape a string for use as a literal Nix search regular expression.
pub fn regex_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(
            ch,
            '.' | '+' | '*' | '?' | '(' | ')' | '|' | '[' | ']' | '{' | '}' | '^' | '$' | '\\'
        ) {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PackageCandidate {
    pub attribute: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub version: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestedVersion {
    Latest,
    Exact(String),
}

impl RequestedVersion {
    pub fn parse(value: &str) -> Result<Self, ValidationError> {
        let value = value.trim();
        if value.eq_ignore_ascii_case("latest") {
            return Ok(Self::Latest);
        }
        if value.is_empty()
            || value.len() > 64
            || !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'+' | b'-' | b'_')
            })
        {
            return Err(ValidationError::InvalidRequest(
                "invalid requested package version".into(),
            ));
        }
        Ok(Self::Exact(value.to_owned()))
    }

    pub fn matches(&self, candidate: &str) -> bool {
        match self {
            Self::Latest => true,
            Self::Exact(requested) => {
                requested.trim_start_matches(['v', 'V']) == candidate.trim_start_matches(['v', 'V'])
            }
        }
    }
}

impl fmt::Display for RequestedVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Latest => f.write_str("latest stable"),
            Self::Exact(version) => f.write_str(version),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageOperation {
    Install,
    Remove,
}

impl fmt::Display for PackageOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Install => "install",
            Self::Remove => "remove",
        })
    }
}

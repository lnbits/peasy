//! wifi pea: existing closed types and validation.
use crate::ValidationError;

pub const MAX_SSID_BYTES: usize = 32;

pub fn validate_ssid(value: &str) -> Result<&str, ValidationError> {
    if value.is_empty() {
        return Err(ValidationError::Empty);
    }
    if value.len() > MAX_SSID_BYTES || value.chars().any(char::is_control) {
        return Err(ValidationError::InvalidRequest(
            "invalid Wi-Fi network name".into(),
        ));
    }
    Ok(value)
}

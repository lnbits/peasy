//! calendar pea: existing closed types and validation.
use crate::ValidationError;

pub const MAX_EVENT_TITLE_BYTES: usize = 160;
pub const LOCAL_DATETIME_BYTES: usize = 19;

pub fn validate_event_title(value: &str) -> Result<&str, ValidationError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ValidationError::Empty);
    }
    if value.len() > MAX_EVENT_TITLE_BYTES || value.chars().any(char::is_control) {
        return Err(ValidationError::InvalidRequest(
            "invalid calendar event title".into(),
        ));
    }
    Ok(value)
}

pub fn validate_local_datetime(value: &str) -> Result<&str, ValidationError> {
    let bytes = value.as_bytes();
    if bytes.len() != LOCAL_DATETIME_BYTES
        || !bytes.iter().enumerate().all(|(index, byte)| match index {
            4 | 7 => *byte == b'-',
            10 => *byte == b'T',
            13 | 16 => *byte == b':',
            _ => byte.is_ascii_digit(),
        })
    {
        return Err(ValidationError::InvalidRequest(
            "calendar start must be a local YYYY-MM-DDTHH:MM:SS value".into(),
        ));
    }
    let number = |range: std::ops::Range<usize>| {
        value[range]
            .parse::<u32>()
            .map_err(|_| ValidationError::InvalidRequest("invalid calendar date".into()))
    };
    let year = number(0..4)?;
    let month = number(5..7)?;
    let day = number(8..10)?;
    let hour = number(11..13)?;
    let minute = number(14..16)?;
    let second = number(17..19)?;
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => 0,
    };
    if day == 0 || day > max_day || hour > 23 || minute > 59 || second > 59 {
        return Err(ValidationError::InvalidRequest(
            "calendar start is not a real local date and time".into(),
        ));
    }
    Ok(value)
}

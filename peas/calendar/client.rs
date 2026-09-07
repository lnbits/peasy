//! calendar pea: unprivileged discovery, review and execution.
use crate::CancellableCommand;
use crate::{
    LocalAction, LocalProposal, LocalResult, PeasyClient, Resolution, safe_stderr, tool_path,
};
use anyhow::{Context, Result, bail};
use peasy_core::{DiffKind, DiffLine, validate_event_title, validate_local_datetime};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

pub(super) fn current_local_time() -> String {
    let date = tool_path("PEASY_DATE", "/run/current-system/sw/bin/date");
    Command::new(date)
        .arg("+%Y-%m-%dT%H:%M:%S %:z %Z")
        .cancellable_output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            format!(
                "Unix timestamp {}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
            )
        })
}

pub(super) fn write_calendar_invite(
    title: &str,
    start_local: &str,
    duration_minutes: u16,
) -> Result<PathBuf> {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from))
        .context("XDG_RUNTIME_DIR or XDG_CACHE_HOME is required for calendar events")?;
    write_calendar_invite_at(&base, title, start_local, duration_minutes)
}

pub(super) fn open_calendar_file(gio: &Path, calendar: &Path) -> Result<()> {
    // GIO uses the freedesktop MIME/default-application association, not GNOME
    // Calendar. GLib is already packaged for the GTK UI; no PIM stack is needed.
    let output = Command::new(gio).arg("open").arg(calendar).cancellable_output()
        .with_context(|| format!("Could not open the event; the .ics file is saved at {}. Configure a default calendar application for text/calendar.", calendar.display()))?;
    if !output.status.success() {
        bail!(
            "Could not open the default calendar application: {}. The .ics file is saved at {}; open/import it manually or configure a text/calendar handler.",
            safe_stderr(&output.stderr),
            calendar.display()
        );
    }
    Ok(())
}

pub(super) fn fold_ical_line(line: &str) -> String {
    let mut folded = String::new();
    let mut octets = 0;
    for ch in line.chars() {
        if octets + ch.len_utf8() > 75 {
            folded.push_str("\r\n ");
            octets = 1;
        }
        folded.push(ch);
        octets += ch.len_utf8();
    }
    folded
}

pub(super) fn write_calendar_invite_at(
    base: &Path,
    title: &str,
    start_local: &str,
    duration_minutes: u16,
) -> Result<PathBuf> {
    validate_event_title(title)?;
    validate_local_datetime(start_local)?;
    if !(5..=1440).contains(&duration_minutes) {
        bail!("calendar duration must be between 5 minutes and 24 hours");
    }
    let stamp = Command::new(tool_path("PEASY_DATE", "/run/current-system/sw/bin/date"))
        .args(["-u", "+%Y%m%dT%H%M%SZ"])
        .cancellable_output()
        .context("creating calendar timestamp")?;
    let stamp_text = String::from_utf8_lossy(&stamp.stdout);
    let stamp_text = stamp_text.trim();
    if !stamp.status.success()
        || stamp_text.len() != 16
        || !stamp_text.bytes().enumerate().all(|(i, b)| match i {
            8 => b == b'T',
            15 => b == b'Z',
            _ => b.is_ascii_digit(),
        })
    {
        bail!("could not generate a valid calendar timestamp");
    }
    let directory = base.join("peasy/calendar");
    fs::create_dir_all(&directory)?;
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = directory.join(format!("event-{nonce}.ics"));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    let summary = title
        .replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,");
    let start = start_local.replace(['-', ':'], "");
    let summary = fold_ical_line(&format!("SUMMARY:{summary}"));
    let contents = format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Peasy//EN\r\nBEGIN:VEVENT\r\nUID:{nonce}@peasy.local\r\nDTSTAMP:{stamp_text}\r\nDTSTART:{start}\r\nDURATION:PT{duration_minutes}M\r\n{summary}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
    );
    file.write_all(contents.as_bytes())?;
    file.sync_all()?;
    Ok(path)
}

impl PeasyClient {
    pub(super) fn propose_calendar(
        &self,
        title: String,
        start_local: String,
        duration_minutes: u16,
    ) -> Result<Resolution> {
        validate_event_title(&title)?;
        validate_local_datetime(&start_local)?;
        if !(5..=1440).contains(&duration_minutes) {
            bail!("calendar duration must be between 5 minutes and 24 hours");
        }
        Ok(Resolution::LocalProposal(LocalProposal {
            title: format!("Create calendar event: {title}"),
            diff: vec![
                DiffLine {
                    kind: DiffKind::Add,
                    text: format!("Title: {title}"),
                },
                DiffLine {
                    kind: DiffKind::Add,
                    text: format!("Starts: {start_local} (local time)"),
                },
                DiffLine {
                    kind: DiffKind::Add,
                    text: format!("Duration: {duration_minutes} minutes"),
                },
            ],
            action: LocalAction::Calendar {
                title,
                start_local,
                duration_minutes,
            },
        }))
    }

    pub(super) fn apply_calendar(
        &self,
        title: &str,
        start_local: &str,
        duration_minutes: &u16,
    ) -> Result<LocalResult> {
        let calendar = write_calendar_invite(title, start_local, *duration_minutes)?;
        open_calendar_file(&self.tools.gio, &calendar)?;
        Ok(LocalResult {
            completed: true,
            message: format!(
                "The event was handed to your default application for review/import. The iCalendar file is saved at {}.",
                calendar.display()
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn calendar_invite_contains_only_validated_ics_data() {
        let temp = tempfile::tempdir().unwrap();
        let path =
            write_calendar_invite_at(temp.path(), "Walk with Dad", "2026-09-27T10:00:00", 60)
                .unwrap();
        let contents = fs::read_to_string(path).unwrap();
        assert!(contents.contains("DTSTART:20260927T100000"));
        assert!(contents.contains("DURATION:PT60M"));
        assert!(contents.contains("SUMMARY:Walk with Dad"));
        let timestamp = contents
            .lines()
            .find(|line| line.starts_with("DTSTAMP:"))
            .unwrap();
        assert_eq!(timestamp.len(), 24);
        assert!(timestamp.ends_with('Z'));
        let calendar = write_calendar_invite_at(
            temp.path(),
            "Planning; Q4, review",
            "2028-02-29T23:59:59",
            30,
        )
        .unwrap();
        let mode = fs::metadata(&calendar).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let contents = fs::read_to_string(calendar).unwrap();
        assert!(contents.contains("SUMMARY:Planning\\; Q4\\, review"));
        assert!(
            write_calendar_invite_at(
                temp.path(),
                "Injected\nDESCRIPTION:bad",
                "2026-09-27T10:00:00",
                60,
            )
            .is_err()
        );
    }

    #[test]
    fn calendar_folds_utf8_and_retains_file_when_handler_is_missing() {
        let temp = tempfile::tempdir().unwrap();
        let title = "Café 🌿 ".repeat(8).trim().to_owned();
        let calendar =
            write_calendar_invite_at(temp.path(), &title, "2026-09-27T10:00:00", 30).unwrap();
        let contents = fs::read_to_string(&calendar).unwrap();
        assert!(contents.split("\r\n").all(|line| line.len() <= 75));
        assert!(
            contents
                .replace("\r\n ", "")
                .contains(&format!("SUMMARY:{title}"))
        );
        let error = open_calendar_file(&temp.path().join("missing-gio"), &calendar)
            .unwrap_err()
            .to_string();
        assert!(error.contains(calendar.to_str().unwrap()));
        assert!(error.contains("text/calendar"));
        assert_eq!(fs::read_to_string(&calendar).unwrap(), contents);
        assert!(write_calendar_invite_at(temp.path(), "Test", "2026-09-27T10:00:00", 0).is_err());
    }
}

//! Reconcile generation values while preserving the user's original settings.
use super::adapters;
use anyhow::{Context, Result, bail};
use peasy_core::{DesktopEnvironment, ThemeSettings};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::Path,
    process::Command,
};

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Baseline {
    accent: Option<String>,
    scheme: Option<String>,
}

pub(super) struct Tools<'a> {
    pub gsettings: &'a Path,
    pub dconf: &'a Path,
    pub plasma: &'a Path,
    pub kconfig: &'a Path,
    pub kread: &'a Path,
}

fn command(program: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .context("synchronizing desktop appearance")?;
    command_value(output)
}

fn command_value(output: std::process::Output) -> Result<String> {
    if !output.status.success() {
        bail!(
            "Desktop appearance command failed: {}",
            crate::safe_stderr(&output.stderr)
        );
    }
    let value = String::from_utf8(output.stdout)?.trim_end().to_owned();
    if value.len() > 1024 || value.chars().any(char::is_control) {
        bail!("invalid desktop appearance value");
    }
    Ok(value)
}

fn read_value(
    desktop: DesktopEnvironment,
    accent: bool,
    tools: &Tools<'_>,
    directory: &Path,
) -> Result<String> {
    if desktop == DesktopEnvironment::Gnome {
        // Read only the user database. Effective gsettings values already
        // include the new generation's defaults by the time this service runs.
        let profile = directory.join("user-profile");
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .custom_flags(libc::O_NOFOLLOW)
            .mode(0o600)
            .open(&profile)?;
        file.write_all(b"user-db:user\n")?;
        command_value(
            Command::new(tools.dconf)
                .env("DCONF_PROFILE", profile)
                .args([
                    "read",
                    if accent {
                        "/org/gnome/desktop/interface/accent-color"
                    } else {
                        "/org/gnome/desktop/interface/color-scheme"
                    },
                ])
                .output()
                .context("reading original user appearance")?,
        )
    } else {
        read_kde(accent, tools)
    }
}

fn read_kde(accent: bool, tools: &Tools<'_>) -> Result<String> {
    command(
        tools.kread,
        &[
            "--file",
            "kdeglobals",
            "--group",
            "General",
            "--key",
            if accent { "AccentColor" } else { "ColorScheme" },
            "--default",
            if accent { "0,0,0" } else { "BreezeLight" },
        ],
    )
}

fn write_kde(tools: &Tools<'_>, key: &str, value: &str) -> Result<()> {
    command(
        tools.kconfig,
        &[
            "--file",
            "kdeglobals",
            "--group",
            "General",
            "--key",
            key,
            value,
        ],
    )?;
    Ok(())
}

fn restore(
    desktop: DesktopEnvironment,
    accent: bool,
    value: &str,
    tools: &Tools<'_>,
) -> Result<()> {
    if value.len() > 1024 || value.chars().any(char::is_control) {
        bail!("invalid saved desktop appearance value");
    }
    if desktop == DesktopEnvironment::Gnome {
        let key = if accent {
            "accent-color"
        } else {
            "color-scheme"
        };
        if value.is_empty() {
            command(
                tools.gsettings,
                &["reset", "org.gnome.desktop.interface", key],
            )?;
        } else {
            command(
                tools.gsettings,
                &["set", "org.gnome.desktop.interface", key, value],
            )?;
        }
    } else {
        // The Plasma CLI is a no-op for the current scheme. An empty key also
        // resolves to BreezeLight, so use a fixed temporary name to force a
        // palette reload, including when restoring the default light scheme.
        let scheme = if accent {
            write_kde(tools, "AccentColor", value)?;
            read_kde(false, tools)?
        } else {
            value.into()
        };
        if scheme.is_empty() || scheme.starts_with('-') || scheme.contains('/') {
            bail!("invalid saved Plasma colour scheme");
        }
        write_kde(tools, "ColorScheme", "PeasyPendingRestore")?;
        if let Err(error) = command(tools.plasma, &[&scheme]) {
            write_kde(tools, "ColorScheme", &scheme)?;
            return Err(error);
        }
        write_kde(tools, "ColorScheme", &scheme)?;
    }
    Ok(())
}

fn save(path: &Path, baseline: &Baseline) -> Result<()> {
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        serde_json::to_writer(&mut file, baseline)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub(super) fn reconcile(
    desktop: DesktopEnvironment,
    load_theme: impl FnOnce() -> Result<ThemeSettings>,
    directory: &Path,
    tools: &Tools<'_>,
) -> Result<()> {
    let name = match desktop {
        DesktopEnvironment::Gnome => "gnome",
        DesktopEnvironment::KdePlasma => "plasma",
        _ => {
            return desktop
                .validate_appearance(&load_theme()?)
                .map_err(Into::into);
        }
    };
    fs::create_dir_all(directory)?;
    let metadata = fs::symlink_metadata(directory)?;
    if !metadata.is_dir() || metadata.uid() != unsafe { libc::geteuid() } {
        bail!("appearance state directory must belong to the current user");
    }
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(directory.join(format!("{name}.lock")))?;
    // GUI updates and the generation watcher may run concurrently.
    lock.lock()?;
    // Read after taking the lock: a waiting GUI invocation must not reapply a
    // generation that a newer watcher has already replaced.
    let theme = load_theme()?;
    desktop.validate_appearance(&theme)?;
    let path = directory.join(format!("{name}.json"));
    let mut baseline = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
    {
        Ok(file) => {
            let meta = file.metadata()?;
            if !meta.is_file()
                || meta.uid() != unsafe { libc::geteuid() }
                || meta.mode() & 0o077 != 0
            {
                bail!("appearance baseline must be a private user-owned regular file");
            }
            let mut bytes = vec![];
            file.take(8193).read_to_end(&mut bytes)?;
            if bytes.len() > 8192 {
                bail!("appearance baseline exceeds its limit");
            }
            serde_json::from_slice::<Baseline>(&bytes)?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Baseline::default(),
        Err(error) => return Err(error.into()),
    };
    if theme.accent_color.is_some() && baseline.accent.is_none() {
        baseline.accent = Some(read_value(desktop, true, tools, directory)?);
    }
    if theme.color_scheme.is_some() && baseline.scheme.is_none() {
        baseline.scheme = Some(read_value(desktop, false, tools, directory)?);
    }
    // Persist ownership before any mutation, so retries and partial failures
    // cannot replace the original baseline with Peasy's own override.
    save(&path, &baseline)?;
    if theme.accent_color.is_none()
        && let Some(value) = &baseline.accent
    {
        restore(desktop, true, value, tools)?;
    }
    if theme.color_scheme.is_none()
        && let Some(value) = &baseline.scheme
    {
        restore(desktop, false, value, tools)?;
    }
    adapters::apply(
        desktop,
        &theme,
        tools.gsettings,
        tools.plasma,
        tools.kconfig,
    )?;
    if theme.accent_color.is_none() {
        baseline.accent = None;
    }
    if theme.color_scheme.is_none() {
        baseline.scheme = None;
    }
    save(&path, &baseline)
}

#[cfg(test)]
mod tests {
    use super::*;
    use peasy_core::{AccentColor, ColorScheme};

    fn reconcile(
        desktop: DesktopEnvironment,
        theme: &ThemeSettings,
        directory: &Path,
        tools: &Tools<'_>,
    ) -> Result<()> {
        super::reconcile(desktop, || Ok(theme.clone()), directory, tools)
    }

    #[test]
    fn an_unset_gnome_override_is_reset_and_failed_mutations_keep_the_baseline() {
        let temp = tempfile::tempdir().unwrap();
        let tool = temp.path().join("tool");
        let log = temp.path().join("log");
        let fail = temp.path().join("fail");
        fs::write(&tool, format!("#!/bin/sh\ncase \"$1\" in\nwritable) echo true;;\nread) exit 0;;\n*) test ! -f '{}' || exit 1; printf '%s\\n' \"$@\" >> '{}' ;;\nesac\n", fail.display(), log.display())).unwrap();
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o700)).unwrap();
        let tools = Tools {
            gsettings: &tool,
            dconf: &tool,
            plasma: &tool,
            kconfig: &tool,
            kread: &tool,
        };
        let state = temp.path().join("state");
        let theme = ThemeSettings {
            accent_color: Some(AccentColor::Blue),
            color_scheme: None,
        };
        fs::write(&fail, "").unwrap();
        assert!(reconcile(DesktopEnvironment::Gnome, &theme, &state, &tools).is_err());
        let baseline: Baseline =
            serde_json::from_slice(&fs::read(state.join("gnome.json")).unwrap()).unwrap();
        assert_eq!(baseline.accent.as_deref(), Some(""));
        fs::remove_file(&fail).unwrap();
        reconcile(
            DesktopEnvironment::Gnome,
            &ThemeSettings::default(),
            &state,
            &tools,
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(&log).unwrap(),
            "reset\norg.gnome.desktop.interface\naccent-color\n"
        );
    }

    #[test]
    fn plasma_restores_the_original_accent_and_forces_a_palette_refresh() {
        let temp = tempfile::tempdir().unwrap();
        let read = temp.path().join("read");
        let write = temp.path().join("write");
        let log = temp.path().join("log");
        fs::write(&read, "#!/bin/sh\ncase \"$6\" in AccentColor) echo 1,2,3;; ColorScheme) echo OriginalScheme;; esac\n").unwrap();
        fs::write(
            &write,
            format!("#!/bin/sh\nprintf '%s\\n' \"$@\" >> '{}'\n", log.display()),
        )
        .unwrap();
        for tool in [&read, &write] {
            fs::set_permissions(tool, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let tools = Tools {
            gsettings: Path::new("/never-use-gnome"),
            dconf: Path::new("/never-use-dconf"),
            plasma: &write,
            kconfig: &write,
            kread: &read,
        };
        let state = temp.path().join("state");
        reconcile(
            DesktopEnvironment::KdePlasma,
            &ThemeSettings {
                accent_color: Some(AccentColor::Blue),
                color_scheme: Some(ColorScheme::Dark),
            },
            &state,
            &tools,
        )
        .unwrap();
        fs::write(&log, "").unwrap();
        reconcile(
            DesktopEnvironment::KdePlasma,
            &ThemeSettings::default(),
            &state,
            &tools,
        )
        .unwrap();
        let calls = fs::read_to_string(log).unwrap();
        assert!(calls.contains("AccentColor\n1,2,3\n"));
        assert!(calls.contains("ColorScheme\nPeasyPendingRestore\nOriginalScheme\n"));
    }

    #[test]
    fn rollback_restores_only_owned_keys_and_keeps_the_original_baseline() {
        let temp = tempfile::tempdir().unwrap();
        let tool = temp.path().join("tool");
        let log = temp.path().join("log");
        fs::write(&tool, format!("#!/bin/sh\ncase \"$1\" in\nwritable) echo true;;\nread) echo \"'original-${{2##*/}}'\";;\n*) printf '%s\\n' \"$@\" >> '{}' ;;\nesac\n", log.display())).unwrap();
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o700)).unwrap();
        let tools = Tools {
            gsettings: &tool,
            dconf: &tool,
            plasma: &tool,
            kconfig: &tool,
            kread: &tool,
        };
        let state = temp.path().join("state");
        let desktop = DesktopEnvironment::Gnome;
        reconcile(desktop, &ThemeSettings::default(), &state, &tools).unwrap();
        assert!(!log.exists());
        let mut theme = ThemeSettings {
            accent_color: Some(AccentColor::Blue),
            color_scheme: Some(ColorScheme::Dark),
        };
        reconcile(desktop, &theme, &state, &tools).unwrap();
        theme.accent_color = Some(AccentColor::Green);
        reconcile(desktop, &theme, &state, &tools).unwrap();
        theme.accent_color = None;
        reconcile(desktop, &theme, &state, &tools).unwrap();
        assert!(
            fs::read_to_string(&log)
                .unwrap()
                .contains("'original-accent-color'")
        );
        reconcile(desktop, &ThemeSettings::default(), &state, &tools).unwrap();
        let output = fs::read_to_string(&log).unwrap();
        assert_eq!(output.matches("'original-accent-color'").count(), 1);
        assert_eq!(output.matches("'original-color-scheme'").count(), 1);
        reconcile(desktop, &ThemeSettings::default(), &state, &tools).unwrap();
        assert_eq!(fs::read_to_string(&log).unwrap(), output);
    }
}

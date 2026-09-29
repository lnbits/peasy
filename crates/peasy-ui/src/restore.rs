//! Read backup data without evaluating any file as Nix or executing bundled code.
use anyhow::{Context, Result, bail};
use peasy_core::{PackageState, PortableBackup, parse_packages_module};
use std::{fs::OpenOptions, io::Read, os::unix::fs::OpenOptionsExt, path::Path};
const MAX_BACKUP_FILE: u64 = 4 * 1024 * 1024;

pub(super) struct Backup {
    pub portable: PortableBackup,
    pub deferred: String,
}

fn read_file(directory: &std::fs::File, name: &str) -> Result<Vec<u8>> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let name = std::ffi::CString::new(name)?;
    // Bind both reads to one selected directory; reject symlinks, devices and
    // FIFOs before reading. The daemon receives typed values, never this path.
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    if !file.metadata()?.is_file() {
        bail!("backup entry must be a regular file");
    }
    let mut bytes = vec![];
    file.take(MAX_BACKUP_FILE + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BACKUP_FILE {
        bail!("backup file exceeds 4 MiB");
    }
    Ok(bytes)
}

pub(super) fn read_backup(path: &Path) -> Result<Backup> {
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let module = String::from_utf8(
        read_file(&directory, "peasy-managed.nix")
            .context("Choose an exported Peasy backup folder containing peasy-managed.nix")?,
    )?;
    let state =
        parse_packages_module(&module).context("Backup module is invalid or was modified")?;
    let portable = PortableBackup::from_state(&state);
    portable.validate()?;
    if portable.state() != state {
        bail!("Backup module contains machine-specific settings; export a portable backup first");
    }
    let review: serde_json::Value =
        serde_json::from_slice(&read_file(&directory, "RESTORE-REVIEW.json")?)?;
    if review["format"] != 1 {
        bail!("Unsupported backup format; update Peasy or export a new backup");
    }
    let mut original: PackageState = serde_json::from_value(review["active_state"].clone())?;
    original.normalize()?;
    if PortableBackup::from_state(&original) != portable {
        bail!("Backup files disagree; choose an intact backup");
    }
    let mut deferred = Vec::new();
    deferred.extend(original.setups.iter().map(|s| {
        format!(
            "Service setup: {} — review services and user access",
            s.package
        )
    }));
    deferred.extend(
        original
            .networks
            .iter()
            .map(|n| format!("Network: {} — select a destination interface", n.id)),
    );
    deferred.extend(
        original
            .appimages
            .iter()
            .map(|a| format!("AppImage: {} — select a compatible release", a.display_name)),
    );
    let deferred = if deferred.is_empty() {
        "No saved service setups, network profiles or AppImages need separate review.".into()
    } else {
        format!(
            "These saved items are not applied by this restore. Recreate them through Peasy on this machine:\n\n{}",
            deferred.join("\n")
        )
    };
    Ok(Backup { portable, deferred })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(path: &Path) {
        let state = PackageState {
            packages: vec!["hello".into()],
            ..PackageState::default()
        };
        std::fs::write(
            path.join("peasy-managed.nix"),
            peasy_core::render_packages_module(&state).unwrap(),
        )
        .unwrap();
        std::fs::write(
            path.join("RESTORE-REVIEW.json"),
            serde_json::to_vec(&serde_json::json!({"format":1,"active_state":state})).unwrap(),
        )
        .unwrap();
    }
    #[test]
    fn reads_portable_backup_and_rejects_tampering_and_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        fixture(temp.path());
        assert_eq!(
            read_backup(temp.path()).unwrap().portable.packages,
            ["hello"]
        );
        let module = temp.path().join("peasy-managed.nix");
        let original = std::fs::read_to_string(&module).unwrap();
        std::fs::write(&module, format!("{original}\n# injected Nix")).unwrap();
        assert!(read_backup(temp.path()).is_err());
        std::fs::remove_file(&module).unwrap();
        std::os::unix::fs::symlink("RESTORE-REVIEW.json", &module).unwrap();
        assert!(read_backup(temp.path()).is_err());
    }
    #[test]
    fn machine_specific_module_and_nonregular_entries_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        fixture(temp.path());
        let plan: peasy_core::NetworkPlan =
            serde_json::from_str(include_str!("../../../peapod/networking/example.json")).unwrap();
        let state = PackageState {
            networks: plan.profiles,
            ..PackageState::default()
        };
        std::fs::write(
            temp.path().join("peasy-managed.nix"),
            peasy_core::render_packages_module(&state).unwrap(),
        )
        .unwrap();
        assert!(read_backup(temp.path()).is_err());
        std::fs::remove_file(temp.path().join("peasy-managed.nix")).unwrap();
        std::fs::create_dir(temp.path().join("peasy-managed.nix")).unwrap();
        assert!(read_backup(temp.path()).is_err());
    }

    #[test]
    fn rejects_disagreeing_files_unsupported_formats_and_large_files() {
        let temp = tempfile::tempdir().unwrap();
        for review in [
            serde_json::json!({"format":2,"active_state":PackageState::default()}),
            serde_json::json!({"format":1,"active_state":PackageState::default()}),
        ] {
            fixture(temp.path());
            std::fs::write(
                temp.path().join("RESTORE-REVIEW.json"),
                serde_json::to_vec(&review).unwrap(),
            )
            .unwrap();
            assert!(read_backup(temp.path()).is_err());
        }
        fixture(temp.path());
        std::fs::write(
            temp.path().join("peasy-managed.nix"),
            vec![b'x'; MAX_BACKUP_FILE as usize + 1],
        )
        .unwrap();
        assert!(read_backup(temp.path()).is_err());
    }
}

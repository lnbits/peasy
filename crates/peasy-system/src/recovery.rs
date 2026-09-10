//! A durable operation journal, not an alternative source of desired state.
use anyhow::{Context, Result, bail};
use peasy_core::{PackageState, RecoveryInfo};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum Phase {
    Building,
    Activating,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Journal {
    pub before: PackageState,
    pub proposed: PackageState,
    pub previous_generation: Option<PathBuf>,
    pub target_generation: Option<PathBuf>,
    pub phase: Phase,
}

pub fn path(managed: &Path) -> PathBuf {
    managed
        .parent()
        .expect("absolute managed path")
        .join("transaction.json")
}
fn notice_path(managed: &Path) -> PathBuf {
    managed
        .parent()
        .expect("absolute managed path")
        .join("recovery.json")
}

pub fn write(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path.parent().context("journal has no parent")?;
    let tmp = parent.join(format!(
        ".journal-{}.tmp",
        hex::encode(rand::random::<[u8; 12]>())
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        serde_json::to_writer(&mut file, value)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&tmp, path)?;
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    let _ = fs::remove_file(tmp);
    result
}

fn read<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Option<T>> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != nix::unistd::geteuid().as_raw()
        || metadata.mode() & 0o077 != 0
    {
        bail!("unsafe recovery record");
    }
    let mut bytes = Vec::new();
    file.take(4 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 4 * 1024 * 1024 {
        bail!("oversized recovery record");
    }
    Ok(Some(serde_json::from_slice(&bytes)?))
}

pub fn load(managed: &Path) -> Result<Option<Journal>> {
    let mut journal: Option<Journal> = read(&path(managed))?;
    if let Some(j) = &mut journal {
        j.before.normalize()?;
        j.proposed.normalize()?;
        for generation in [&j.previous_generation, &j.target_generation]
            .into_iter()
            .flatten()
        {
            if !generation.starts_with("/nix/store") || generation.components().count() != 4 {
                bail!("invalid recovery generation");
            }
        }
    }
    Ok(journal)
}

pub fn clear(managed: &Path) -> Result<()> {
    match fs::remove_file(path(managed)) {
        Ok(()) => fs::File::open(managed.parent().unwrap())?.sync_all()?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

pub fn generation(active: &Path) -> Option<PathBuf> {
    fs::canonicalize(active)
        .ok()
        .filter(|p| p.starts_with("/nix/store") && p.components().count() == 4)
}

pub fn info(managed: &Path, active: &Path) -> Result<Option<RecoveryInfo>> {
    if let Some(j) = load(managed)? {
        return Ok(Some(describe(
            &j,
            active,
            true,
            "An interrupted system change needs review. Activation may have partially changed the running system. Restore the previous generation, or inspect the system before making another change.",
        )));
    }
    let mut notice: Option<RecoveryInfo> = read(&notice_path(managed))?;
    if let Some(notice) = &mut notice {
        notice.active_generation = generation(active).map(|p| p.to_string_lossy().into_owned());
    }
    Ok(notice)
}

fn describe(j: &Journal, active: &Path, needs_attention: bool, message: &str) -> RecoveryInfo {
    RecoveryInfo {
        message: message.into(),
        intended_change: peasy_core::module_diff(&j.before, &j.proposed)
            .unwrap_or_default()
            .into_iter()
            .take(100)
            .collect(),
        needs_attention,
        intended_packages: j
            .proposed
            .packages
            .iter()
            .cloned()
            .chain(j.proposed.setups.iter().map(|s| s.package.clone()))
            .chain(j.proposed.appimages.iter().map(|p| p.id.clone()))
            .collect(),
        active_generation: generation(active).map(|p| p.to_string_lossy().into_owned()),
        previous_generation: j
            .previous_generation
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned()),
    }
}

pub fn startup(managed: &Path, active: &Path) -> Result<()> {
    if let Some(j) = load(managed)? {
        let current = crate::state::load_managed(managed)?;
        // Only undo our own unfinished write. Never overwrite a concurrent
        // administrator change or an activation that could still be running.
        if j.phase == Phase::Building
            && generation(active) == j.previous_generation
            && (current == j.proposed || current == j.before)
        {
            crate::state::write_managed_atomic(managed, &j.before)?;
            write(
                &notice_path(managed),
                &describe(
                    &j,
                    active,
                    false,
                    "An interrupted build was detected. Peasy restored the previous configuration; no activation was started. You can review and retry the request.",
                ),
            )?;
            clear(managed)?;
        }
    }
    Ok(())
}

pub fn completed(managed: &Path) -> Result<()> {
    clear(managed)?;
    let _ = fs::remove_file(notice_path(managed));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restart_restores_only_an_unactivated_owned_write() {
        let tmp = tempfile::tempdir().unwrap();
        let managed = tmp.path().join("managed.nix");
        let before = PackageState::default();
        let proposed = before
            .with_change(peasy_core::PackageOperation::Install, "hello")
            .unwrap();
        crate::state::write_managed_atomic(&managed, &proposed).unwrap();
        let mut journal = Journal {
            before: before.clone(),
            proposed: proposed.clone(),
            previous_generation: None,
            target_generation: None,
            phase: Phase::Building,
        };
        write(&path(&managed), &journal).unwrap();
        startup(&managed, &tmp.path().join("absent")).unwrap();
        assert_eq!(crate::state::load_managed(&managed).unwrap(), before);
        assert!(
            !info(&managed, &tmp.path().join("absent"))
                .unwrap()
                .unwrap()
                .needs_attention
        );
        let mut notice: RecoveryInfo = read(&notice_path(&managed)).unwrap().unwrap();
        notice.active_generation = Some("/nix/store/old-generation".into());
        write(&notice_path(&managed), &notice).unwrap();
        assert_eq!(
            info(&managed, &tmp.path().join("absent"))
                .unwrap()
                .unwrap()
                .active_generation,
            None
        );
        journal.phase = Phase::Activating;
        crate::state::write_managed_atomic(&managed, &proposed).unwrap();
        write(&path(&managed), &journal).unwrap();
        startup(&managed, &tmp.path().join("absent")).unwrap();
        assert_eq!(crate::state::load_managed(&managed).unwrap(), proposed);
        assert!(
            info(&managed, &tmp.path().join("absent"))
                .unwrap()
                .unwrap()
                .needs_attention
        );
    }
}

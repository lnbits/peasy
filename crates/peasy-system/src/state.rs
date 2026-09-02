use anyhow::{Context, Result, bail};
use peasy_core::{PackageState, parse_packages_module, render_packages_module};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

pub fn load_managed(path: &Path) -> Result<PackageState> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                bail!("Peasy managed module is not a regular file");
            }
            if metadata.len() > 1024 * 1024 {
                bail!("Peasy managed module is unexpectedly large");
            }
            let source = fs::read_to_string(path).context("reading Peasy managed module")?;
            parse_packages_module(&source).context("parsing Peasy managed module")
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(PackageState::default()),
        Err(error) => Err(error).context("inspecting Peasy managed module"),
    }
}

pub fn write_managed_atomic(path: &Path, state: &PackageState) -> Result<()> {
    if !path.is_absolute() {
        bail!("Peasy managed module path must be absolute");
    }
    let parent = path.parent().context("managed module path has no parent")?;
    fs::create_dir_all(parent)?;
    if let Ok(metadata) = fs::symlink_metadata(path)
        && (metadata.file_type().is_symlink() || !metadata.is_file())
    {
        bail!("refusing to replace a non-regular Peasy managed module");
    }
    let temporary = parent.join(format!(".peasy-managed-{}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .open(&temporary)
        .context("creating temporary Peasy managed module")?;
    file.set_permissions(fs::Permissions::from_mode(0o644))?;
    file.write_all(render_packages_module(state)?.as_bytes())?;
    file.sync_all()?;
    fs::rename(&temporary, path)?;
    let directory = OpenOptions::new().read(true).open(parent)?;
    directory.sync_all()?;
    Ok(())
}

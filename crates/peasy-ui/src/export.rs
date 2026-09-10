use anyhow::{Context, Result};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

const PEASY_SOURCE: &str = "/run/current-system/sw/share/peasy/source";
const PEASY_MODULE_POINTER: &str = "/etc/peasy/module-import-path";
const HOST_CONFIGURATION_POINTER: &str = "/etc/peasy/host-configuration-path";
const MAX_CONFIGURATION_BYTES: u64 = 4 * 1024 * 1024;
const MAX_EXPORT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_EXPORT_ENTRIES: usize = 4096;
const EXPORT_DIRECTORY: &str = "peasy-system-config";

#[derive(Clone, Debug)]
pub(super) struct ConfigurationExport {
    source: PathBuf,
    peasy_module: PathBuf,
    peasy_source: PathBuf,
}

#[derive(Default)]
struct ExportSize {
    bytes: u64,
    entries: usize,
    excluded: Vec<String>,
}

pub(super) fn configuration_export(_socket: &Path) -> Result<ConfigurationExport> {
    configuration_export_from(
        Path::new(HOST_CONFIGURATION_POINTER),
        Path::new(PEASY_MODULE_POINTER),
        Path::new(PEASY_SOURCE),
    )
}

fn configuration_export_from(
    pointer: &Path,
    peasy_module_pointer: &Path,
    peasy_source: &Path,
) -> Result<ConfigurationExport> {
    let source = configured_source_path(pointer)?;
    let metadata = fs::metadata(&source)
        .with_context(|| format!("reading metadata for {}", source.display()))?;
    if !metadata.is_file() {
        anyhow::bail!("{} is not a regular configuration file", source.display());
    }
    if metadata.len() > MAX_CONFIGURATION_BYTES {
        anyhow::bail!("configuration is larger than 4 MiB");
    }
    let name = source
        .file_name()
        .and_then(|name| name.to_str())
        .context("configured source has no portable file name")?;
    if name != "configuration.nix" {
        anyhow::bail!("portable system export currently requires a configuration.nix host source");
    }
    let peasy_module = configured_path(peasy_module_pointer, "/nix/store/peasy/nix/module.nix")?;
    if !peasy_source.is_dir() {
        anyhow::bail!("installed Peasy source is unavailable for the portable export");
    }
    Ok(ConfigurationExport {
        source,
        peasy_module,
        peasy_source: peasy_source.to_owned(),
    })
}

pub(super) fn write_configuration_export(
    export: &ConfigurationExport,
    parent: &Path,
) -> Result<PathBuf> {
    if !parent.is_dir() {
        anyhow::bail!("export destination is not a directory");
    }
    let source_root = export
        .source
        .parent()
        .context("configured source has no parent directory")?;
    if fs::canonicalize(parent)?.starts_with(fs::canonicalize(source_root)?) {
        anyhow::bail!("choose a destination outside the active configuration directory");
    }
    let destination = parent.join(EXPORT_DIRECTORY);
    if destination.exists() {
        anyhow::bail!(
            "{} already exists; rename it or choose another folder",
            destination.display()
        );
    }
    let temporary = parent.join(format!(".{EXPORT_DIRECTORY}-{}", std::process::id()));
    if temporary.exists() {
        anyhow::bail!("a temporary Peasy export already exists");
    }

    fs::create_dir(&temporary)?;
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o700))?;
    let result: Result<()> = (|| {
        let host_destination = temporary.join("host");
        let mut size = ExportSize::default();
        copy_configuration_directory(source_root, source_root, &host_destination, &mut size)?;
        copy_configuration_directory(
            &export.peasy_source,
            &export.peasy_source,
            &temporary.join("peasy"),
            &mut size,
        )?;
        // Both the original relative import and Peasy's configured writable path
        // refer to the same module; there is no second copy to drift on rebuild.
        let managed = host_destination.join(".peasy");
        if managed.exists() {
            fs::rename(&managed, temporary.join(".peasy"))?;
        } else {
            fs::create_dir(temporary.join(".peasy"))?;
        }
        std::os::unix::fs::symlink("../.peasy", &managed)?;
        make_host_configuration_portable(
            &host_destination.join("configuration.nix"),
            &export.peasy_module,
        )?;
        write_private_file(
            &temporary.join("configuration.nix"),
            br#"# Exported by Peasy. The complete host configuration is under ./host.
{ lib, ... }:
{
  imports = [
    ./host/configuration.nix
  ];
  services.peasy.hostConfiguration = lib.mkForce "/etc/nixos/configuration.nix";
  services.peasy.managedModule = lib.mkForce "/etc/nixos/.peasy/peasy-managed.nix";
}
"#,
        )?;
        write_private_file(
            &temporary.join("README.txt"),
            br#"PEASY NIXOS SYSTEM EXPORT

This folder contains the complete host configuration tree under host/, including
.peasy/peasy-managed.nix (also reached through host/.peasy), plus Peasy under peasy/.
INVENTORY.json lists included files and excluded paths. Common secret files and
Git metadata are excluded, but inline private values may remain in configuration
files. Review this backup before sharing. Restore any deliberately excluded files
required by your configuration from your own secure backup.

To restore on another NixOS machine:

  1. Review the files for machine-specific settings and private values.
  2. From this folder, back up and replace the destination configuration:

       sudo cp -a /etc/nixos /etc/nixos.before-peasy-restore
       sudo cp -a configuration.nix host peasy .peasy /etc/nixos/

  3. When restoring to different hardware, replace the bundled hardware module:

       nixos-generate-config --show-hardware-config | sudo tee /etc/nixos/host/hardware-configuration.nix >/dev/null

  4. Rebuild:

       sudo nixos-rebuild switch --no-flake

This is a source backup, not a locked package closure. The destination Nixpkgs
source determines package versions for a traditional non-flake rebuild.

Absolute imports outside the original configuration directory must also be
made available on the new machine or changed to portable paths before rebuilding.
"#,
        )?;
        let mut included = Vec::new();
        inventory(&temporary, &temporary, &mut included)?;
        included.sort_by_key(|entry| entry["path"].as_str().unwrap_or_default().to_owned());
        size.excluded.sort();
        write_private_file(
            &temporary.join("INVENTORY.json"),
            &serde_json::to_vec_pretty(
                &serde_json::json!({"included": included, "excluded": size.excluded,
                "privacy": "Configuration files can contain inline secrets. Review before sharing."}),
            )?,
        )?;
        fs::rename(&temporary, &destination)?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = fs::remove_dir_all(&temporary);
        return Err(error);
    }
    Ok(destination)
}

fn make_host_configuration_portable(path: &Path, peasy_module: &Path) -> Result<()> {
    let peasy_root = peasy_module
        .parent()
        .and_then(Path::parent)
        .context("Peasy module path has no source root")?;
    let peasy_root = peasy_root
        .to_str()
        .context("Peasy module source path is not UTF-8")?;
    rewrite_nix_imports(
        path.parent().context("configuration has no parent")?,
        peasy_root,
    )?;
    Ok(())
}

fn rewrite_nix_imports(directory: &Path, peasy_root: &str) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.is_dir() {
            rewrite_nix_imports(&path, peasy_root)?;
        } else if metadata.is_file() && path.extension().is_some_and(|ext| ext == "nix") {
            let contents =
                fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
            let portable = contents.replace(peasy_root, "/etc/nixos/peasy");
            fs::write(&path, portable)?;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        }
    }
    Ok(())
}

fn copy_configuration_directory(
    root: &Path,
    source: &Path,
    destination: &Path,
    size: &mut ExportSize,
) -> Result<()> {
    fs::create_dir(destination)?;
    fs::set_permissions(destination, fs::Permissions::from_mode(0o700))?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        size.entries += 1;
        if size.entries > MAX_EXPORT_ENTRIES {
            anyhow::bail!("configuration tree contains more than {MAX_EXPORT_ENTRIES} entries");
        }
        let source_path = entry.path();
        if excluded_name(&entry.file_name().to_string_lossy()) {
            size.excluded
                .push(source_path.to_string_lossy().into_owned());
            continue;
        }
        let destination_path = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path)?;
        if metadata.is_dir() {
            copy_configuration_directory(root, &source_path, &destination_path, size)?;
        } else if metadata.is_file() {
            size.bytes = size.bytes.saturating_add(metadata.len());
            if size.bytes > MAX_EXPORT_BYTES {
                anyhow::bail!("configuration tree is larger than 64 MiB");
            }
            let mut contents = Vec::new();
            OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(&source_path)?
                .take(MAX_EXPORT_BYTES + 1)
                .read_to_end(&mut contents)?;
            if contents.len() as u64 != metadata.len() {
                anyhow::bail!("{} changed while exporting; retry", source_path.display());
            }
            write_private_file(&destination_path, &contents)?;
        } else if metadata.file_type().is_symlink() {
            let target = fs::read_link(&source_path)?;
            if target.is_absolute() {
                if is_nix_build_result_link(&source_path, &target) {
                    size.excluded
                        .push(source_path.to_string_lossy().into_owned());
                    continue;
                }
                anyhow::bail!(
                    "{} is an absolute symlink and cannot be exported portably",
                    source_path.display()
                );
            }
            // Reject links that escape the copied tree, including ../ secrets.
            if !fs::canonicalize(&source_path)?.starts_with(fs::canonicalize(root)?) {
                anyhow::bail!(
                    "{} links outside the configuration tree",
                    source_path.display()
                );
            }
            std::os::unix::fs::symlink(target, destination_path)?;
        } else {
            anyhow::bail!(
                "{} is not a portable configuration file",
                source_path.display()
            );
        }
    }
    Ok(())
}

fn excluded_name(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | ".env"
            | "id_rsa"
            | "id_ed25519"
            | "openai-key"
            | "transaction.json"
            | "recovery.json"
    ) || name.starts_with(".journal-")
        || name.starts_with(".peasy-managed-")
        || name.starts_with(".env.")
        || name.ends_with(".key")
        || name.ends_with(".pem")
        || name.ends_with("~")
        || name.ends_with(".bak")
}

fn inventory(root: &Path, directory: &Path, entries: &mut Vec<serde_json::Value>) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.is_dir() {
            inventory(root, &path, entries)?;
        } else {
            entries.push(serde_json::json!({
            "path": path.strip_prefix(root)?.to_string_lossy(), "bytes": metadata.len(),
            "symlink": if metadata.file_type().is_symlink() { Some(fs::read_link(&path)?) } else { None }
        }));
        }
    }
    Ok(())
}

fn is_nix_build_result_link(path: &Path, target: &Path) -> bool {
    let name = path.file_name().and_then(|name| name.to_str());
    target.starts_with("/nix/store")
        && name.is_some_and(|name| name == "result" || name.starts_with("result-"))
}

fn write_private_file(path: &Path, contents: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(contents)?;
    file.sync_all()?;
    Ok(())
}

fn configured_source_path(pointer: &Path) -> Result<PathBuf> {
    configured_path(pointer, "/etc/nixos/configuration.nix")
}

fn configured_path(pointer: &Path, fallback: &str) -> Result<PathBuf> {
    let source = match fs::metadata(pointer) {
        Ok(metadata) => {
            if !metadata.is_file() || metadata.len() > 4096 {
                anyhow::bail!("configured export pointer is invalid");
            }
            let mut value = String::new();
            OpenOptions::new()
                .read(true)
                .open(pointer)?
                .take(4097)
                .read_to_string(&mut value)?;
            if value.len() > 4096 {
                anyhow::bail!("configured export pointer is invalid");
            }
            value.trim().to_owned()
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => fallback.to_owned(),
        Err(error) => return Err(error.into()),
    };
    if source.is_empty() || source.len() > 4096 || source.chars().any(char::is_control) {
        anyhow::bail!("configured export path is invalid");
    }
    let source = PathBuf::from(source);
    if !source.is_absolute() {
        anyhow::bail!("configured export path must be absolute");
    }
    Ok(source)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires PEASY_TEST_NIX and PEASY_TEST_NIXPKGS; package builds run this"]
    fn exported_configuration_passes_real_nixos_assertions() {
        let temp = tempfile::tempdir().unwrap();
        let host = temp.path().join("source");
        let peasy = temp.path().join("peasy-source");
        fs::create_dir_all(host.join(".peasy")).unwrap();
        fs::create_dir_all(peasy.join("nix")).unwrap();
        fs::write(
            peasy.join("nix/module.nix"),
            include_str!("../../../nix/module.nix"),
        )
        .unwrap();
        fs::write(
            host.join("configuration.nix"),
            format!(
                r#"{{ pkgs, ... }}: {{
          imports = [ "{}" ./.peasy/peasy-managed.nix ];
          services.peasy = {{ enable = true; desktop.enable = false; package = pkgs.hello; }};
          boot.loader.grub.devices = [ "nodev" ];
          fileSystems."/" = {{ device = "none"; fsType = "tmpfs"; }};
          system.stateVersion = "26.05";
        }}"#,
                peasy.join("nix/module.nix").display()
            ),
        )
        .unwrap();
        fs::write(
            host.join(".peasy/peasy-managed.nix"),
            "{ pkgs, ... }: { environment.systemPackages = [ pkgs.hello ]; }",
        )
        .unwrap();
        let export = ConfigurationExport {
            source: host.join("configuration.nix"),
            peasy_module: peasy.join("nix/module.nix"),
            peasy_source: peasy,
        };
        let destination = write_configuration_export(&export, temp.path()).unwrap();
        // Rebase the fixed restore import into the temporary test root. Keep
        // the generated hostConfiguration/managedModule settings unchanged:
        // their exact /etc/nixos layout is what the assertions below validate.
        let host_configuration = destination.join("host/configuration.nix");
        let exported = fs::read_to_string(&host_configuration).unwrap();
        assert!(exported.contains("\"/etc/nixos/peasy/nix/module.nix\""));
        fs::write(
            &host_configuration,
            exported.replace(
                "/etc/nixos/peasy",
                destination.join("peasy").to_str().unwrap(),
            ),
        )
        .unwrap();
        let nixpkgs = std::env::var("PEASY_TEST_NIXPKGS").expect("pinned Nixpkgs required");
        let expression = format!(
            r#"let host = import ({nixpkgs} + "/nixos/lib/eval-config.nix") {{
            system = "{}"; modules = [ {} ]; }};
            in assert builtins.all (a: a.assertion) host.config.assertions;
            assert host.config.services.peasy.hostConfiguration == "/etc/nixos/configuration.nix";
            assert host.config.services.peasy.managedModule == "/etc/nixos/.peasy/peasy-managed.nix";
            assert builtins.elem "hello" (map host.pkgs.lib.getName host.config.environment.systemPackages);
            true"#,
            std::env::var("PEASY_TEST_SYSTEM")
                .unwrap_or_else(|_| format!("{}-linux", std::env::consts::ARCH)),
            destination.join("configuration.nix").display()
        );
        let output =
            std::process::Command::new(std::env::var("PEASY_TEST_NIX").expect("Nix required"))
                .args([
                    "--eval",
                    "--strict",
                    "--readonly-mode",
                    "--store",
                    "dummy://",
                    "--expr",
                    &expression,
                ])
                .env("XDG_CACHE_HOME", temp.path().join("cache"))
                .output()
                .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "true");
    }

    #[test]
    fn export_rejects_escaping_links_and_preserves_internal_relative_links() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("host");
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(root.join("module.nix"), "{}").unwrap();
        std::os::unix::fs::symlink("../module.nix", root.join("nested/module.nix")).unwrap();
        let dest = temp.path().join("copy");
        copy_configuration_directory(&root, &root, &dest, &mut ExportSize::default()).unwrap();
        assert_eq!(
            fs::read_to_string(dest.join("nested/module.nix")).unwrap(),
            "{}"
        );
        fs::write(temp.path().join("secret"), "private").unwrap();
        std::os::unix::fs::symlink("../secret", root.join("escape")).unwrap();
        assert!(
            copy_configuration_directory(
                &root,
                &root,
                &temp.path().join("bad"),
                &mut ExportSize::default()
            )
            .is_err()
        );
    }

    #[test]
    fn configuration_export_contains_host_tree_and_peasy_managed_module() {
        let temp = tempfile::tempdir().unwrap();
        let host = temp.path().join("source");
        fs::create_dir(&host).unwrap();
        let source = host.join("configuration.nix");
        let pointer = temp.path().join("host-configuration-path");
        let original_peasy_root = temp.path().join("original-peasy");
        let original_module = original_peasy_root.join("nix/module.nix");
        let contents = format!(
            "{{ pkgs, ... }}: {{ imports = [ {} ./.peasy/peasy-managed.nix ]; environment.systemPackages = [ pkgs.vlc ]; }}\n",
            original_module.display()
        );
        fs::write(&source, &contents).unwrap();
        fs::write(host.join(".env"), "SECRET=synthetic-test-value").unwrap();
        fs::create_dir(host.join(".git")).unwrap();
        fs::write(host.join(".git/config"), "private metadata").unwrap();
        fs::write(host.join("hardware-configuration.nix"), b"{ ... }: {}\n").unwrap();
        std::os::unix::fs::symlink(
            "/nix/store/00000000000000000000000000000000-build-result",
            host.join("result"),
        )
        .unwrap();
        fs::create_dir(host.join(".peasy")).unwrap();
        fs::write(
            host.join(".peasy/peasy-managed.nix"),
            b"{ pkgs, ... }: { environment.systemPackages = [ pkgs.firefox ]; }\n",
        )
        .unwrap();
        fs::write(&pointer, format!("{}\n", source.display())).unwrap();
        let peasy_source = temp.path().join("peasy-source");
        fs::create_dir_all(peasy_source.join("nix")).unwrap();
        fs::write(peasy_source.join("nix/module.nix"), b"{ ... }: {}\n").unwrap();
        let module_pointer = temp.path().join("module-import-path");
        fs::write(
            &module_pointer,
            original_module.to_string_lossy().as_bytes(),
        )
        .unwrap();

        let export = configuration_export_from(&pointer, &module_pointer, &peasy_source).unwrap();
        let selected = temp.path().join("selected");
        fs::create_dir(&selected).unwrap();
        let destination = write_configuration_export(&export, &selected).unwrap();

        assert_eq!(
            fs::read_to_string(destination.join("host/configuration.nix")).unwrap(),
            contents.replace(original_peasy_root.to_str().unwrap(), "/etc/nixos/peasy")
        );
        assert!(
            destination
                .join("host/hardware-configuration.nix")
                .is_file()
        );
        assert!(
            fs::read_to_string(destination.join("host/.peasy/peasy-managed.nix"))
                .unwrap()
                .contains("pkgs.firefox")
        );
        assert!(
            fs::read_to_string(destination.join("configuration.nix"))
                .unwrap()
                .contains("./host/configuration.nix")
        );
        assert!(destination.join("README.txt").is_file());
        assert!(destination.join("peasy/nix/module.nix").is_file());
        assert!(!destination.join("host/result").exists());
        assert!(!destination.join("host/.env").exists());
        assert!(!destination.join("host/.git").exists());
        assert!(destination.join(".peasy/peasy-managed.nix").is_file());
        let wrapper = fs::read_to_string(destination.join("configuration.nix")).unwrap();
        assert!(wrapper.contains("/etc/nixos/.peasy/peasy-managed.nix"));
        let inventory = fs::read_to_string(destination.join("INVENTORY.json")).unwrap();
        assert!(inventory.contains(".env"));
        assert!(inventory.contains(".git"));
        assert!(!inventory.contains("synthetic-test-value"));
    }

    #[test]
    fn configuration_export_rejects_relative_and_oversized_sources() {
        let temp = tempfile::tempdir().unwrap();
        let pointer = temp.path().join("host-configuration-path");
        fs::write(&pointer, "../configuration.nix\n").unwrap();
        assert!(
            configuration_export_from(
                &pointer,
                Path::new("/missing-module-pointer"),
                Path::new("/missing-peasy-source"),
            )
            .is_err()
        );

        let source = temp.path().join("configuration.nix");
        fs::write(&source, vec![b'x'; MAX_CONFIGURATION_BYTES as usize + 1]).unwrap();
        fs::write(&pointer, source.to_string_lossy().as_bytes()).unwrap();
        assert!(
            configuration_export_from(
                &pointer,
                Path::new("/missing-module-pointer"),
                Path::new("/missing-peasy-source"),
            )
            .is_err()
        );
    }
}

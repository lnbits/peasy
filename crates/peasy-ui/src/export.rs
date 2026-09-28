use anyhow::{Context, Result};
use peasy_core::{PackageState, render_packages_module};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

const MAX_CONFIGURATION_BYTES: u64 = 4 * 1024 * 1024;
const MAX_EXPORT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_EXPORT_ENTRIES: usize = 4096;
const EXPORT_DIRECTORY: &str = "peasy-system-config";

#[derive(Clone, Debug)]
pub(super) struct ConfigurationExport {
    source: PathBuf,
    peasy_source: PathBuf,
    state: PackageState,
}

#[derive(Default)]
struct ExportSize {
    bytes: u64,
    entries: usize,
    excluded: Vec<String>,
}

pub(super) fn configuration_export(_socket: &Path) -> Result<ConfigurationExport> {
    // Resolve once so state and bundled code belong to the same generation,
    // even if an activation completes while the folder picker is open.
    let system = fs::canonicalize("/run/current-system")?;
    configuration_export_from(
        &system.join("etc/peasy/host-configuration-path"),
        &system.join("etc/peasy/state.json"),
        &system.join("sw/share/peasy/source"),
    )
}

fn configuration_export_from(
    pointer: &Path,
    state_path: &Path,
    peasy_source: &Path,
) -> Result<ConfigurationExport> {
    let source = configured_source_path(pointer)?;
    if !peasy_source.is_dir() {
        anyhow::bail!("installed Peasy source is unavailable for the backup");
    }
    let mut state: PackageState = match fs::File::open(state_path) {
        Ok(file) => {
            let mut bytes = Vec::new();
            file.take(MAX_CONFIGURATION_BYTES + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() as u64 > MAX_CONFIGURATION_BYTES {
                anyhow::bail!("Peasy state is larger than 4 MiB");
            }
            serde_json::from_slice(&bytes).context("reading active Peasy state")?
        }
        // A new installation has no managed generation yet.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => PackageState::default(),
        Err(error) => return Err(error.into()),
    };
    state.normalize()?;
    Ok(ConfigurationExport {
        source,
        peasy_source: peasy_source.to_owned(),
        state,
    })
}

fn portable_state(state: &PackageState) -> PackageState {
    // Only this closed subset is imported on the destination. Network
    // interfaces, binary architectures and service/account bindings need a new
    // proposal there; copying arbitrary host modules cannot preserve hardware.
    peasy_core::PortableBackup::from_state(state).state()
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
        .context("configured source has no parent")?;
    let parent_real = fs::canonicalize(parent)?;
    if parent_real.starts_with(fs::canonicalize(source_root).unwrap_or(source_root.to_owned()))
        || parent_real.starts_with(fs::canonicalize(&export.peasy_source)?)
    {
        anyhow::bail!(
            "choose a destination outside the configuration and Peasy source directories"
        );
    }
    let destination = parent.join(EXPORT_DIRECTORY);
    if destination.symlink_metadata().is_ok() {
        anyhow::bail!(
            "{} already exists; rename it or choose another folder",
            destination.display()
        );
    }
    let temporary = parent.join(format!(".{EXPORT_DIRECTORY}-{}", std::process::id()));
    fs::create_dir(&temporary).context("creating private temporary backup directory")?;
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o700))?;
    let result: Result<()> = (|| {
        let mut size = ExportSize::default();
        copy_configuration_directory(
            &export.peasy_source,
            &export.peasy_source,
            &temporary.join("peasy"),
            &mut size,
        )?;
        let mut archive_size = ExportSize {
            bytes: size.bytes,
            entries: size.entries,
            ..ExportSize::default()
        };
        // Host sources are reference material only. Protected or unavailable
        // sources must not prevent backing up the active typed Peasy state.
        let archive_error = match copy_configuration_directory(
            source_root,
            source_root,
            &temporary.join("host-reference"),
            &mut archive_size,
        ) {
            Ok(()) => {
                size.excluded.extend(archive_size.excluded);
                None
            }
            Err(error) => {
                fs::remove_dir_all(temporary.join("host-reference"))?;
                Some(format!("Host source archive was not included: {error:#}"))
            }
        };
        let portable = portable_state(&export.state);
        write_private_file(
            &temporary.join("peasy-managed.nix"),
            render_packages_module(&portable)?.as_bytes(),
        )?;
        // Keep the complete active state as data, never as an automatically
        // imported Nix module. No original hardware settings are activated.
        write_private_file(
            &temporary.join("RESTORE-REVIEW.json"),
            &serde_json::to_vec_pretty(&serde_json::json!({
                "format": 1,
                "source_configuration": export.source,
                "active_state": export.state,
                "requires_review": {
                    "networks": "Rediscover destination interfaces and recreate profiles in Peasy.",
                    "appimages": "Select and review a release for the destination CPU architecture.",
                    "setups": "Recreate service and account setup through Peasy on the destination. Database contents need a separate backup.",
                    "host_reference": "Reference only. Do not replace the destination host or import its old hardware, bootloader or disk settings."
                },
                "host_archive_error": archive_error,
            }))?,
        )?;
        let summary = format!(
            "\nBACKUP CONTENTS\n\nPortable: {} standalone packages, {} pea packages, and appearance preferences.\nRequires destination review: {} service setups, {} network profiles, {} AppImages.\n{}\n",
            portable.packages.len(), portable.peas.len(), export.state.setups.len(),
            export.state.networks.len(), export.state.appimages.len(),
            archive_error.as_deref().unwrap_or("Original host source is under host-reference/ for reference only, including any flake.nix and flake.lock.")
        );
        write_private_file(
            &temporary.join("README.txt"),
            format!("{}{}", include_str!("export-restore.txt"), summary).as_bytes(),
        )?;
        let mut included = Vec::new();
        inventory(&temporary, &temporary, &mut included)?;
        included.sort_by_key(|entry| entry["path"].as_str().unwrap_or_default().to_owned());
        size.excluded.sort();
        write_private_file(
            &temporary.join("INVENTORY.json"),
            &serde_json::to_vec_pretty(&serde_json::json!({
                "format": 2, "included": included, "excluded": size.excluded,
                "host_archive_error": archive_error,
                "privacy": "Configuration files can contain inline secrets. Review before sharing."
            }))?,
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
    use peasy_core::{
        AccentColor, ColorScheme, ManagedSetup, NetworkProfile, SystemSetup, ThemeSettings,
        parse_packages_module,
    };

    fn example_state() -> PackageState {
        let mut state = PackageState {
            peasy_release: None,
            resources: peasy_core::ResourceState::default(),
            packages: vec!["hello".into()],
            peas: vec![peasy_core::pea::PeaPin {
                id: "appearance".into(),
                version: "1.1.0".into(),
                revision: "a".repeat(40),
                hash: "b".repeat(64),
                host_api: peasy_core::pea::HOST_API,
                permissions: vec!["appearance".into()],
            }],
            theme: ThemeSettings {
                accent_color: Some(AccentColor::Green),
                color_scheme: Some(ColorScheme::Dark),
            },
            setups: vec![ManagedSetup {
                package: "virt-manager".into(),
                settings: SystemSetup {
                    packages: vec![],
                    enable: vec!["virtualisation.libvirtd.enable".into()],
                    groups: vec!["libvirtd".into()],
                    postgresql: None,
                },
                user: Some("old_user".into()),
                uid: Some(1234),
            }],
            networks: vec![serde_json::from_value::<NetworkProfile>(serde_json::json!({
                "id": "old-network", "interface": "enp99s0", "kind": "ethernet",
                "wifi_mode": null, "ssid": null, "ipv4": "auto", "addresses": [],
                "gateway": null, "dns": [], "autoconnect": true
            })).unwrap()],
            appimages: vec![serde_json::from_value(serde_json::json!({
                "id": "appimage.example.editor", "display_name": "Editor", "repository": "example/editor",
                "version": "1.0", "release_tag": "v1.0", "asset_name": "editor.AppImage",
                "url": "https://github.com/example/editor/releases/download/v1.0/editor.AppImage",
                "hash": "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
                "architecture": "x86_64", "size": 1024
            })).unwrap()],
        };
        state.normalize().unwrap();
        state
    }

    fn fixture(root: &Path, flake: bool) -> ConfigurationExport {
        let host = root.join("source");
        let peasy = root.join("peasy-source");
        fs::create_dir_all(host.join(".peasy")).unwrap();
        fs::create_dir_all(peasy.join("nix")).unwrap();
        fs::write(
            peasy.join("nix/module.nix"),
            include_str!("../../../nix/module.nix"),
        )
        .unwrap();
        let source = host.join(if flake {
            "flake.nix"
        } else {
            "configuration.nix"
        });
        // Arbitrary original modules must never enter the destination's graph.
        fs::write(
            &source,
            "throw \"the original host must not be imported\"\n",
        )
        .unwrap();
        fs::write(
            host.join("hardware-configuration.nix"),
            r#"{ ... }: {
          fileSystems."/".device = "/dev/disk/by-uuid/OLD-DISK";
          boot.loader.grub.devices = [ "/dev/old-disk" ];
          boot.initrd.availableKernelModules = [ "old-driver" ];
          nixpkgs.hostPlatform = "x86_64-linux";
        }"#,
        )
        .unwrap();
        if flake {
            fs::write(host.join("flake.lock"), "{\"original\":true}\n").unwrap();
        }
        fs::write(host.join(".env"), "SECRET=synthetic-test-value").unwrap();
        let state = example_state();
        fs::write(
            host.join(".peasy/peasy-managed.nix"),
            render_packages_module(&state).unwrap(),
        )
        .unwrap();
        let state_path = root.join("state.json");
        fs::write(&state_path, serde_json::to_vec(&state).unwrap()).unwrap();
        let pointer = root.join("host-configuration-path");
        fs::write(&pointer, source.to_str().unwrap()).unwrap();
        configuration_export_from(&pointer, &state_path, &peasy).unwrap()
    }

    #[test]
    fn portable_backup_preserves_state_but_never_imports_original_hardware() {
        for flake in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let export = fixture(temp.path(), flake);
            let original = fs::read(&export.source).unwrap();
            let destination = write_configuration_export(&export, temp.path()).unwrap();
            let module = fs::read_to_string(destination.join("peasy-managed.nix")).unwrap();
            assert_eq!(
                parse_packages_module(&module).unwrap(),
                portable_state(&export.state)
            );
            for forbidden in [
                "OLD-DISK",
                "old-driver",
                "old_user",
                "enp99s0",
                "AppImagePackage",
                "example/editor",
                "host-reference",
                "boot.loader",
            ] {
                assert!(!module.contains(forbidden), "{forbidden}");
            }
            assert!(!destination.join("configuration.nix").exists());
            assert!(!destination.join("flake.nix").exists());
            let review: serde_json::Value =
                serde_json::from_slice(&fs::read(destination.join("RESTORE-REVIEW.json")).unwrap())
                    .unwrap();
            assert_eq!(
                review["active_state"],
                serde_json::to_value(&export.state).unwrap()
            );
            assert!(review["host_archive_error"].is_null());
            assert!(
                destination
                    .join("host-reference/hardware-configuration.nix")
                    .is_file()
            );
            if flake {
                assert_eq!(
                    fs::read(destination.join("host-reference/flake.lock")).unwrap(),
                    fs::read(export.source.parent().unwrap().join("flake.lock")).unwrap()
                );
            }
            assert!(!destination.join("host-reference/.env").exists());
            let inventory = fs::read_to_string(destination.join("INVENTORY.json")).unwrap();
            assert!(inventory.contains(".env"));
            assert!(!inventory.contains("synthetic-test-value"));
            assert_eq!(fs::read(&export.source).unwrap(), original);
            assert_eq!(
                fs::metadata(&destination).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(destination.join("peasy-managed.nix"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            let restore = fs::read_to_string(destination.join("README.txt")).unwrap();
            assert!(restore.contains(
                "Requires destination review: 1 service setups, 1 network profiles, 1 AppImages."
            ));
            assert!(restore.contains("Restore backup"));
            assert!(!restore.contains("nixos-generate-config"));
        }
    }

    #[test]
    fn unavailable_host_archive_is_reported_without_losing_active_state() {
        let temp = tempfile::tempdir().unwrap();
        let export = fixture(temp.path(), true);
        fs::remove_dir_all(export.source.parent().unwrap()).unwrap();
        let destination = write_configuration_export(&export, temp.path()).unwrap();
        assert!(!destination.join("host-reference").exists());
        assert!(destination.join("peasy-managed.nix").is_file());
        let readme = fs::read_to_string(destination.join("README.txt")).unwrap();
        assert!(readme.contains("Host source archive was not included:"));
        let inventory: serde_json::Value =
            serde_json::from_slice(&fs::read(destination.join("INVENTORY.json")).unwrap()).unwrap();
        assert!(inventory["host_archive_error"].is_string());
    }

    #[test]
    fn invalid_active_state_is_rejected_and_new_installations_export_empty_state() {
        let temp = tempfile::tempdir().unwrap();
        let export = fixture(temp.path(), false);
        let pointer = temp.path().join("host-configuration-path");
        let state = temp.path().join("state.json");
        for bytes in [
            b"not JSON".to_vec(),
            b"{\"packages\":[],\"unknown\":true}".to_vec(),
            vec![b'x'; MAX_CONFIGURATION_BYTES as usize + 1],
        ] {
            fs::write(&state, bytes).unwrap();
            assert!(configuration_export_from(&pointer, &state, &export.peasy_source).is_err());
        }
        fs::remove_file(&state).unwrap();
        let empty = configuration_export_from(&pointer, &state, &export.peasy_source).unwrap();
        assert_eq!(empty.state, PackageState::default());
        fs::write(&pointer, "../configuration.nix").unwrap();
        assert!(configuration_export_from(&pointer, &state, &export.peasy_source).is_err());
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
    fn backup_never_overwrites_existing_files_or_recurses_into_its_sources() {
        let temp = tempfile::tempdir().unwrap();
        let export = fixture(temp.path(), false);
        assert!(write_configuration_export(&export, export.source.parent().unwrap()).is_err());
        assert!(write_configuration_export(&export, &export.peasy_source).is_err());
        let destination = write_configuration_export(&export, temp.path()).unwrap();
        let original = fs::read(destination.join("peasy-managed.nix")).unwrap();
        assert!(write_configuration_export(&export, temp.path()).is_err());
        assert_eq!(
            fs::read(destination.join("peasy-managed.nix")).unwrap(),
            original
        );
    }

    #[test]
    #[ignore = "requires PEASY_TEST_NIX and PEASY_TEST_NIXPKGS; checks.export runs this"]
    fn exported_configuration_passes_real_nixos_assertions() {
        let temp = tempfile::tempdir().unwrap();
        let export = fixture(temp.path(), true);
        let destination = write_configuration_export(&export, temp.path()).unwrap();
        let nixpkgs = std::env::var("PEASY_TEST_NIXPKGS").expect("pinned Nixpkgs required");
        // Evaluate the actual output against unrelated destination hardware on
        // both architectures. No source host module can be imported: it throws.
        for system in ["x86_64-linux", "aarch64-linux"] {
            let expression = format!(
                r#"
              let host = import ({nixpkgs} + "/nixos/lib/eval-config.nix") {{
                system = "{system}";
                modules = [ {peasy}/nix/module.nix {destination}/peasy-managed.nix
                  ({{ pkgs, ... }}: {{
                    services.peasy = {{ enable = true; desktop.enable = false; package = pkgs.hello; }};
                    fileSystems."/" = {{ device = "/dev/disk/by-uuid/DESTINATION-DISK"; fsType = "ext4"; }};
                    boot.loader.systemd-boot.enable = true;
                    boot.loader.grub.enable = false;
                    boot.initrd.availableKernelModules = [ "virtio_pci" ];
                    services.xserver.videoDrivers = [ "modesetting" ];
                    networking.hostName = "destination";
                    users.users.destination = {{ isNormalUser = true; uid = 1000; }};
                    system.stateVersion = "26.05";
                  }})
                ];
              }};
              in assert builtins.all (a: a.assertion) host.config.assertions;
              assert host.pkgs.stdenv.hostPlatform.system == "{system}";
              assert host.config.fileSystems."/".device == "/dev/disk/by-uuid/DESTINATION-DISK";
              assert host.config.boot.loader.systemd-boot.enable && !host.config.boot.loader.grub.enable;
              assert builtins.elem "virtio_pci" host.config.boot.initrd.availableKernelModules;
              assert !(builtins.elem "old-driver" host.config.boot.initrd.availableKernelModules);
              assert host.config.services.xserver.videoDrivers == [ "modesetting" ];
              assert host.config.networking.hostName == "destination";
              assert !(host.config.users.users ? old_user);
              assert !host.config.virtualisation.libvirtd.enable;
              assert host.config.networking.networkmanager.ensureProfiles.profiles == {{}};
              assert host.config.environment.etc."peasy/managed-module-path".text == "/etc/nixos/.peasy/peasy-managed.nix\n";
              assert builtins.elem "hello" (map host.pkgs.lib.getName host.config.environment.systemPackages);
              assert (builtins.fromJSON host.config.environment.etc."peasy/theme.json".text).accent_color == "green";
              true
            "#,
                peasy = destination.join("peasy").display(),
                destination = destination.display()
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
                "{system}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "true");
        }
    }
}

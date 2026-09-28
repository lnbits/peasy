use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub const REQUEST_FILE: &str = "activation-request.json";
pub const RESULT_FILE: &str = "activation-result.json";
const GUARD_FILE: &str = "activation-guard.json";
const ACTIVATION_DIRECTORY: &str = "activation";

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActivationRequest {
    pub system: PathBuf,
    pub guard: ActivationGuard,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActivationGuard {
    pub active_system: PathBuf,
    pub generation: Option<PathBuf>,
    pub managed_module: PathBuf,
    pub state_hash: String,
}

impl ActivationGuard {
    pub fn new(
        active_system: &Path,
        managed_module: &Path,
        expected: &peasy_core::PackageState,
    ) -> Result<Self> {
        Ok(Self {
            active_system: active_system.into(),
            generation: crate::recovery::generation(active_system),
            managed_module: managed_module.into(),
            state_hash: hash_state(expected)?,
        })
    }

    pub fn check(&self) -> Result<()> {
        if crate::recovery::generation(&self.active_system) != self.generation {
            bail!(
                "The active system changed during this operation; review again before activating"
            );
        }
        if state_hash(&self.managed_module)? != self.state_hash {
            bail!(
                "The managed configuration changed outside this operation; review again before activating"
            );
        }
        Ok(())
    }
}

fn state_hash(path: &Path) -> Result<String> {
    hash_state(&crate::state::load_managed(path)?)
}

fn hash_state(state: &peasy_core::PackageState) -> Result<String> {
    use sha2::{Digest, Sha256};
    Ok(hex::encode(Sha256::digest(
        peasy_core::render_packages_module(state)?.as_bytes(),
    )))
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActivationResult {
    pub activated: bool,
    pub message: String,
}

pub fn write_request(runtime_dir: &Path, system: &Path, guard: ActivationGuard) -> Result<()> {
    let directory = runtime_dir.join(ACTIVATION_DIRECTORY);
    // A service-start failure must not leave the caller reading the result of
    // an earlier activation attempt.
    let _ = fs::remove_file(directory.join(RESULT_FILE));
    write_private_json(
        &directory.join(REQUEST_FILE),
        &ActivationRequest {
            system: system.to_owned(),
            guard,
        },
    )
}

pub fn read_result(runtime_dir: &Path) -> Result<ActivationResult> {
    let bytes = fs::read(runtime_dir.join(ACTIVATION_DIRECTORY).join(RESULT_FILE))
        .context("reading activation result")?;
    serde_json::from_slice(&bytes).context("parsing activation result")
}

pub fn run_helper(runtime_dir: &Path, nix_env: &Path) -> Result<()> {
    let result_path = runtime_dir.join(ACTIVATION_DIRECTORY).join(RESULT_FILE);
    let _ = fs::remove_file(&result_path);
    let result = activate(runtime_dir, nix_env);
    let _ = fs::remove_file(runtime_dir.join(ACTIVATION_DIRECTORY).join(GUARD_FILE));
    let record = match &result {
        Ok(()) => ActivationResult {
            activated: true,
            message: "NixOS generation activated".into(),
        },
        Err(error) => ActivationResult {
            activated: false,
            message: format!("{error:#}"),
        },
    };
    write_private_json(&result_path, &record)?;
    result
}

fn activate(runtime_dir: &Path, nix_env: &Path) -> Result<()> {
    if !nix_env.is_absolute() {
        bail!("trusted nix-env path must be absolute");
    }
    let request_path = runtime_dir.join(ACTIVATION_DIRECTORY).join(REQUEST_FILE);
    let request = read_request(&request_path)?;
    fs::remove_file(&request_path).context("consuming activation request")?;
    let system = fs::canonicalize(&request.system).context("resolving proposed system")?;
    let switch = system.join("bin/switch-to-configuration");
    if !system.starts_with("/nix/store/") || !switch.is_file() {
        bail!("activation request is not a valid NixOS store result");
    }
    request.guard.check()?;
    write_private_json(
        &runtime_dir.join(ACTIVATION_DIRECTORY).join(GUARD_FILE),
        &request,
    )?;

    let profile = trusted_command(nix_env)
        .args(["--profile", "/nix/var/nix/profiles/system", "--set"])
        .arg(&system)
        .output()
        .context("installing NixOS system generation")?;
    if !profile.status.success() {
        bail!(
            "could not install system generation: {}",
            command_failure(&profile)
        );
    }
    request.guard.check()?;
    let activation = trusted_command(&switch)
        .env("PEASY_ACTIVATION_GUARD", "1")
        .arg("switch")
        .output()
        .context("activating NixOS system generation")?;
    if !activation.status.success() {
        bail!("system activation failed: {}", command_failure(&activation));
    }
    Ok(())
}

// NixOS invokes this under its switch-to-configuration lock, closing the
// race between the helper's check and a competing administrator activation.
pub fn check_guard(runtime_dir: &Path) -> Result<()> {
    read_request(&runtime_dir.join(ACTIVATION_DIRECTORY).join(GUARD_FILE))?
        .guard
        .check()
}

fn read_request(request_path: &Path) -> Result<ActivationRequest> {
    let metadata = fs::symlink_metadata(request_path).context("inspecting activation request")?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.uid() != 0
        || metadata.mode() & 0o077 != 0
        || metadata.len() > 8192
    {
        bail!("activation request is not a private root-owned regular file");
    }
    let bytes = fs::read(request_path)?;
    serde_json::from_slice(&bytes).context("parsing activation request")
}

fn trusted_command(program: &Path) -> Command {
    let mut command = Command::new(program);
    command.env_clear();
    command.env("PATH", "/run/current-system/sw/bin");
    command.env("HOME", "/var/empty");
    command
}

pub(crate) fn write_private_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path.parent().context("activation path has no parent")?;
    fs::create_dir_all(parent)?;
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    let temporary = parent.join(format!(".activation-{}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    serde_json::to_writer(&mut file, value)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    fs::rename(temporary, path)?;
    Ok(())
}

fn command_failure(output: &Output) -> String {
    let stderr = output_tail(&output.stderr);
    let details = if stderr.trim().is_empty() {
        output_tail(&output.stdout)
    } else {
        stderr
    };
    if details.trim().is_empty() {
        output.status.to_string()
    } else {
        format!("{}\n{details}", output.status)
    }
}

fn output_tail(bytes: &[u8]) -> String {
    // Activation prints progress first and the failed units/cause last. Keep
    // the end, not the preamble, while bounding the error returned over IPC.
    let text = String::from_utf8_lossy(bytes);
    let mut lines: Vec<_> = text.lines().rev().take(32).collect();
    lines.reverse();
    let tail = lines.join("\n");
    let mut chars: Vec<_> = tail.chars().rev().take(4096).collect();
    chars.reverse();
    chars.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;

    #[test]
    fn activation_guard_rejects_source_and_generation_changes() {
        let temp = tempfile::tempdir().unwrap();
        let managed = temp.path().join("managed.nix");
        let before = peasy_core::PackageState::default();
        crate::state::write_managed_atomic(&managed, &before).unwrap();
        let mut guard =
            ActivationGuard::new(&temp.path().join("active"), &managed, &before).unwrap();
        guard.check().unwrap();
        let changed = before
            .with_change(peasy_core::PackageOperation::Install, "hello")
            .unwrap();
        crate::state::write_managed_atomic(&managed, &changed).unwrap();
        assert!(
            guard
                .check()
                .unwrap_err()
                .to_string()
                .contains("configuration changed")
        );
        crate::state::write_managed_atomic(&managed, &before).unwrap();
        guard.generation = Some("/nix/store/previous-generation".into());
        assert!(
            guard
                .check()
                .unwrap_err()
                .to_string()
                .contains("active system changed")
        );
    }

    #[test]
    fn activation_error_keeps_final_cause_and_exit_status() {
        let output = Output {
            status: std::process::ExitStatus::from_raw(4 << 8),
            stdout: Vec::new(),
            stderr: format!("{}failed unit: example.service\n", "progress\n".repeat(100))
                .into_bytes(),
        };
        let message = command_failure(&output);
        assert!(message.starts_with("exit status: 4\n"));
        assert!(message.ends_with("failed unit: example.service"));
        assert_eq!(message.lines().count(), 33);
    }

    #[test]
    fn activation_error_tail_is_bounded_and_unicode_safe() {
        let text = format!("{}final cause", "é".repeat(10_000));
        let tail = output_tail(text.as_bytes());
        assert_eq!(tail.chars().count(), 4096);
        assert!(tail.ends_with("final cause"));
        assert!(!tail.contains('\u{fffd}'));
        assert!(output_tail(b"invalid: \xff").contains('\u{fffd}'));
    }

    #[test]
    fn activation_error_reports_signals_and_stdout_fallback() {
        let mut output = Output {
            status: std::process::ExitStatus::from_raw(9),
            stdout: b"stdout-only cause".to_vec(),
            stderr: b"\n".to_vec(),
        };
        let message = command_failure(&output);
        assert!(message.contains("signal"));
        assert!(message.contains('9'));
        assert!(message.ends_with("stdout-only cause"));
        output.stdout.clear();
        assert_eq!(command_failure(&output), output.status.to_string());
    }

    #[test]
    fn a_new_request_removes_any_stale_activation_result() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = temporary.path().join(ACTIVATION_DIRECTORY);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join(RESULT_FILE), b"stale result").unwrap();

        let system = Path::new("/nix/store/example-system");
        let managed = temporary.path().join("managed.nix");
        crate::state::write_managed_atomic(&managed, &Default::default()).unwrap();
        let guard = ActivationGuard::new(temporary.path(), &managed, &Default::default()).unwrap();
        write_request(temporary.path(), system, guard).unwrap();

        assert!(!directory.join(RESULT_FILE).exists());
        let request: ActivationRequest =
            serde_json::from_slice(&fs::read(directory.join(REQUEST_FILE)).unwrap()).unwrap();
        assert_eq!(request.system, system);
    }
}

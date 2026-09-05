use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub const REQUEST_FILE: &str = "activation-request.json";
pub const RESULT_FILE: &str = "activation-result.json";
const ACTIVATION_DIRECTORY: &str = "activation";

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActivationRequest {
    pub system: PathBuf,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActivationResult {
    pub activated: bool,
    pub message: String,
}

pub fn write_request(runtime_dir: &Path, system: &Path) -> Result<()> {
    let directory = runtime_dir.join(ACTIVATION_DIRECTORY);
    // A service-start failure must not leave the caller reading the result of
    // an earlier activation attempt.
    let _ = fs::remove_file(directory.join(RESULT_FILE));
    write_private_json(
        &directory.join(REQUEST_FILE),
        &ActivationRequest {
            system: system.to_owned(),
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
    let metadata = fs::symlink_metadata(&request_path).context("inspecting activation request")?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.uid() != 0
        || metadata.mode() & 0o077 != 0
        || metadata.len() > 8192
    {
        bail!("activation request is not a private root-owned regular file");
    }
    let bytes = fs::read(&request_path)?;
    fs::remove_file(&request_path).context("consuming activation request")?;
    let request: ActivationRequest =
        serde_json::from_slice(&bytes).context("parsing activation request")?;
    let system = fs::canonicalize(&request.system).context("resolving proposed system")?;
    let switch = system.join("bin/switch-to-configuration");
    if !system.starts_with("/nix/store/") || !switch.is_file() {
        bail!("activation request is not a valid NixOS store result");
    }

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
    let activation = trusted_command(&switch)
        .arg("switch")
        .output()
        .context("activating NixOS system generation")?;
    if !activation.status.success() {
        bail!("system activation failed: {}", command_failure(&activation));
    }
    Ok(())
}

fn trusted_command(program: &Path) -> Command {
    let mut command = Command::new(program);
    command.env_clear();
    command.env("PATH", "/run/current-system/sw/bin");
    command.env("HOME", "/var/empty");
    command
}

fn write_private_json(path: &Path, value: &impl Serialize) -> Result<()> {
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
        write_request(temporary.path(), system).unwrap();

        assert!(!directory.join(RESULT_FILE).exists());
        let request: ActivationRequest =
            serde_json::from_slice(&fs::read(directory.join(REQUEST_FILE)).unwrap()).unwrap();
        assert_eq!(request.system, system);
    }
}

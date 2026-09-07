#[path = "../../../peas/appearance/system.rs"]
mod appearance;
#[path = "../../../peas/appimages/system.rs"]
mod appimages;
#[path = "../../../peas/packages/system.rs"]
mod packages;
#[path = "../../../peas/system_configuration/system.rs"]
mod system_configuration;
use packages::CachedSearch;

use crate::{activation, state};
use anyhow::{Context, Result, bail};
use peasy_core::{
    ApplyResult, DiffLine, PackageOperation, PackageState, ProposalChange, ThemeSettings,
    render_packages_module, render_system_expression, validate_attribute,
};
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Debug)]
pub enum RebuildTarget {
    Configuration {
        path: PathBuf,
    },
    Flake {
        reference: String,
        nixos_rebuild: PathBuf,
    },
}

#[derive(Clone, Debug)]
pub struct BackendConfig {
    pub appimage_policy: PathBuf,
    pub runtime_dir: PathBuf,
    pub nix: PathBuf,
    pub systemctl: PathBuf,
    pub nixpkgs: PathBuf,
    pub system: String,
    pub managed_module: PathBuf,
    pub rebuild_target: RebuildTarget,
}

pub trait CommandRunner: Send + Sync {
    fn run(&self, program: &Path, args: &[OsString], cwd: Option<&Path>) -> Result<Output>;
}

pub struct ProcessRunner;

impl CommandRunner for ProcessRunner {
    fn run(&self, program: &Path, args: &[OsString], cwd: Option<&Path>) -> Result<Output> {
        if !program.is_absolute() {
            bail!("trusted executable path must be absolute");
        }
        let mut command = Command::new(program);
        command.args(args);
        command.env_clear();
        command.env("PATH", "/run/current-system/sw/bin");
        command.env("HOME", "/var/empty");
        command.env("XDG_CACHE_HOME", "/run/peasy/nix-cache");
        command.env(
            "NIX_CONFIG",
            "extra-experimental-features = nix-command flakes",
        );
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        crate::process::run(command, Duration::from_secs(60 * 60))
            .with_context(|| format!("running trusted executable {}", program.display()))
    }
}

pub struct NixBackend {
    config: BackendConfig,
    runner: Arc<dyn CommandRunner>,
    search_cache: Mutex<HashMap<String, CachedSearch>>,
    verified_packages: Mutex<HashMap<String, String>>,
    apply_lock: Mutex<()>,
    evaluation_lock: Mutex<()>,
}

#[derive(Clone)]

pub struct Preview {
    pub before: PackageState,
    pub change: ProposalChange,
    pub title: String,
    pub diff: Vec<DiffLine>,
}

impl NixBackend {
    pub fn new(config: BackendConfig, runner: Arc<dyn CommandRunner>) -> Result<Self> {
        if !config.nixpkgs.starts_with("/nix/store") {
            bail!("Nixpkgs must reside in the Nix store");
        }
        match &config.rebuild_target {
            RebuildTarget::Configuration { path } if !path.is_absolute() => {
                bail!("host configuration must be an absolute path");
            }
            RebuildTarget::Flake { reference, .. } => {
                let directory = reference.split('#').next().unwrap_or_default();
                if !directory.starts_with('/') {
                    bail!("host flake must use an absolute local path");
                }
            }
            RebuildTarget::Configuration { .. } => {}
        }
        if !config.managed_module.is_absolute() {
            bail!("Peasy managed module must use an absolute path");
        }
        fs::create_dir_all(&config.runtime_dir)?;
        let transactions = config.runtime_dir.join("transactions");
        fs::create_dir_all(&transactions)?;
        fs::set_permissions(&transactions, fs::Permissions::from_mode(0o700))?;
        if !config.managed_module.exists() {
            state::write_managed_atomic(&config.managed_module, &PackageState::default())?;
        }
        state::load_managed(&config.managed_module)?;
        Ok(Self {
            config,
            runner,
            search_cache: Mutex::new(HashMap::new()),
            verified_packages: Mutex::new(HashMap::new()),
            apply_lock: Mutex::new(()),
            evaluation_lock: Mutex::new(()),
        })
    }

    fn current_state(&self) -> Result<PackageState> {
        state::load_managed(&self.config.managed_module)
    }

    pub fn packages(&self) -> Result<Vec<String>> {
        let state = self.current_state()?;
        Ok(state
            .packages
            .iter()
            .cloned()
            .chain(state.appimages.iter().map(|package| package.id.clone()))
            .chain(state.setups.iter().map(|setup| setup.package.clone()))
            .collect())
    }

    pub fn theme(&self) -> Result<ThemeSettings> {
        Ok(self.current_state()?.theme)
    }

    pub fn managed_module(&self) -> Result<String> {
        render_packages_module(&self.current_state()?).map_err(Into::into)
    }

    pub fn apply(
        &self,
        change: &ProposalChange,
        expected: &PackageState,
        proposal_id: &str,
    ) -> Result<ApplyResult> {
        let cancellation = peasy_core::cancellation::Cancellation::current();
        cancellation.check()?;
        let _guard = self
            .apply_lock
            .try_lock()
            .map_err(|_| anyhow::anyhow!("A system change is already running"))?;
        let previous = self.current_state()?;
        if &previous != expected {
            bail!("proposal is stale because Peasy state changed; review a new diff");
        }
        let (proposed, message) = match change {
            ProposalChange::Setup { operation, setup } => {
                self.apply_setup_state(&previous, *operation, setup)?
            }
            ProposalChange::Package {
                operation, package, ..
            } => {
                validate_attribute(package)?;
                // A normal apply follows `preview_package`, which already
                // verified this attribute against our immutable Nixpkgs store
                // path. Keep the check for defense in depth, but avoid running
                // the same two Nix evaluations twice for one proposal.
                self.verify(package)?;
                if *operation == PackageOperation::Remove
                    && !previous.packages.iter().any(|item| item == package)
                {
                    bail!("Peasy does not manage `{package}`");
                }
                let proposed = previous.with_change(*operation, package)?;
                let message = match operation {
                    PackageOperation::Install => format!("{package} installed."),
                    PackageOperation::Remove => {
                        let dependents = proposed.setup_dependents(package);
                        if dependents.is_empty() {
                            format!("{package} removed.")
                        } else {
                            format!(
                                "Independent Peasy entry for {package} removed. The package is retained for: {}.",
                                dependents.join(", ")
                            )
                        }
                    }
                };
                (proposed, message)
            }
            ProposalChange::Theme { theme } => {
                let proposed = previous.with_theme(theme)?;
                (
                    proposed,
                    "Appearance saved in the active NixOS generation.".to_owned(),
                )
            }
            ProposalChange::AppImage { operation, package } => {
                package.validate()?;
                let proposed = match operation {
                    PackageOperation::Install => {
                        self.authorize_appimage(package)?;
                        previous.with_appimage_install(package)?
                    }
                    PackageOperation::Remove => {
                        if !previous
                            .appimages
                            .iter()
                            .any(|existing| existing.id == package.id && existing == package)
                        {
                            bail!("external AppImage is no longer in the reviewed state");
                        }
                        previous.with_appimage_remove(&package.id)?
                    }
                };
                let message = format!(
                    "External {} {} {}.",
                    package.display_name,
                    package.version,
                    match operation {
                        PackageOperation::Install => "installed",
                        PackageOperation::Remove => "removed",
                    }
                );
                (proposed, message)
            }
        };
        let _evaluation = self.evaluation_lock.try_lock().map_err(|_| {
            anyhow::anyhow!("A Nix operation is already running; try again shortly")
        })?;
        let stage = self
            .config
            .runtime_dir
            .join("transactions")
            .join(proposal_id);
        if stage.exists() {
            bail!("proposal staging directory already exists");
        }
        fs::create_dir_all(&stage)?;
        struct StagingCleanup<'a>(&'a Path);
        impl Drop for StagingCleanup<'_> {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(self.0);
            }
        }
        let _staging_cleanup = StagingCleanup(&stage);
        fs::set_permissions(&stage, fs::Permissions::from_mode(0o700))?;
        fs::write(
            stage.join("peasy-managed.nix"),
            render_packages_module(&proposed)?,
        )?;

        let out_link = stage.join("result");
        let system_expression = match &self.config.rebuild_target {
            RebuildTarget::Configuration { path } => {
                let system_expression = stage.join("system.nix");
                fs::write(
                    &system_expression,
                    render_system_expression(&self.config.nixpkgs, path, &self.config.system)?,
                )?;
                Some(system_expression)
            }
            RebuildTarget::Flake { .. } => None,
        };

        cancellation.check()?;
        state::write_managed_atomic(&self.config.managed_module, &proposed)?;
        let build_result = match &self.config.rebuild_target {
            RebuildTarget::Configuration { .. } => {
                let system_expression = system_expression
                    .as_ref()
                    .context("missing staged system expression")?;
                self.runner.run(
                    &self.config.nix,
                    &[
                        "build".into(),
                        "--file".into(),
                        system_expression.as_os_str().to_owned(),
                        "--out-link".into(),
                        out_link.as_os_str().to_owned(),
                    ],
                    Some(&stage),
                )
            }
            RebuildTarget::Flake {
                reference,
                nixos_rebuild,
            } => {
                let path_reference = format!("path:{reference}");
                self.runner.run(
                    nixos_rebuild,
                    &[
                        "build".into(),
                        "--flake".into(),
                        path_reference.into(),
                        "--no-write-lock-file".into(),
                        "--out-link".into(),
                        out_link.as_os_str().to_owned(),
                    ],
                    Some(&stage),
                )
            }
        };
        let build = match build_result {
            Ok(build) => build,
            Err(error) => {
                state::write_managed_atomic(&self.config.managed_module, &previous)
                    .context("restoring peasy-managed.nix after build command failed")?;
                let _ = fs::remove_dir_all(&stage);
                return Err(error);
            }
        };
        if !build.status.success() {
            state::write_managed_atomic(&self.config.managed_module, &previous)
                .context("restoring peasy-managed.nix after build failure")?;
            let _ = fs::remove_dir_all(&stage);
            return Ok(ApplyResult {
                configuration_valid: false,
                build_successful: false,
                activated: false,
                message: format!("Configuration test failed: {}", useful_stderr(&build)),
            });
        }
        let system = match fs::canonicalize(&out_link).context("NixOS build produced no result") {
            Ok(system) => system,
            Err(error) => {
                state::write_managed_atomic(&self.config.managed_module, &previous)
                    .context("restoring peasy-managed.nix after invalid build output")?;
                return Err(error);
            }
        };
        if !system.starts_with("/nix/store/")
            || !system.join("bin/switch-to-configuration").is_file()
        {
            state::write_managed_atomic(&self.config.managed_module, &previous)
                .context("restoring peasy-managed.nix after invalid system result")?;
            bail!("NixOS build returned an invalid system path");
        }
        if let Err(error) = verify_built_managed_state(&system, &proposed) {
            state::write_managed_atomic(&self.config.managed_module, &previous)
                .context("restoring peasy-managed.nix after integration check failed")?;
            let _ = fs::remove_dir_all(&stage);
            return Ok(ApplyResult {
                configuration_valid: false,
                build_successful: false,
                activated: false,
                message: format!(
                    "The host configuration did not import {}: {error}",
                    self.config.managed_module.display()
                ),
            });
        }
        // Cancellation is allowed until this atomic transition. Once protected,
        // neither window closure nor a disconnected client can kill activation.
        if let Err(error) = cancellation.protect() {
            state::write_managed_atomic(&self.config.managed_module, &previous)
                .context("restoring peasy-managed.nix after cancellation")?;
            let _ = fs::remove_dir_all(&stage);
            return Err(error.into());
        }
        let activation_attempt = (|| -> Result<_> {
            activation::write_request(&self.config.runtime_dir, &system)?;
            let activation = self.runner.run(
                &self.config.systemctl,
                &["start".into(), "peasy-activate.service".into()],
                None,
            )?;
            let activation_result = match activation::read_result(&self.config.runtime_dir) {
                Ok(result) => result,
                Err(error) => {
                    let command_error = useful_stderr(&activation);
                    let command_error = if command_error.is_empty() {
                        format!("systemctl exited with {}", activation.status)
                    } else {
                        command_error
                    };
                    bail!(
                        "activation service failed before returning a result: {command_error}; {error:#}"
                    );
                }
            };
            Ok((activation, activation_result))
        })();
        let (activation, activation_result) = match activation_attempt {
            Ok(result) => result,
            Err(error) => {
                state::write_managed_atomic(&self.config.managed_module, &previous)
                    .context("restoring peasy-managed.nix after activation could not run")?;
                return Err(error);
            }
        };
        if !activation.status.success() || !activation_result.activated {
            state::write_managed_atomic(&self.config.managed_module, &previous)
                .context("restoring peasy-managed.nix after activation failure")?;
            bail!("system activation failed: {}", activation_result.message);
        }

        let _ = fs::remove_dir_all(&stage);
        Ok(ApplyResult {
            configuration_valid: true,
            build_successful: true,
            activated: true,
            message,
        })
    }

    pub fn is_applying(&self) -> bool {
        self.apply_lock.try_lock().is_err()
    }
}

fn verify_built_managed_state(system: &Path, expected: &PackageState) -> Result<()> {
    let path = system.join("etc/peasy/state.json");
    let mut actual: PackageState = serde_json::from_slice(
        &fs::read(&path).with_context(|| format!("reading {}", path.display()))?,
    )
    .context("parsing the built Peasy state")?;
    actual.normalize()?;
    if &actual != expected {
        bail!("the built generation contains different Peasy state");
    }
    Ok(())
}

fn useful_stderr(output: &Output) -> String {
    let text = String::from_utf8_lossy(&output.stderr);
    let mut lines = text.lines().rev().take(12).collect::<Vec<_>>();
    lines.reverse();
    lines.join("\n").chars().take(1600).collect()
}

fn human_name(value: &str) -> String {
    value
        .split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use peasy_core::AppImagePackage;
    use std::collections::VecDeque;
    use std::os::unix::process::ExitStatusExt;

    struct MockRunner {
        outputs: Mutex<VecDeque<Output>>,
        calls: Mutex<Vec<Vec<OsString>>>,
    }

    impl CommandRunner for MockRunner {
        fn run(&self, _program: &Path, args: &[OsString], _cwd: Option<&Path>) -> Result<Output> {
            self.calls.lock().unwrap().push(args.to_vec());
            self.outputs
                .lock()
                .unwrap()
                .pop_front()
                .context("unexpected trusted command")
        }
    }

    fn output(code: i32, stdout: &str, stderr: &str) -> Output {
        Output {
            status: std::process::ExitStatus::from_raw(code << 8),
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    #[test]
    fn cancelled_build_restores_source_cleans_staging_and_releases_locks() {
        struct CancelBuild;
        impl CommandRunner for CancelBuild {
            fn run(&self, _: &Path, args: &[OsString], cwd: Option<&Path>) -> Result<Output> {
                assert_eq!(args[0], "build", "must never reach activation");
                assert!(cwd.unwrap().join("peasy-managed.nix").is_file());
                let token = peasy_core::cancellation::Cancellation::current();
                token.cancel();
                token.check()?;
                unreachable!()
            }
        }
        let temp = tempfile::tempdir().unwrap();
        let backend =
            NixBackend::new(config(temp.path().join("state")), Arc::new(CancelBuild)).unwrap();
        let before = backend.current_state().unwrap();
        let preview = backend
            .preview_theme(ThemeSettings {
                accent_color: Some(peasy_core::AccentColor::Green),
                color_scheme: None,
            })
            .unwrap();
        let token = peasy_core::cancellation::Cancellation::default();
        let id = "a".repeat(48);
        let error = token
            .scope(|| backend.apply(&preview.change, &preview.before, &id))
            .unwrap_err();
        assert!(
            error
                .downcast_ref::<peasy_core::cancellation::Cancelled>()
                .is_some()
        );
        assert_eq!(backend.current_state().unwrap(), before);
        assert!(
            !backend
                .config
                .runtime_dir
                .join("transactions")
                .join(id)
                .exists()
        );
        assert!(!backend.is_applying());
        assert!(backend.evaluation_lock.try_lock().is_ok());
    }

    fn config(runtime_dir: PathBuf) -> BackendConfig {
        let managed_module = runtime_dir.join("source/peasy-managed.nix");
        BackendConfig {
            appimage_policy: runtime_dir.join("appimage-policy.json"),
            runtime_dir,
            nix: "/trusted/nix".into(),
            systemctl: "/trusted/systemctl".into(),
            nixpkgs: "/nix/store/00000000000000000000000000000000-nixpkgs".into(),
            system: "x86_64-linux".into(),
            managed_module,
            rebuild_target: RebuildTarget::Configuration {
                path: "/etc/nixos/configuration.nix".into(),
            },
        }
    }

    fn appimage() -> AppImagePackage {
        AppImagePackage {
            id: "appimage.example.nostr-chat".into(),
            display_name: "Nostr Chat".into(),
            repository: "example/nostr-chat".into(),
            version: "1.2".into(),
            release_tag: "v1.2".into(),
            asset_name: "nostr-chat-x86_64.AppImage".into(),
            url: "https://github.com/example/nostr-chat/releases/download/v1.2/nostr-chat-x86_64.AppImage".into(),
            hash: "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".into(),
            architecture: peasy_core::AppImageArchitecture::X86_64,
            size: 42_000_000,
        }
    }

    #[test]
    fn external_appimage_review_is_pinned_and_needs_no_network_privilege() {
        let temporary = tempfile::tempdir().unwrap();
        let runner = Arc::new(MockRunner {
            outputs: Mutex::new(VecDeque::new()),
            calls: Mutex::new(vec![]),
        });
        let backend =
            NixBackend::new(config(temporary.path().join("state")), runner.clone()).unwrap();
        let package = appimage();
        assert!(backend.preview_appimage_install(package.clone()).is_err());
        let policy = peasy_core::AppImagePolicy(Some(std::collections::BTreeMap::from([(
            package.repository.clone(),
            vec![package.hash.clone()],
        )])));
        fs::write(
            &backend.config.appimage_policy,
            serde_json::to_vec(&policy).unwrap(),
        )
        .unwrap();
        let preview = backend.preview_appimage_install(package.clone()).unwrap();
        assert!(preview.title.contains("Install external Nostr Chat 1.2"));
        assert!(
            preview
                .diff
                .iter()
                .any(|line| line.text.contains(&package.repository))
        );
        assert!(
            preview
                .diff
                .iter()
                .any(|line| line.text.contains(&package.hash))
        );
        assert!(
            preview
                .diff
                .iter()
                .any(|line| line.text.contains("wrapType2"))
        );
        assert!(runner.calls.lock().unwrap().is_empty());

        // The default module policy permits review without a preapproved digest.
        // The privileged daemon still validates the complete record and rechecks
        // policy at Apply if an administrator tightens it after the preview.
        fs::write(&backend.config.appimage_policy, "null").unwrap();
        let preview = backend.preview_appimage_install(package.clone()).unwrap();
        assert!(
            preview
                .diff
                .iter()
                .any(|line| line.text == format!("Download: {}", package.url))
        );
        fs::write(&backend.config.appimage_policy, "{}").unwrap();
        assert!(
            backend
                .apply(&preview.change, &preview.before, "policy-changed")
                .is_err()
        );
        assert!(backend.packages().unwrap().is_empty());
        assert!(runner.calls.lock().unwrap().is_empty());
        fs::write(&backend.config.appimage_policy, "null").unwrap();
        let mut unpinned = package;
        unpinned.hash = "not-a-hash".into();
        assert!(backend.preview_appimage_install(unpinned).is_err());
        assert!(runner.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn verification_uses_the_pinned_import_and_reuses_its_cache() {
        let temporary = tempfile::tempdir().unwrap();
        let runner = Arc::new(MockRunner {
            outputs: Mutex::new(VecDeque::from([
                output(0, "{}", ""),
                output(0, "hello", ""),
            ])),
            calls: Mutex::new(Vec::new()),
        });
        let backend =
            NixBackend::new(config(temporary.path().join("state")), runner.clone()).unwrap();
        assert_eq!(
            backend.verify("hello").unwrap(),
            backend.verify("hello").unwrap()
        );
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        for call in calls.iter() {
            assert!(call.iter().any(|arg| arg == "--expr"));
            let expression = call.last().unwrap().to_string_lossy();
            assert!(expression.contains("builtins.toPath"));
            assert!(expression.contains("getAttrFromPath"));
            assert!(!expression.contains("path:"));
        }
    }

    #[test]
    fn failed_configuration_build_never_activates_or_commits() {
        let temporary = tempfile::tempdir().unwrap();
        let runner = Arc::new(MockRunner {
            outputs: Mutex::new(VecDeque::from([
                output(0, "{}", ""),
                output(0, "telegram-desktop", ""),
                output(1, "", "deliberate build failure"),
            ])),
            calls: Mutex::new(vec![]),
        });
        let backend =
            NixBackend::new(config(temporary.path().join("state")), runner.clone()).unwrap();
        let preview = backend
            .preview_package(PackageOperation::Install, "telegram-desktop")
            .unwrap();
        let result = backend
            .apply(&preview.change, &preview.before, &"a".repeat(48))
            .unwrap();
        assert!(!result.activated);
        assert!(backend.packages().unwrap().is_empty());
        assert_eq!(backend.current_state().unwrap(), PackageState::default());
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[2][0], "build");
        assert!(calls[2].iter().any(|argument| argument == "--file"));
        assert!(!calls[2].iter().any(|argument| argument == "--flake"));
    }

    #[test]
    fn setup_build_failure_restores_every_contribution() {
        let temporary = tempfile::tempdir().unwrap();
        let runner = Arc::new(MockRunner {
            outputs: Mutex::new(VecDeque::from([
                output(0, "{}", ""),
                output(0, "hello", ""),
                output(1, "", "deliberate setup build failure"),
            ])),
            calls: Mutex::new(vec![]),
        });
        let backend =
            NixBackend::new(config(temporary.path().join("state")), runner.clone()).unwrap();
        let settings = peasy_core::SystemSetup {
            packages: vec![],
            enable: vec!["services.printing.enable".into()],
            groups: vec![],
        };
        let preview = backend
            .preview_setup("hello".into(), settings, 1000)
            .unwrap();
        assert!(
            preview
                .diff
                .iter()
                .any(|line| line.text.contains("services.printing.enable = true"))
        );
        let result = backend
            .apply(&preview.change, &preview.before, &"e".repeat(48))
            .unwrap();
        assert!(!result.activated);
        assert_eq!(backend.current_state().unwrap(), preview.before);
        assert_eq!(runner.calls.lock().unwrap().len(), 3);
    }

    #[test]
    fn setup_uninstall_preserves_shared_and_external_ownership() {
        let temporary = tempfile::tempdir().unwrap();
        let runner = Arc::new(MockRunner {
            outputs: Mutex::new(VecDeque::new()),
            calls: Mutex::new(vec![]),
        });
        let backend =
            NixBackend::new(config(temporary.path().join("state")), runner.clone()).unwrap();
        let setup: peasy_core::ManagedSetup = serde_json::from_str(include_str!(
            "../../../peas/system_configuration/example.json"
        ))
        .unwrap();
        let mut other = setup.clone();
        other.package = "virt-viewer".into();
        let state = PackageState::default()
            .with_change(PackageOperation::Install, "virt-manager")
            .unwrap()
            .with_setup(setup)
            .unwrap()
            .with_setup(other)
            .unwrap();
        assert!(state.packages.is_empty());
        state::write_managed_atomic(&backend.config.managed_module, &state).unwrap();
        let preview = backend
            .preview_package(PackageOperation::Remove, "virt-manager")
            .unwrap();
        let ProposalChange::Setup { operation, setup } = &preview.change else {
            panic!("must remove full setup")
        };
        let (after, _) = backend
            .apply_setup_state(&preview.before, *operation, setup)
            .unwrap();
        assert_eq!(
            after.effective_packages().into_iter().collect::<Vec<_>>(),
            ["virt-viewer"]
        );
        assert!(
            render_packages_module(&after)
                .unwrap()
                .contains("  virtualisation.libvirtd.enable = true;")
        );
        assert!(runner.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn invalid_setup_never_runs_nix_or_changes_state() {
        let temporary = tempfile::tempdir().unwrap();
        let runner = Arc::new(MockRunner {
            outputs: Mutex::new(VecDeque::new()),
            calls: Mutex::new(vec![]),
        });
        let backend =
            NixBackend::new(config(temporary.path().join("state")), runner.clone()).unwrap();
        for settings in [
            peasy_core::SystemSetup {
                packages: vec![],
                enable: vec!["services.openssh.enable".into()],
                groups: vec![],
            },
            peasy_core::SystemSetup {
                packages: vec![],
                enable: vec!["virtualisation.libvirtd.enable".into()],
                groups: vec!["wheel".into()],
            },
        ] {
            assert!(
                backend
                    .preview_setup("hello".into(), settings, 1000)
                    .is_err()
            );
        }
        let setup: peasy_core::ManagedSetup = serde_json::from_str(include_str!(
            "../../../peas/system_configuration/example.json"
        ))
        .unwrap();
        assert!(
            backend
                .preview_setup(setup.package, setup.settings, 0)
                .is_err()
        );
        assert!(runner.calls.lock().unwrap().is_empty());
        assert_eq!(backend.current_state().unwrap(), PackageState::default());
    }

    #[test]
    fn failed_setup_uninstall_restores_the_complete_prior_setup() {
        let temporary = tempfile::tempdir().unwrap();
        let runner = Arc::new(MockRunner {
            outputs: Mutex::new(VecDeque::from([output(
                1,
                "",
                "deliberate uninstall build failure",
            )])),
            calls: Mutex::new(vec![]),
        });
        let backend =
            NixBackend::new(config(temporary.path().join("state")), runner.clone()).unwrap();
        let setup: peasy_core::ManagedSetup = serde_json::from_str(include_str!(
            "../../../peas/system_configuration/example.json"
        ))
        .unwrap();
        let before = PackageState::default().with_setup(setup).unwrap();
        state::write_managed_atomic(&backend.config.managed_module, &before).unwrap();
        let preview = backend
            .preview_package(PackageOperation::Remove, "virt-manager")
            .unwrap();
        let result = backend
            .apply(&preview.change, &preview.before, &"f".repeat(48))
            .unwrap();
        assert!(!result.activated);
        assert_eq!(backend.current_state().unwrap(), before);
        assert_eq!(runner.calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn theme_preview_contains_the_exact_reviewable_change() {
        let temporary = tempfile::tempdir().unwrap();
        let runner = Arc::new(MockRunner {
            outputs: Mutex::new(VecDeque::new()),
            calls: Mutex::new(vec![]),
        });
        let backend =
            NixBackend::new(config(temporary.path().join("state")), runner.clone()).unwrap();
        let preview = backend
            .preview_theme(ThemeSettings {
                accent_color: Some(peasy_core::AccentColor::Blue),
                color_scheme: Some(peasy_core::ColorScheme::Dark),
            })
            .unwrap();

        assert_eq!(preview.before, PackageState::default());
        assert_eq!(
            preview.title,
            "Change appearance to blue accent and dark mode"
        );
        assert!(preview.diff.iter().any(|line| {
            line.kind == peasy_core::DiffKind::Add && line.text.contains("accent-color = \"blue\"")
        }));
        assert!(preview.diff.iter().any(|line| {
            line.kind == peasy_core::DiffKind::Add
                && line.text.contains("color-scheme = \"prefer-dark\"")
        }));
        assert!(runner.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn failed_theme_build_never_activates_or_commits() {
        let temporary = tempfile::tempdir().unwrap();
        let runner = Arc::new(MockRunner {
            outputs: Mutex::new(VecDeque::from([output(
                1,
                "",
                "deliberate theme build failure",
            )])),
            calls: Mutex::new(vec![]),
        });
        let backend =
            NixBackend::new(config(temporary.path().join("state")), runner.clone()).unwrap();
        let result = backend
            .apply(
                &ProposalChange::Theme {
                    theme: ThemeSettings {
                        accent_color: Some(peasy_core::AccentColor::Purple),
                        color_scheme: Some(peasy_core::ColorScheme::Dark),
                    },
                },
                &PackageState::default(),
                &"c".repeat(48),
            )
            .unwrap();

        assert!(!result.configuration_valid);
        assert!(!result.build_successful);
        assert!(!result.activated);
        assert_eq!(backend.theme().unwrap(), ThemeSettings::default());
        assert_eq!(backend.current_state().unwrap(), PackageState::default());
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0][0], "build");
    }

    #[test]
    fn stale_proposal_is_rejected_before_any_command_or_write() {
        let temporary = tempfile::tempdir().unwrap();
        let runner = Arc::new(MockRunner {
            outputs: Mutex::new(VecDeque::new()),
            calls: Mutex::new(vec![]),
        });
        let backend =
            NixBackend::new(config(temporary.path().join("state")), runner.clone()).unwrap();
        state::write_managed_atomic(
            &backend.config.managed_module,
            &PackageState {
                packages: vec!["vlc".into()],
                setups: Vec::new(),
                appimages: Vec::new(),
                theme: ThemeSettings::default(),
            },
        )
        .unwrap();

        let result = backend.apply(
            &ProposalChange::Theme {
                theme: ThemeSettings {
                    accent_color: Some(peasy_core::AccentColor::Green),
                    color_scheme: None,
                },
            },
            &PackageState::default(),
            &"d".repeat(48),
        );

        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("proposal is stale")
        );
        assert!(runner.calls.lock().unwrap().is_empty());
        assert!(
            !temporary
                .path()
                .join("state/transactions")
                .join("d".repeat(48))
                .exists()
        );
    }

    #[test]
    fn unknown_package_never_changes_state() {
        let temporary = tempfile::tempdir().unwrap();
        let runner = Arc::new(MockRunner {
            outputs: Mutex::new(VecDeque::from([output(1, "", "unknown package")])),
            calls: Mutex::new(vec![]),
        });
        let backend = NixBackend::new(config(temporary.path().join("state")), runner).unwrap();
        let before = PackageState::default();
        assert!(
            backend
                .apply(
                    &ProposalChange::Package {
                        operation: PackageOperation::Install,
                        package: "not-a-real-package".into(),
                        display_name: "Not Real".into(),
                    },
                    &before,
                    &"b".repeat(48),
                )
                .is_err()
        );
        assert!(backend.packages().unwrap().is_empty());
        assert_eq!(backend.current_state().unwrap(), PackageState::default());
    }

    #[test]
    fn search_text_cannot_become_a_nix_option() {
        let temporary = tempfile::tempdir().unwrap();
        let runner = Arc::new(MockRunner {
            outputs: Mutex::new(VecDeque::from([output(0, "{}", "")])),
            calls: Mutex::new(vec![]),
        });
        let backend =
            NixBackend::new(config(temporary.path().join("state")), runner.clone()).unwrap();
        assert!(backend.search("--option").unwrap().is_empty());
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls[0].last().unwrap(), ".*--option.*");
    }

    #[test]
    fn search_filters_for_the_host_ranks_desktop_apps_and_uses_cache() {
        let temporary = tempfile::tempdir().unwrap();
        let search = serde_json::json!({
            "legacyPackages.x86_64-linux.whatsapp-for-mac": {
                "pname": "whatsapp-for-mac",
                "description": "WhatsApp desktop client for macOS"
            },
            "legacyPackages.x86_64-linux.whatsapp-electron": {
                "pname": "whatsapp-electron",
                "description": "Unofficial WhatsApp desktop client for Linux"
            },
            "legacyPackages.x86_64-linux.whatsapp-emoji-font": {
                "pname": "whatsapp-emoji-font",
                "description": "Emoji font"
            },
            "legacyPackages.x86_64-linux.whatsapp-chat-exporter": {
                "pname": "whatsapp-chat-exporter",
                "description": "Export WhatsApp chats"
            },
            "legacyPackages.x86_64-linux.altus": {
                "pname": "altus",
                "description": "An Electron WhatsApp client"
            }
        });
        let compatible = serde_json::json!([
            "whatsapp-electron",
            "whatsapp-emoji-font",
            "whatsapp-chat-exporter",
            "altus"
        ]);
        let runner = Arc::new(MockRunner {
            outputs: Mutex::new(VecDeque::from([
                output(0, &search.to_string(), ""),
                output(0, &compatible.to_string(), ""),
            ])),
            calls: Mutex::new(vec![]),
        });
        let backend =
            NixBackend::new(config(temporary.path().join("state")), runner.clone()).unwrap();

        let first = backend.search("whatsapp").unwrap();
        assert_eq!(first[0].attribute, "whatsapp-electron");
        assert!(
            !first
                .iter()
                .any(|item| item.attribute == "whatsapp-for-mac")
        );
        assert!(
            first
                .iter()
                .position(|item| item.attribute == "altus")
                .unwrap()
                < first
                    .iter()
                    .position(|item| item.attribute == "whatsapp-emoji-font")
                    .unwrap()
        );

        let second = backend.search("WHATSAPP").unwrap();
        assert_eq!(first, second);
        let preview = backend
            .preview_package(PackageOperation::Install, "whatsapp-electron")
            .unwrap();
        assert_eq!(preview.title, "Install Whatsapp Electron");
        let calls = runner.calls.lock().unwrap();
        assert_eq!(
            calls.len(),
            2,
            "repeated searches and proposal verification must use the cache"
        );
        assert_eq!(calls[0][0], "search");
        assert_eq!(calls[1][0], "eval");
        assert!(calls[1].iter().any(|argument| argument == "--impure"));
        let compatibility_expression = calls[1].last().unwrap().to_string_lossy();
        assert!(compatibility_expression.contains("lib.meta.availableOn"));
        assert!(compatibility_expression.contains("builtins.tryEval"));
        assert!(compatibility_expression.contains("x86_64-linux"));
    }
}

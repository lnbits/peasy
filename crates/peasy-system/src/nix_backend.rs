#[path = "../../../peas/appearance/system.rs"]
mod appearance;
#[path = "../../../peas/appimages/system.rs"]
mod appimages;
#[path = "../../../peas/packages/system.rs"]
mod packages;
#[path = "../../../peas/system_configuration/system.rs"]
mod system_configuration;
use packages::CachedSearch;

use crate::{activation, recovery, state};
use anyhow::{Context, Result, bail};
use peasy_core::{
    ApplyResult, DiffLine, PackageOperation, PackageState, ProposalChange, ThemeSettings,
    render_packages_module, validate_attribute,
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
    Configuration { path: PathBuf },
    Flake { reference: String },
}

#[derive(Clone, Debug)]
pub struct BackendConfig {
    pub identity: Option<PathBuf>,
    pub active_system: PathBuf,
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

    apply_lock: Mutex<()>,
    evaluation_lock: Mutex<()>,
}

#[derive(Clone)]

pub struct Preview {
    pub packages: Vec<peasy_core::PackageIdentity>,
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
        recovery::startup(&config.managed_module, &config.active_system)?;
        Ok(Self {
            config,
            runner,
            search_cache: Mutex::new(HashMap::new()),

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
        reviewed: &[peasy_core::PackageIdentity],
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
        if let ProposalChange::Recovery { generation } = change {
            return self.restore_previous(generation);
        }
        if recovery::load(&self.config.managed_module)?.is_some() {
            bail!("An interrupted change needs recovery before another system change");
        }
        let mut expected_attributes = match change {
            ProposalChange::Package {
                operation: PackageOperation::Install,
                package,
                ..
            } => vec![package.clone()],
            ProposalChange::Setup {
                operation: PackageOperation::Install,
                setup,
            } => {
                let mut a = setup.settings.packages.clone();
                a.push(setup.package.clone());
                a
            }
            _ => vec![],
        };
        expected_attributes.sort();
        expected_attributes.dedup();
        let mut reviewed_attributes = reviewed
            .iter()
            .map(|p| p.attribute.clone())
            .collect::<Vec<_>>();
        reviewed_attributes.sort();
        reviewed_attributes.dedup();
        if expected_attributes != reviewed_attributes {
            bail!("Package review is incomplete; review the change again");
        }
        let (proposed, message) = match change {
            ProposalChange::Recovery { .. } => unreachable!(),
            ProposalChange::Setup { operation, setup } => {
                self.apply_setup_state(&previous, *operation, setup)?
            }
            ProposalChange::Package {
                operation, package, ..
            } => {
                validate_attribute(package)?;
                // The build expression enforces the reviewed host derivation.

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
        let system_expression = stage.join("system.nix");
        let assertions = reviewed.iter().map(|p| format!(
            "assert ((host.pkgs.lib.getAttrFromPath (host.pkgs.lib.splitString \".\" {}) host.pkgs).drvPath == {}) || builtins.throw \"Reviewed package changed; review again\";",
            peasy_core::nix_string(&p.attribute), peasy_core::nix_string(&p.drv_path)
        )).collect::<Vec<_>>();
        // Assertions and the toplevel share one evaluation of the host module
        // graph. A mutable overlay cannot silently change reviewed packages.
        let assertions = assertions.join("\n");
        fs::write(
            &system_expression,
            format!(
                "let host = {}; in\n{}\nhost.config.system.build.toplevel",
                self.host_expression()?,
                assertions
            ),
        )?;
        cancellation.check()?;
        let mut journal = recovery::Journal {
            before: previous.clone(),
            proposed: proposed.clone(),
            previous_generation: recovery::generation(&self.config.active_system),
            target_generation: None,
            phase: recovery::Phase::Building,
        };
        recovery::write(&recovery::path(&self.config.managed_module), &journal)?;
        struct JournalCleanup<'a> {
            managed: &'a Path,
            previous: &'a PackageState,
            proposed: &'a PackageState,
            activating: bool,
        }
        impl JournalCleanup<'_> {
            fn rollback(&self) -> Result<()> {
                if self.activating {
                    return Ok(());
                }
                let current = state::load_managed(self.managed)?;
                if current == *self.proposed {
                    state::write_managed_atomic(self.managed, self.previous)
                        .context("Could not restore the previous configuration; open System status and recovery")?;
                } else if current != *self.previous {
                    bail!(
                        "The managed configuration changed outside this operation; open System status and recovery"
                    );
                }
                recovery::clear(self.managed)
                    .context("Could not clear the interrupted-operation record; open System status and recovery")
            }
        }
        impl Drop for JournalCleanup<'_> {
            fn drop(&mut self) {
                // Best effort for unexpected unwinding; ordinary failure paths
                // call rollback explicitly so restoration errors reach the UI.
                let _ = self.rollback();
            }
        }
        let mut journal_cleanup = JournalCleanup {
            managed: &self.config.managed_module,
            previous: &previous,
            proposed: &proposed,
            activating: false,
        };
        state::write_managed_atomic(&self.config.managed_module, &proposed)?;
        peasy_core::progress::report(peasy_core::OperationStage::Validating);
        let build_result = self.runner.run(
            &self.config.nix,
            &[
                "build".into(),
                "--impure".into(),
                "--no-write-lock-file".into(),
                "--log-format".into(),
                "internal-json".into(),
                "--file".into(),
                system_expression.into_os_string(),
                "--out-link".into(),
                out_link.as_os_str().to_owned(),
            ],
            Some(&stage),
        );
        let build = match build_result {
            Ok(build) => build,
            Err(error) => {
                journal_cleanup.rollback()?;
                return Err(error);
            }
        };
        if !build.status.success() {
            journal_cleanup.rollback()?;
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
                journal_cleanup.rollback()?;
                return Err(error);
            }
        };
        if !system.starts_with("/nix/store/")
            || !system.join("bin/switch-to-configuration").is_file()
        {
            journal_cleanup.rollback()?;
            bail!("NixOS build returned an invalid system path");
        }
        if let Err(error) = verify_built_managed_state(&system, &proposed) {
            journal_cleanup.rollback()?;
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
            journal_cleanup.rollback()?;
            let _ = fs::remove_dir_all(&stage);
            return Err(error.into());
        }
        journal.phase = recovery::Phase::Activating;
        journal.target_generation = Some(system.clone());
        recovery::write(&recovery::path(&self.config.managed_module), &journal)?;
        journal_cleanup.activating = true;
        peasy_core::progress::report(peasy_core::OperationStage::Activating);
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
                return Err(error);
            }
        };
        if !activation.status.success() || !activation_result.activated {
            bail!("system activation failed: {}", activation_result.message);
        }

        let _ = fs::remove_dir_all(&stage);
        recovery::completed(&self.config.managed_module)?;
        peasy_core::progress::report(peasy_core::OperationStage::Completed);
        Ok(ApplyResult {
            configuration_valid: true,
            build_successful: true,
            activated: true,
            message,
        })
    }

    pub fn should_restart(&self) -> bool {
        self.config.identity.as_ref().is_some_and(|expected| {
            match (
                fs::read(expected),
                fs::read(
                    self.config
                        .active_system
                        .join("etc/peasy/daemon-identity.json"),
                ),
            ) {
                (Ok(expected), Ok(active)) => expected != active,
                _ => false,
            }
        })
    }

    pub fn inspect(&self) -> Result<peasy_core::ServiceStatus> {
        Ok(peasy_core::ServiceStatus {
            version: env!("CARGO_PKG_VERSION").into(),
            protocol: 2,
            executable: std::env::current_exe()?.to_string_lossy().into_owned(),
            nixpkgs: self.config.nixpkgs.to_string_lossy().into_owned(),
            restart_pending: self.should_restart(),
            applying: self.is_applying(),
            recovery: if self.is_applying() {
                None
            } else {
                recovery::info(&self.config.managed_module, &self.config.active_system)?
            },
        })
    }

    pub fn preview_recovery(&self) -> Result<Preview> {
        let journal = recovery::load(&self.config.managed_module)?
            .context("No interrupted operation needs recovery")?;
        let generation = journal.previous_generation.context("Previous generation is unavailable; select a known-good generation from the boot menu")?.to_string_lossy().into_owned();
        Ok(Preview {
            before: self.current_state()?,
            packages: vec![],
            change: ProposalChange::Recovery {
                generation: generation.clone(),
            },
            title: "Restore the previous system generation".into(),
            diff: vec![DiffLine {
                kind: peasy_core::DiffKind::Context,
                text: format!(
                    "Restore {generation}. This changes the whole system generation. User files are retained; service side effects may require separate attention."
                ),
            }],
        })
    }

    fn restore_previous(&self, generation: &str) -> Result<ApplyResult> {
        let mut journal = recovery::load(&self.config.managed_module)?
            .context("Recovery is no longer pending")?;
        let previous = journal
            .previous_generation
            .as_ref()
            .context("Previous generation is unavailable")?;
        if previous.to_string_lossy() != generation {
            bail!("Recovery proposal is stale");
        }
        let running = self.runner.run(
            &self.config.systemctl,
            &[
                "show".into(),
                "--property=ActiveState".into(),
                "--value".into(),
                "peasy-activate.service".into(),
            ],
            None,
        )?;
        if !running.status.success() {
            bail!("Could not check activation service state; refresh before recovery");
        }
        if !matches!(
            String::from_utf8_lossy(&running.stdout).trim(),
            "inactive" | "failed"
        ) {
            bail!("System activation is still running. Wait for it to finish before recovery");
        }
        peasy_core::cancellation::Cancellation::current().protect()?;
        journal.phase = recovery::Phase::Activating;
        journal.target_generation = Some(previous.clone());
        recovery::write(&recovery::path(&self.config.managed_module), &journal)?;
        peasy_core::progress::report(peasy_core::OperationStage::Activating);
        activation::write_request(&self.config.runtime_dir, previous)?;
        let output = self.runner.run(
            &self.config.systemctl,
            &["start".into(), "peasy-activate.service".into()],
            None,
        )?;
        let result = activation::read_result(&self.config.runtime_dir)?;
        if !output.status.success() || !result.activated {
            bail!("Recovery activation failed: {}", result.message);
        }
        state::write_managed_atomic(&self.config.managed_module, &journal.before)?;
        recovery::completed(&self.config.managed_module)?;
        peasy_core::progress::report(peasy_core::OperationStage::Completed);
        Ok(ApplyResult {
            configuration_valid: true,
            build_successful: true,
            activated: true,
            message:
                "Previous system generation restored. Review the request again before retrying."
                    .into(),
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
    let raw = String::from_utf8_lossy(&output.stderr);
    let text = raw
        .lines()
        .filter_map(|line| {
            if let Some(json) = line.strip_prefix("@nix ") {
                serde_json::from_str::<serde_json::Value>(json)
                    .ok()?
                    .get("msg")?
                    .as_str()
                    .map(str::to_owned)
            } else {
                Some(line.to_owned())
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
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

    fn identity_output(attributes: &[&str]) -> Output {
        output(
            0,
            &serde_json::to_string(
                &attributes
                    .iter()
                    .map(|attribute| peasy_core::PackageIdentity {
                        attribute: (*attribute).into(),
                        name: (*attribute).into(),
                        version: "1.0".into(),
                        drv_path: format!(
                            "/nix/store/00000000000000000000000000000000-{attribute}.drv"
                        ),
                    })
                    .collect::<Vec<_>>(),
            )
            .unwrap(),
            "",
        )
    }

    #[test]
    fn failed_build_preserves_a_concurrent_edit_and_reports_recovery() {
        struct ChangedDuringBuild(PathBuf);
        impl CommandRunner for ChangedDuringBuild {
            fn run(&self, _: &Path, _: &[OsString], _: Option<&Path>) -> Result<Output> {
                let foreign =
                    PackageState::default().with_change(PackageOperation::Install, "hello")?;
                state::write_managed_atomic(&self.0, &foreign)?;
                Ok(output(1, "", "failed build"))
            }
        }
        let temp = tempfile::tempdir().unwrap();
        let configuration = config(temp.path().join("state"));
        let runner = Arc::new(ChangedDuringBuild(configuration.managed_module.clone()));
        let backend = NixBackend::new(configuration, runner).unwrap();
        let preview = backend
            .preview_theme(ThemeSettings {
                accent_color: Some(peasy_core::AccentColor::Blue),
                color_scheme: None,
            })
            .unwrap();
        let error = backend
            .apply(&preview.change, &preview.before, &"a".repeat(48), &[])
            .unwrap_err();
        assert!(error.to_string().contains("changed outside this operation"));
        assert_eq!(backend.packages().unwrap(), ["hello"]);
        assert!(backend.inspect().unwrap().recovery.unwrap().needs_attention);
    }

    #[test]
    fn interrupted_build_is_recovered_after_process_exit() {
        const FIXTURE: &str = "PEASY_CRASH_TEST_DIRECTORY";
        if let Some(directory) = std::env::var_os(FIXTURE) {
            struct ExitDuringBuild;
            impl CommandRunner for ExitDuringBuild {
                fn run(&self, _: &Path, args: &[OsString], _: Option<&Path>) -> Result<Output> {
                    assert_eq!(args[0], "build");
                    std::process::exit(93);
                }
            }
            let backend =
                NixBackend::new(config(PathBuf::from(directory)), Arc::new(ExitDuringBuild))
                    .unwrap();
            let preview = backend
                .preview_theme(ThemeSettings {
                    accent_color: Some(peasy_core::AccentColor::Blue),
                    color_scheme: None,
                })
                .unwrap();
            let _ = backend.apply(&preview.change, &preview.before, &"a".repeat(48), &[]);
            panic!("fixture must terminate during build");
        }
        let temp = tempfile::tempdir().unwrap();
        let runtime = temp.path().join("state");
        let result = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "nix_backend::tests::interrupted_build_is_recovered_after_process_exit",
            ])
            .env(FIXTURE, &runtime)
            .output()
            .unwrap();
        assert_eq!(
            result.status.code(),
            Some(93),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let configuration = config(runtime);
        assert!(
            state::load_managed(&configuration.managed_module)
                .unwrap()
                .theme
                .accent_color
                .is_some()
        );
        assert!(
            recovery::load(&configuration.managed_module)
                .unwrap()
                .is_some()
        );
        let backend = NixBackend::new(
            configuration,
            Arc::new(MockRunner {
                outputs: Mutex::new(VecDeque::new()),
                calls: Mutex::new(vec![]),
            }),
        )
        .unwrap();
        assert_eq!(backend.current_state().unwrap(), PackageState::default());
        let notice = backend.inspect().unwrap().recovery.unwrap();
        assert!(!notice.needs_attention);
        assert!(
            notice
                .intended_change
                .iter()
                .any(|line| line.text.contains("blue"))
        );
    }

    #[test]
    fn upgrade_drains_running_request_and_delivers_result_before_exit() {
        use peasy_core::{IpcRequest, IpcResponse};
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixStream;
        use std::sync::mpsc;
        struct Allow;
        impl crate::authorization::Authorizer for Allow {
            fn authorize(&self, _: &crate::authorization::Peer) -> Result<()> {
                Ok(())
            }
        }
        struct PausedBuild {
            started: mpsc::Sender<()>,
            resume: Mutex<mpsc::Receiver<()>>,
        }
        impl CommandRunner for PausedBuild {
            fn run(&self, _: &Path, args: &[OsString], _: Option<&Path>) -> Result<Output> {
                assert_eq!(args[0], "build");
                self.started.send(()).unwrap();
                self.resume
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(10))
                    .unwrap();
                Ok(output(1, "", "deliberate fixture failure"))
            }
        }
        fn connect(path: &Path, request: &IpcRequest) -> BufReader<UnixStream> {
            let mut stream = UnixStream::connect(path).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            serde_json::to_writer(&mut stream, request).unwrap();
            stream.write_all(b"\n").unwrap();
            BufReader::new(stream)
        }
        fn response(stream: &mut BufReader<UnixStream>) -> IpcResponse {
            loop {
                let mut line = String::new();
                stream.read_line(&mut line).unwrap();
                let value: IpcResponse = serde_json::from_str(&line).unwrap();
                if !matches!(value, IpcResponse::Progress { .. }) {
                    return value;
                }
            }
        }
        let temp = tempfile::tempdir().unwrap();
        let mut configuration = config(temp.path().join("state"));
        let expected = temp.path().join("identity");
        fs::write(&expected, "old").unwrap();
        configuration.identity = Some(expected);
        let active = configuration
            .active_system
            .join("etc/peasy/daemon-identity.json");
        fs::create_dir_all(active.parent().unwrap()).unwrap();
        fs::write(&active, "old").unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        let backend = Arc::new(
            NixBackend::new(
                configuration,
                Arc::new(PausedBuild {
                    started: started_tx,
                    resume: Mutex::new(resume_rx),
                }),
            )
            .unwrap(),
        );
        let socket = temp.path().join("ipc");
        let server =
            crate::server::Server::new(socket.clone(), backend.clone(), Arc::new(Allow)).unwrap();
        let serving = std::thread::spawn(move || server.run());
        let start = std::time::Instant::now();
        while !socket.exists() && start.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(10));
        }
        let IpcResponse::Proposal { proposal } = response(&mut connect(
            &socket,
            &IpcRequest::ProposeTheme {
                theme: ThemeSettings {
                    accent_color: Some(peasy_core::AccentColor::Blue),
                    color_scheme: None,
                },
            },
        )) else {
            panic!("expected proposal")
        };
        let mut applying = connect(
            &socket,
            &IpcRequest::ApplyWithProgress {
                proposal: proposal.id,
            },
        );
        started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        fs::write(active, "new").unwrap();
        assert!(matches!(
            response(&mut connect(&socket, &IpcRequest::Status)),
            IpcResponse::Error { .. }
        ));
        assert!(!serving.is_finished());
        resume_tx.send(()).unwrap();
        assert!(matches!(
            response(&mut applying),
            IpcResponse::Applied { .. }
        ));
        serving.join().unwrap().unwrap();
        assert_eq!(backend.current_state().unwrap(), PackageState::default());
        assert!(
            recovery::load(&backend.config.managed_module)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn active_generation_identity_requests_restart_only_when_changed() {
        let temp = tempfile::tempdir().unwrap();
        let mut config = config(temp.path().join("state"));
        let expected = temp.path().join("identity");
        fs::write(&expected, "old").unwrap();
        config.identity = Some(expected);
        fs::create_dir_all(config.active_system.join("etc/peasy")).unwrap();
        let active = config.active_system.join("etc/peasy/daemon-identity.json");
        fs::write(&active, "old").unwrap();
        let backend = NixBackend::new(
            config,
            Arc::new(MockRunner {
                outputs: Mutex::new(VecDeque::new()),
                calls: Mutex::new(vec![]),
            }),
        )
        .unwrap();
        assert!(!backend.should_restart());
        fs::write(&active, "new").unwrap();
        assert!(backend.should_restart());
        assert!(backend.inspect().unwrap().restart_pending);
    }

    #[test]
    fn recovery_never_overlaps_an_active_or_unknown_activation_service() {
        for service in [
            output(0, "activating\n", ""),
            output(0, "deactivating\n", ""),
            output(1, "", "cannot check"),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let runner = Arc::new(MockRunner {
                outputs: Mutex::new(VecDeque::from([service])),
                calls: Mutex::new(vec![]),
            });
            let backend =
                NixBackend::new(config(temp.path().join("state")), runner.clone()).unwrap();
            let before = backend.current_state().unwrap();
            let generation = "/nix/store/00000000000000000000000000000000-old-system";
            let journal = recovery::Journal {
                before: before.clone(),
                proposed: before.clone(),
                previous_generation: Some(generation.into()),
                target_generation: None,
                phase: recovery::Phase::Activating,
            };
            recovery::write(&recovery::path(&backend.config.managed_module), &journal).unwrap();
            let error = backend
                .apply(
                    &ProposalChange::Recovery {
                        generation: generation.into(),
                    },
                    &before,
                    &"a".repeat(48),
                    &[],
                )
                .unwrap_err();
            assert!(!error.to_string().is_empty());
            assert_eq!(runner.calls.lock().unwrap().len(), 1);
            assert!(
                recovery::load(&backend.config.managed_module)
                    .unwrap()
                    .is_some()
            );
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
            .scope(|| backend.apply(&preview.change, &preview.before, &id, &preview.packages))
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
            identity: None,
            active_system: runtime_dir.join("active-system"),
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
                .apply(
                    &preview.change,
                    &preview.before,
                    "policy-changed",
                    &preview.packages
                )
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
    fn verification_uses_host_package_set_and_refreshes_identity() {
        let temporary = tempfile::tempdir().unwrap();
        let runner = Arc::new(MockRunner {
            outputs: Mutex::new(VecDeque::from([
                identity_output(&["hello"]),
                identity_output(&["hello"]),
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
            assert!(expression.contains("attrByPath"));
            assert!(expression.contains("evaluated"));
            assert!(expression.contains("drv_path"));
            assert!(!expression.contains("path:"));
        }
    }

    #[test]
    fn failed_configuration_build_never_activates_or_commits() {
        let temporary = tempfile::tempdir().unwrap();
        let runner = Arc::new(MockRunner {
            outputs: Mutex::new(VecDeque::from([
                identity_output(&["telegram-desktop"]),
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
            .apply(
                &preview.change,
                &preview.before,
                &"a".repeat(48),
                &preview.packages,
            )
            .unwrap();
        assert!(!result.activated);
        assert!(backend.packages().unwrap().is_empty());
        assert_eq!(backend.current_state().unwrap(), PackageState::default());
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1][0], "build");
        assert!(calls[1].iter().any(|argument| argument == "--file"));
        assert!(!calls[1].iter().any(|argument| argument == "--flake"));
    }

    #[test]
    fn setup_build_failure_restores_every_contribution() {
        let temporary = tempfile::tempdir().unwrap();
        let runner = Arc::new(MockRunner {
            outputs: Mutex::new(VecDeque::from([
                identity_output(&["hello"]),
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
            .apply(
                &preview.change,
                &preview.before,
                &"e".repeat(48),
                &preview.packages,
            )
            .unwrap();
        assert!(!result.activated);
        assert_eq!(backend.current_state().unwrap(), preview.before);
        assert_eq!(runner.calls.lock().unwrap().len(), 2);
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
            .apply(
                &preview.change,
                &preview.before,
                &"f".repeat(48),
                &preview.packages,
            )
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
                &[],
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
            &[],
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
                    &[],
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
        assert_eq!(calls[0][calls[0].len() - 2], "");
        assert!(!calls[0].iter().any(|arg| arg == "--option"));
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
        let compatible = [
            "whatsapp-electron",
            "whatsapp-emoji-font",
            "whatsapp-chat-exporter",
            "altus",
        ];
        let runner = Arc::new(MockRunner {
            outputs: Mutex::new(VecDeque::from([
                output(0, &search.to_string(), ""),
                identity_output(&compatible),
                identity_output(&["whatsapp-electron"]),
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
            3,
            "searches use the cache, but review must refresh the derivation"
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

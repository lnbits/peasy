//! Provider bootstrap uses typed, reviewed system changes and needs no model.
use crate::ollama_models::{InstalledModel, list_ollama_model_details};
use anyhow::{Context, Result, bail};
use peasy_core::resources::{ManagedService, ServiceAction};
use peasy_core::{IpcRequest, ResourceChange};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetupAction {
    Install,
    Start,
}

impl SetupAction {
    pub fn request(self) -> IpcRequest {
        IpcRequest::ProposeResources {
            change: match self {
                Self::Install => ResourceChange::ServiceEnabled {
                    service: ManagedService::Ollama,
                    enabled: true,
                },
                Self::Start => ResourceChange::Service {
                    unit: "ollama.service".into(),
                    action: ServiceAction::Start,
                },
            },
        }
    }
}

/// Only an actual refused local TCP connection warrants service discovery.
/// HTTP errors, malformed inventories, timeouts and cancellations are not evidence
/// that Ollama needs installing. Preserve the error chain until this check.
pub fn connection_refused(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|error| error.kind() == std::io::ErrorKind::ConnectionRefused)
    })
}

#[derive(Debug)]
pub struct SetupRequired(pub SetupAction);
impl std::fmt::Display for SetupRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Ollama setup requires user approval")
    }
}
impl std::error::Error for SetupRequired {}

/// Called only when the user selects the local provider or presses Refresh.
/// A healthy API wins over process discovery, including manually started servers.
pub fn list_or_start_models(starting: impl FnOnce()) -> Result<Vec<InstalledModel>> {
    let url = crate::DEFAULT_OLLAMA_URL;
    match list_ollama_model_details(url) {
        Ok(models) => return Ok(models),
        Err(error) if connection_refused(&error) => {}
        Err(error) => return Err(error),
    }
    static START: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let _guard = loop {
        crate::Cancellation::current().check()?;
        if let Ok(guard) = START.try_lock() {
            break guard;
        }
        if std::time::Instant::now() >= deadline {
            bail!("Ollama startup is already in progress; refresh shortly");
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    };
    // Another settings window may have started it while we waited.
    match list_ollama_model_details(url) {
        Ok(models) => return Ok(models),
        Err(error) if connection_refused(&error) => {}
        Err(error) => return Err(error),
    }
    starting();
    if let Some(action) = discover_and_start(&LocalRuntime)? {
        return Err(SetupRequired(action).into());
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        crate::Cancellation::current().check()?;
        match list_ollama_model_details(url) {
            Ok(models) => return Ok(models),
            Err(error) if connection_refused(&error) && std::time::Instant::now() < deadline => {}
            Err(error) => {
                return Err(
                    error.context("Ollama did not become ready; check its service and refresh")
                );
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

trait Runtime {
    fn state(&self, user: bool, unit: &str) -> Result<Value>;
    fn start(&self, user: bool, unit: &str) -> Result<()>;
    fn binary(&self) -> Option<std::path::PathBuf>;
    fn launch(&self, binary: &std::path::Path) -> Result<()>;
}

fn discover_and_start(runtime: &dyn Runtime) -> Result<Option<SetupAction>> {
    let system = runtime.state(false, "ollama.service")?;
    match action_for_service(&system)? {
        Some(SetupAction::Start) => {
            // No password prompting or privilege bypass. If systemd requires
            // authorization, retain Peasy's reviewed, authenticated start action.
            if runtime.start(false, "ollama.service").is_err() {
                crate::Cancellation::current().check()?;
                return Ok(Some(SetupAction::Start));
            }
            return Ok(None);
        }
        None => return Ok(None),
        Some(SetupAction::Install) => {}
    }
    for unit in ["ollama.service", "peasy-ollama.service"] {
        let mut state = runtime.state(true, unit)?;
        // Use the same strict state rules for the fixed Peasy-owned user unit.
        if state["Id"] == unit {
            state["Id"] = Value::String("ollama.service".into());
        }
        match action_for_service(&state)? {
            Some(SetupAction::Start) => {
                runtime.start(true, unit)?;
                return Ok(None);
            }
            None => return Ok(None),
            Some(SetupAction::Install) => {}
        }
    }
    if let Some(binary) = runtime.binary() {
        runtime.launch(&binary)?;
        Ok(None)
    } else {
        Ok(Some(SetupAction::Install))
    }
}

struct LocalRuntime;
impl Runtime for LocalRuntime {
    fn state(&self, user: bool, unit: &str) -> Result<Value> {
        let mut command = systemctl()?;
        if user {
            command.arg("--user");
        }
        command.args([
            "show",
            "--no-pager",
            "--property=Id,LoadState,ActiveState",
            "--",
            unit,
        ]);
        let output = checked(&mut command)?;
        let fields: serde_json::Map<String, Value> = output
            .lines()
            .filter_map(|line| line.split_once('='))
            .map(|(key, value)| (key.into(), Value::String(value.into())))
            .collect();
        Ok(Value::Object(fields))
    }
    fn start(&self, user: bool, unit: &str) -> Result<()> {
        let mut command = systemctl()?;
        if user {
            command.arg("--user");
        }
        checked(command.args(["--no-ask-password", "start", "--", unit]))?;
        Ok(())
    }
    fn binary(&self) -> Option<std::path::PathBuf> {
        let mut directories: Vec<_> = std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).collect())
            .unwrap_or_default();
        // Desktop sessions need not include nix profile paths in PATH.
        if let Some(home) = std::env::var_os("HOME") {
            let home = std::path::PathBuf::from(home);
            directories.push(home.join(".nix-profile/bin"));
            directories.push(home.join(".local/state/nix/profile/bin"));
        }
        directories.push("/run/current-system/sw/bin".into());
        find_binary(&directories)
    }
    fn launch(&self, binary: &std::path::Path) -> Result<()> {
        let runner = std::env::var_os("PEASY_SYSTEMD_RUN")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| "/run/current-system/sw/bin/systemd-run".into());
        if !runner.is_absolute() {
            bail!("systemd-run path must be absolute");
        }
        let mut command = std::process::Command::new(runner);
        command
            .args([
                "--user",
                "--quiet",
                "--collect",
                "--unit=peasy-ollama.service",
                "--service-type=exec",
                "--setenv=OLLAMA_HOST=127.0.0.1:11434",
                "--setenv=OLLAMA_KEEP_ALIVE=2m",
                "--setenv=OLLAMA_NUM_PARALLEL=1",
                "--",
            ])
            .arg(binary)
            .arg("serve");
        if let Err(error) = checked(&mut command) {
            // Another Peasy process may have won the fixed-unit startup race.
            crate::Cancellation::current().check()?;
            let state = self.state(true, "peasy-ollama.service")?;
            if state["LoadState"] != "loaded"
                || !matches!(state["ActiveState"].as_str(), Some("active" | "activating"))
            {
                return Err(error);
            }
        }
        Ok(())
    }
}

fn find_binary(directories: &[std::path::PathBuf]) -> Option<std::path::PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    directories
        .iter()
        .filter(|path| path.is_absolute())
        .find_map(|directory| {
            let path = directory.join("ollama");
            let metadata = path.metadata().ok()?;
            (metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
                .then(|| path.canonicalize().ok())
                .flatten()
        })
}

fn systemctl() -> Result<std::process::Command> {
    Ok(std::process::Command::new(
        peasy_core::resource_native::Tool::Systemctl.path()?,
    ))
}
fn checked(command: &mut std::process::Command) -> Result<String> {
    let output = peasy_core::process::run(
        command.env("LC_ALL", "C"),
        std::time::Duration::from_secs(20),
    )?;
    if !output.status.success() {
        bail!(
            "Could not start or inspect local Ollama: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    String::from_utf8(output.stdout).context("invalid service status")
}

fn action_for_service(service: &Value) -> Result<Option<SetupAction>> {
    match service["LoadState"].as_str() {
        Some("not-found") => Ok(Some(SetupAction::Install)),
        Some("loaded") if service["Id"] == "ollama.service" => {
            match service["ActiveState"].as_str() {
                Some("inactive" | "failed") => Ok(Some(SetupAction::Start)),
                Some("active" | "activating" | "deactivating" | "reloading") => Ok(None),
                _ => bail!("unknown Ollama service state"),
            }
        }
        Some("masked") => {
            bail!("Ollama is masked by system configuration; ask the administrator to unmask it")
        }
        _ => bail!("could not determine Ollama service availability"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct FakeRuntime {
        states: std::cell::RefCell<std::collections::VecDeque<Value>>,
        calls: std::cell::RefCell<Vec<String>>,
        binary: Option<std::path::PathBuf>,
        deny_start: bool,
    }
    impl Runtime for FakeRuntime {
        fn state(&self, user: bool, unit: &str) -> Result<Value> {
            self.calls
                .borrow_mut()
                .push(format!("inspect {user} {unit}"));
            Ok(self
                .states
                .borrow_mut()
                .pop_front()
                .expect("unexpected service lookup"))
        }
        fn start(&self, user: bool, unit: &str) -> Result<()> {
            self.calls.borrow_mut().push(format!("start {user} {unit}"));
            if self.deny_start {
                bail!("authorization needed");
            }
            Ok(())
        }
        fn binary(&self) -> Option<std::path::PathBuf> {
            self.calls.borrow_mut().push("find binary".into());
            self.binary.clone()
        }
        fn launch(&self, binary: &std::path::Path) -> Result<()> {
            self.calls
                .borrow_mut()
                .push(format!("launch {}", binary.display()));
            Ok(())
        }
    }
    fn service(active: &str) -> Value {
        json!({"Id":"ollama.service", "LoadState":"loaded", "ActiveState":active})
    }
    fn runtime(states: Vec<Value>, installed: bool, deny_start: bool) -> FakeRuntime {
        FakeRuntime {
            states: std::cell::RefCell::new(states.into()),
            calls: Default::default(),
            binary: installed.then(|| "/nix/store/example/bin/ollama".into()),
            deny_start,
        }
    }
    #[test]
    fn installed_services_start_without_creating_a_second_server() {
        for active in ["active", "activating", "inactive", "failed"] {
            let r = runtime(vec![service(active)], true, false);
            assert_eq!(discover_and_start(&r).unwrap(), None);
            let expected = if matches!(active, "inactive" | "failed") {
                2
            } else {
                1
            };
            assert_eq!(r.calls.borrow().len(), expected);
            assert!(
                !r.calls
                    .borrow()
                    .iter()
                    .any(|call| call.starts_with("launch"))
            );
        }
        let r = runtime(vec![service("inactive")], true, true);
        assert_eq!(discover_and_start(&r).unwrap(), Some(SetupAction::Start));
        assert_eq!(r.calls.borrow().len(), 2); // no root workaround or second daemon
    }
    #[test]
    fn standalone_installation_and_user_services_are_recognised() {
        let missing = json!({"LoadState":"not-found"});
        let r = runtime(vec![missing.clone(), service("inactive")], true, false);
        assert_eq!(discover_and_start(&r).unwrap(), None);
        assert_eq!(
            r.calls.borrow().last().unwrap(),
            "start true ollama.service"
        );
        let r = runtime(
            vec![missing.clone(), missing.clone(), missing.clone()],
            true,
            false,
        );
        assert_eq!(discover_and_start(&r).unwrap(), None);
        assert_eq!(
            r.calls.borrow().last().unwrap(),
            "launch /nix/store/example/bin/ollama"
        );
        let r = runtime(
            vec![
                missing.clone(),
                missing.clone(),
                json!({"Id":"peasy-ollama.service","LoadState":"loaded","ActiveState":"active"}),
            ],
            true,
            false,
        );
        assert_eq!(discover_and_start(&r).unwrap(), None);
        assert_eq!(r.calls.borrow().len(), 3);
        let r = runtime(
            vec![missing.clone(), missing.clone(), missing],
            false,
            false,
        );
        assert_eq!(discover_and_start(&r).unwrap(), Some(SetupAction::Install));
        assert!(
            !r.calls
                .borrow()
                .iter()
                .any(|call| call.starts_with("start") || call.starts_with("launch"))
        );
    }
    #[test]
    fn masked_or_unknown_service_does_not_trigger_install_or_launch() {
        for state in [json!({"LoadState":"masked"}), json!({}), service("unknown")] {
            let r = runtime(vec![state], true, false);
            assert!(discover_and_start(&r).is_err());
            assert_eq!(r.calls.borrow().len(), 1);
        }
    }
    #[test]
    fn binary_discovery_requires_an_executable_file_and_absolute_directory() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let binary = temp.path().join("ollama");
        let dirs = [std::path::PathBuf::from("."), temp.path().to_path_buf()];
        assert!(find_binary(&dirs).is_none());
        std::fs::write(&binary, "test fixture").unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(find_binary(&dirs).is_none());
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(find_binary(&dirs), Some(binary.canonicalize().unwrap()));
    }

    #[test]
    fn setup_requires_confirmed_missing_or_stopped_service() {
        assert_eq!(
            action_for_service(&json!({"LoadState":"not-found"})).unwrap(),
            Some(SetupAction::Install)
        );
        for active in [
            "inactive",
            "failed",
            "active",
            "activating",
            "deactivating",
            "reloading",
        ] {
            assert_eq!(
                action_for_service(
                    &json!({"Id":"ollama.service", "LoadState":"loaded", "ActiveState":active})
                )
                .unwrap(),
                if matches!(active, "inactive" | "failed") {
                    Some(SetupAction::Start)
                } else {
                    None
                }
            );
        }
        for invalid in [
            json!({}),
            json!({"LoadState":"masked"}),
            json!({"Id":"other.service", "LoadState":"loaded", "ActiveState":"inactive"}),
        ] {
            assert!(action_for_service(&invalid).is_err());
        }
        assert!(matches!(
            SetupAction::Start.request(),
            IpcRequest::ProposeResources {
                change: ResourceChange::Service {
                    action: ServiceAction::Start,
                    ..
                }
            }
        ));
        assert!(matches!(
            SetupAction::Install.request(),
            IpcRequest::ProposeResources {
                change: ResourceChange::ServiceEnabled {
                    service: ManagedService::Ollama,
                    enabled: true
                }
            }
        ));
    }

    #[test]
    fn only_refused_connections_trigger_discovery() {
        for kind in [
            std::io::ErrorKind::ConnectionRefused,
            std::io::ErrorKind::TimedOut,
            std::io::ErrorKind::PermissionDenied,
        ] {
            let error = anyhow::Error::from(std::io::Error::from(kind))
                .context("connecting to local Ollama");
            assert_eq!(
                connection_refused(&error),
                kind == std::io::ErrorKind::ConnectionRefused
            );
        }
        assert!(!connection_refused(&anyhow::anyhow!(
            "HTTP 500: connection refused"
        )));
        assert!(!connection_refused(&anyhow::anyhow!(
            "Ollama returned invalid JSON"
        )));
        // Exercise the real HTTP error chain, not just a constructed io::Error.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let error = crate::ollama_models::list_ollama_model_details(&format!("http://{address}"))
            .unwrap_err();
        assert!(connection_refused(&error), "{error:#}");
    }
}

//! Model-visible generic configuration catalogue and privileged proposal bridge.
use crate::{PeasyClient, Resolution};
use anyhow::{Result, bail};
use peasy_core::{IpcRequest, IpcResponse, SYSTEM_ENABLE_OPTIONS, SYSTEM_GROUPS, SystemSetup};
use serde_json::{Value, json};

pub(super) fn instructions() -> String {
    format!(
        "System-configuration pea: an installation must include the NixOS integration needed to make the application usable, not just its executable. For install_package, set setup to null for a standalone application, or an object containing packages (up to 8 supporting Nixpkgs attributes), enable (reviewed boolean NixOS options to enable), and groups (access for the requesting user). Infer requirements from the user's goal and your NixOS knowledge, not from keyword recipes. The primary package must still be an exact search candidate. Supporting attributes are separately checked against pinned Nixpkgs by the daemon; do not guess uncertain names. Only these enable options exist: {:?}. Group/required-option pairs: {:?}. NixOS modules supply their own dependencies: do not add packages already provided by an enabled module unnecessarily. libvirtd enables the local VM management service; programs.virt-manager enables desktop integration; libvirtd group grants powerful VM management access. Printing enables local CUPS, sane enables scanners, bluetooth enables Bluetooth support. Include only necessary settings, never expose network listeners or broaden access unrelated to the request. No arbitrary configuration keys, shell, file contents or account names are accepted. An existing setup is replaced in full, so retain its still-needed contributions when updating it. Review and administrator approval are mandatory. Group changes require logging out and back in. If the catalogue cannot express required setup, explain the limitation instead of claiming the application will work. Removal withdraws Peasy's setup contributions only; administrator settings and VM/user data are retained.",
        SYSTEM_ENABLE_OPTIONS, SYSTEM_GROUPS
    )
}

pub(super) fn schema() -> Value {
    json!({
        "type": ["object", "null"], "additionalProperties": false,
        "properties": {
            "packages": {"type": "array", "maxItems": 8, "items": {"type": "string", "maxLength": peasy_core::MAX_ATTRIBUTE_BYTES}},
            "enable": {"type": "array", "maxItems": 8, "items": {"type": "string", "enum": SYSTEM_ENABLE_OPTIONS}},
            "groups": {"type": "array", "maxItems": 4, "items": {"type": "string", "enum": SYSTEM_GROUPS.iter().map(|(group, _)| *group).collect::<std::collections::BTreeSet<_>>()}}
        },
        "required": ["packages", "enable", "groups"]
    })
}

impl PeasyClient {
    // A fallback search choice has not yet been assessed as an installation.
    // Evaluate that exact selection once so this path cannot skip integration.
    pub(super) fn propose_selected_package(
        &self,
        candidate: peasy_core::PackageCandidate,
    ) -> Result<Resolution> {
        let installed = match self.ipc.request(&IpcRequest::GetPackages)? {
            IpcResponse::Packages { packages } => packages,
            _ => bail!("unexpected response to GetPackages"),
        };
        let module = match self.ipc.request(&IpcRequest::GetManagedModule) {
            Ok(IpcResponse::ManagedModule { module }) => module,
            Err(error) if error.to_string().contains("invalid typed IPC request") => {
                "# Peasy-managed configuration is unavailable from this system service version."
                    .into()
            }
            Ok(_) => bail!("unexpected response to GetManagedModule"),
            Err(error) => return Err(error),
        };
        let theme = match self.ipc.request(&IpcRequest::GetTheme)? {
            IpcResponse::Theme { theme } => theme,
            _ => bail!("unexpected response to GetTheme"),
        };
        let action = self.model.interpret(
            &format!("Install the selected package `{}` with any required supported system integration. Do not substitute another package.", candidate.attribute),
            &module, Some(std::slice::from_ref(&candidate)), Some(&installed), &theme, None,
        )?;
        let decision = self.engine.resolve(&peasy_core::EngineInput {
            action,
            candidates: vec![candidate.clone()],
            installed,
        })?;
        match decision {
            peasy_core::EngineDecision::Install {
                setup: Some(setup), ..
            } => self.propose_setup_candidate(candidate, setup),
            peasy_core::EngineDecision::Install { setup: None, .. } => {
                self.propose_candidate(candidate)
            }
            peasy_core::EngineDecision::Explain(message) => Ok(Resolution::Explain(message)),
            peasy_core::EngineDecision::Cancel => Ok(Resolution::Cancel),
            _ => bail!("could not prepare the selected package safely; no change was made"),
        }
    }

    pub(super) fn propose_setup(&self, package: String, setup: SystemSetup) -> Result<Resolution> {
        setup.validate()?;
        match self
            .ipc
            .request(&IpcRequest::ProposeSetup { package, setup })?
        {
            IpcResponse::Proposal { proposal } => Ok(Resolution::Proposal(proposal)),
            _ => bail!("unexpected response to ProposeSetup"),
        }
    }

    pub(super) fn propose_setup_candidate(
        &self,
        candidate: peasy_core::PackageCandidate,
        setup: SystemSetup,
    ) -> Result<Resolution> {
        let package = candidate.attribute.clone();
        *self
            .recent_package
            .lock()
            .expect("recent package mutex poisoned") = Some(candidate);
        self.propose_setup(package, setup)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;

    #[test]
    #[ignore = "requires PEASY_TEST_ENGINE; packaged Nix checks provide the compiled guest"]
    fn setup_selection_retains_the_exact_candidate_for_follow_up_uninstall() {
        let temp = tempfile::tempdir().unwrap();
        let socket = temp.path().join("ipc.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let setup: peasy_core::ManagedSetup =
            serde_json::from_str(include_str!("example.json")).unwrap();
        let expected = setup.settings.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let request: IpcRequest = serde_json::from_str(&line).unwrap();
            assert_eq!(
                request,
                IpcRequest::ProposeSetup {
                    package: "virt-manager".into(),
                    setup: expected
                }
            );
            let proposal = peasy_core::Proposal {
                packages: vec![],
                id: "a".repeat(48),
                title: "Set up virt-manager".into(),
                diff: vec![],
                change: peasy_core::ProposalChange::Setup {
                    operation: peasy_core::PackageOperation::Install,
                    setup,
                },
            };
            serde_json::to_writer(
                &mut stream,
                &IpcResponse::Proposal {
                    proposal: Box::new(proposal),
                },
            )
            .unwrap();
            stream.write_all(b"\n").unwrap();
        });
        let engine = std::env::var_os("PEASY_TEST_ENGINE").expect("built Wasm path");
        let client = PeasyClient::with_provider(
            socket,
            std::path::Path::new(&engine),
            crate::ModelProvider::Ollama {
                base_url: "http://127.0.0.1:11434".into(),
                model: "unused-test-model".into(),
            },
        )
        .unwrap();
        let candidate = peasy_core::PackageCandidate {
            attribute: "virt-manager".into(),
            name: "Virtual Machine Manager".into(),
            description: "VM management".into(),
            version: "5.0.0".into(),
        };
        let setup: peasy_core::ManagedSetup =
            serde_json::from_str(include_str!("example.json")).unwrap();
        assert!(matches!(
            client
                .propose_setup_candidate(candidate.clone(), setup.settings)
                .unwrap(),
            Resolution::Proposal(_)
        ));
        assert_eq!(
            client.recent_package.lock().unwrap().as_ref(),
            Some(&candidate)
        );
        server.join().unwrap();
    }
}

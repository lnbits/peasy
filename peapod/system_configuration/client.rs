//! Model-visible generic configuration catalogue and privileged proposal bridge.
use crate::{PeasyClient, Resolution};
use anyhow::{Result, bail};
use peasy_core::{IpcRequest, IpcResponse, SYSTEM_ENABLE_OPTIONS, SYSTEM_GROUPS, SystemSetup};

pub(super) fn instructions() -> String {
    let mut instructions = format!(
        "System-configuration pea: an installation must include the NixOS integration needed to make the application usable, not just its executable. For install_package, set setup to null for a standalone application, or an object containing packages (up to 8 supporting Nixpkgs attributes), enable (reviewed boolean NixOS options to enable), and groups (access for the requesting user). Infer requirements from the user's goal and your NixOS knowledge, not from keyword recipes. The primary package must still be a host-verified attribute, either from search or direct verification. Supporting attributes are separately checked against pinned Nixpkgs by the daemon; do not guess uncertain names. Reviewed options and their effects: {:?}. Group/required-option pairs (empty prerequisite means a standard NixOS device group): {:?}. NixOS modules supply their own dependencies: do not add packages already provided by an enabled module unnecessarily. libvirtd enables the local VM management service; programs.virt-manager enables desktop integration; libvirtd group grants powerful VM management access. Printing enables local CUPS, sane enables scanners, bluetooth enables Bluetooth support. Include only necessary settings, never expose remote network listeners or broaden access unrelated to the request. No arbitrary configuration keys, shell, file contents or account names are accepted. An existing setup is replaced in full, so retain its still-needed contributions when updating it. Review and administrator approval are mandatory. Group changes require logging out and back in. If the catalogue cannot express required setup, provide manual steps instead of claiming the application will work. Removal withdraws Peasy's setup contributions only; administrator settings and VM/user data are retained.",
        SYSTEM_ENABLE_OPTIONS, SYSTEM_GROUPS
    );
    instructions.push_str(" Typed PostgreSQL setup is available through setup.postgresql: {package: a versioned PostgreSQL attribute, caller_database: boolean}. Use null when not needed. This enables a local server at boot and pins its major version. Set caller_database=true when a local development database for the requesting user is wanted; the daemon binds its name and ownership, with peer authentication and no superuser privileges. Supported attributes are postgresql_14 through postgresql_18, subject to availability on the host. Select a supported stable version from search results and keep an existing Peasy server's version. Never request an automatic major-version upgrade. Use system_profile.postgresql to recognise existing servers; do not take over administrator-managed PostgreSQL. For an ambiguous request such as install PostgreSQL, use explain to ask whether a local server or tools only are wanted. For an explicit local service or development database request, propose the complete setup immediately. For explicit client/tools-only requests, leave postgresql null. Remote access, passwords, arbitrary database names, role grants and migrations are unsupported; explain that clearly instead of proposing a partial install. Do not add a second PostgreSQL package when its module already supplies the selected server. Configuration rollback and removal retain database contents and roles; neither undoes SQL changes.");
    instructions.push_str(include_str!("manual-guidance.txt"));
    instructions
}

impl PeasyClient {
    // A fallback search choice has not yet been assessed as an installation.
    // Evaluate that exact selection once so this path cannot skip integration.
    pub(super) fn propose_selected_package(
        &self,
        candidate: peasy_core::PackageCandidate,
        request: &str,
        pea: Option<&peasy_core::pea::PeaManifest>,
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
        let answer = self.model.interpret(
            &format!("Original request: {request}\nSelected package: `{}`. Prepare the installation consistent with the original request, including supported service setup when requested. Preserve tools-only intent. Give concrete numbered manual steps for unsupported requirements or ask whether a local server or tools only are wanted when ambiguous. Do not substitute another package.", candidate.attribute),
            &module, Some(std::slice::from_ref(&candidate)), Some(&installed), &theme, None, pea,
        )?;
        let decision = self.engine.resolve(&peasy_core::EngineInput {
            action: answer.action,
            candidates: vec![candidate.clone()],
            installed,
        })?;
        match decision {
            peasy_core::EngineDecision::Install { setup, message, .. } => {
                let resolution = match setup {
                    Some(setup) => self.propose_setup_candidate(candidate, setup)?,
                    None => self.propose_candidate(candidate)?,
                };
                Ok(crate::packages::with_install_guidance(
                    resolution,
                    message.as_deref(),
                ))
            }
            peasy_core::EngineDecision::Explain(message) => {
                Ok(Resolution::explanation(message, answer.needs_reply))
            }
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
    #[ignore = "requires PEASY_TEST_ENGINE; packaged checks provide the Wasm guest"]
    fn service_clarification_keeps_the_original_request_for_a_short_reply() {
        let temp = tempfile::tempdir().unwrap();
        let socket = temp.path().join("ipc.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = std::thread::spawn(move || {
            for _ in 0..6 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut line = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut line)
                    .unwrap();
                let response = match serde_json::from_str::<IpcRequest>(&line).unwrap() {
                    IpcRequest::GetPackages => IpcResponse::Packages { packages: vec![] },
                    IpcRequest::GetTheme => IpcResponse::Theme {
                        theme: peasy_core::ThemeSettings::default(),
                    },
                    IpcRequest::GetManagedModule => IpcResponse::ManagedModule {
                        module: peasy_core::render_packages_module(
                            &peasy_core::PackageState::default(),
                        )
                        .unwrap(),
                    },
                    _ => panic!("clarification must not mutate the system"),
                };
                serde_json::to_writer(&mut stream, &response).unwrap();
                stream.write_all(b"\n").unwrap();
            }
        });
        let engine = std::env::var_os("PEASY_TEST_ENGINE").unwrap();
        let response = serde_json::json!({"message":{"role":"assistant","content":serde_json::json!({"result":{"action":"request_clarification","message":"Local server or tools only?"}}).to_string()},"done":true});
        let (url, first) = crate::tests::serve_ollama_once(response.clone());
        let mut client = PeasyClient::with_provider(
            socket,
            std::path::Path::new(&engine),
            crate::ModelProvider::Ollama {
                base_url: url,
                model: "test".into(),
            },
        )
        .unwrap();
        assert!(matches!(
            client.resolve("install PostgreSQL").unwrap(),
            Resolution::Clarify(_)
        ));
        first
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        let (url, second) = crate::tests::serve_ollama_once(response);
        client.model = crate::ModelBackend::new(crate::ModelProvider::Ollama {
            base_url: url,
            model: "test".into(),
        })
        .unwrap();
        client.resolve("local server").unwrap();
        let (_, body) = second
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        let body = body.to_string();
        assert!(
            body.contains(
                "Previous user request (context only for a follow-up): install PostgreSQL"
            )
        );
        assert!(body.contains("Current user request: local server"));
        assert!(body.contains("Peasy asked: Local server or tools only?"));
        server.join().unwrap();
    }

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
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            assert_eq!(
                serde_json::from_str::<IpcRequest>(&line).unwrap(),
                IpcRequest::ApplyWithProgress {
                    proposal: "a".repeat(48)
                }
            );
            serde_json::to_writer(
                &mut stream,
                &IpcResponse::Applied {
                    result: peasy_core::ApplyResult {
                        configuration_valid: true,
                        build_successful: true,
                        activated: true,
                        message: "Setup applied.".into(),
                    },
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
        let guidance = "1. Create a guest VM after applying. 2. Select its storage and installation media in the application; Peasy has not created a VM.";
        let choice = crate::Choice {
            pea: None,
            intro: Some(guidance.into()),
            candidates: vec![crate::ChoiceItem {
                name: candidate.name.clone(),
                attribute: candidate.attribute.clone(),
                description: candidate.description.clone(),
                version: candidate.version.clone(),
                source: crate::ChoiceSource::SystemSetup {
                    candidate: candidate.clone(),
                    setup: setup.settings,
                },
            }],
        };
        let Resolution::Proposal(proposal) = client.select(choice, 0).unwrap() else {
            panic!("expected setup proposal")
        };
        assert!(
            proposal
                .diff
                .iter()
                .any(|line| line.text.contains(guidance))
        );
        assert!(
            proposal
                .diff
                .iter()
                .any(|line| line.text.contains("not applied by Peasy"))
        );
        let result = client.apply(&proposal).unwrap();
        assert!(result.message.contains(guidance));
        assert!(result.message.starts_with("Setup applied."));
        assert_eq!(
            client.recent_package.lock().unwrap().as_ref(),
            Some(&candidate)
        );
        server.join().unwrap();
    }
}

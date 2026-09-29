//! packages pea: unprivileged discovery, review and execution.
use crate::{Choice, ChoiceItem, ChoiceSource, PeasyClient, Resolution, ResolveStage};
use anyhow::{Context, Result, bail};
use peasy_core::{
    EngineDecision, EngineInput, IpcRequest, IpcResponse, PackageCandidate, RequestedVersion,
    ThemeSettings, validate_query,
};

const GUIDANCE_PREFIX: &str = "AI guidance (manual steps are not applied by Peasy):\n";

pub(super) fn append_install_guidance(
    proposal: &peasy_core::Proposal,
    result: &mut peasy_core::ApplyResult,
) {
    if result.activated
        && let Some(note) = proposal
            .diff
            .iter()
            .find(|line| line.text.starts_with(GUIDANCE_PREFIX))
    {
        result.message.push_str("\n\n");
        result.message.push_str(&note.text);
    }
}

/// Explanatory model text is displayed only; it never changes daemon-held effects.
pub(super) fn with_install_guidance(
    mut resolution: Resolution,
    message: Option<&str>,
) -> Resolution {
    if let (Resolution::Proposal(proposal), Some(message)) = (&mut resolution, message)
        && !message.trim().is_empty()
        && !proposal
            .diff
            .iter()
            .any(|line| line.text.starts_with(GUIDANCE_PREFIX))
    {
        proposal.diff.push(peasy_core::DiffLine {
            kind: peasy_core::DiffKind::Context,
            text: format!("{GUIDANCE_PREFIX}{message}"),
        });
    }
    resolution
}

pub(super) fn nix_choice(candidate: PackageCandidate, request: &str) -> ChoiceItem {
    ChoiceItem {
        name: candidate.name.clone(),
        attribute: candidate.attribute.clone(),
        description: candidate.description.clone(),
        version: candidate.version.clone(),
        source: ChoiceSource::Nixpkgs {
            candidate,
            request: request.into(),
        },
    }
}

pub(super) fn has_direct_package_match(candidates: &[PackageCandidate], query: &str) -> bool {
    let compact = |value: &str| {
        value
            .chars()
            .filter(|character| character.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    let query = compact(query);
    query.len() >= 3
        && candidates.iter().any(|candidate| {
            let attribute = compact(candidate.attribute.rsplit('.').next().unwrap_or_default());
            let name = compact(&candidate.name);
            attribute == query
                || name == query
                || attribute.starts_with(&query)
                || name.starts_with(&query)
        })
}

pub(super) fn package_choices(candidates: Vec<PackageCandidate>, request: &str) -> Resolution {
    Resolution::Choose(Choice {
        pea: None,
        intro: Some("I found these installable matches. Choose the one you want:".into()),
        candidates: candidates
            .into_iter()
            .map(|candidate| nix_choice(candidate, request))
            .collect(),
    })
}

impl PeasyClient {
    /// A suggested attribute is not a candidate until the host verifies it.
    pub(super) fn lookup_package_hint(&self, attribute: &str) -> Result<Option<PackageCandidate>> {
        peasy_core::validate_attribute(attribute)?;
        let candidates = match self.ipc.request(&IpcRequest::LookupPackage {
            attribute: attribute.to_owned(),
        }) {
            Ok(IpcResponse::SearchResults { candidates }) => candidates,
            // Keep discovery working with an older, still-running service.
            Err(e) if e.to_string().contains("invalid typed IPC request") => return Ok(None),
            Err(e) => return Err(e),
            _ => bail!("unexpected response to LookupPackage"),
        };
        if candidates.len() > 1 || candidates.iter().any(|p| p.attribute != attribute) {
            bail!("host returned a different package for an exact lookup");
        }
        Ok(candidates.into_iter().next())
    }

    fn search_packages(&self, query: &str, refresh: bool) -> Result<Vec<PackageCandidate>> {
        let query = validate_query(query)?.to_owned();
        let mut response = self.ipc.request(&IpcRequest::SearchPackages {
            query: query.clone(),
            refresh,
        });
        if refresh
            && response
                .as_ref()
                .is_err_and(|e| e.to_string().contains("invalid typed IPC request"))
        {
            response = self.ipc.request(&IpcRequest::SearchPackages {
                query,
                refresh: false,
            });
        }
        match response? {
            IpcResponse::SearchResults { candidates } => Ok(candidates),
            _ => bail!("unexpected response to SearchPackages"),
        }
    }

    #[allow(clippy::too_many_arguments)] // Explicit, bounded agent-loop context.
    pub(super) fn resolve_package_agent<F>(
        &self,
        request: &str,
        mut query: String,
        mut version: Option<RequestedVersion>,
        installed: &[String],
        theme: &ThemeSettings,
        managed_configuration: &str,
        pea: Option<&peasy_core::pea::PeaManifest>,
        progress: &mut F,
    ) -> Result<Resolution>
    where
        F: FnMut(ResolveStage),
    {
        for _ in 0..3 {
            progress(ResolveStage::SearchingPackages);
            let mut candidates = self.search_packages(&query, version.is_some())?;
            if let Some(RequestedVersion::Exact(requested)) = &version {
                candidates.retain(|candidate| {
                    !candidate.version.is_empty()
                        && RequestedVersion::Exact(requested.clone()).matches(&candidate.version)
                });
            }

            progress(ResolveStage::EvaluatingResults);
            let action = self.model.interpret(
                request,
                managed_configuration,
                Some(&candidates),
                Some(installed),
                theme,
                None,
                pea,
            )?;
            match self.engine.resolve(&EngineInput {
                action,
                candidates: candidates.clone(),
                installed: installed.to_vec(),
            })? {
                EngineDecision::Install {
                    package,
                    message,
                    setup,
                } => {
                    let candidate = candidates
                        .into_iter()
                        .find(|candidate| candidate.attribute == package)
                        .context("agent selected a missing package candidate")?;
                    if let Some(message) = message {
                        let mut item = nix_choice(candidate.clone(), request);
                        if let Some(setup) = setup {
                            item.source = ChoiceSource::SystemSetup { candidate, setup };
                        }
                        return Ok(Resolution::Choose(Choice {
                            pea: None,
                            intro: Some(message),
                            candidates: vec![item],
                        })
                        .with_pea(pea));
                    }
                    progress(ResolveStage::PreparingChange);
                    if let Some(setup) = setup {
                        return self.propose_setup_candidate(candidate, setup);
                    }
                    return self.propose_candidate(candidate);
                }
                EngineDecision::Search {
                    query: next_query,
                    version: next_version,
                } => {
                    if next_query.eq_ignore_ascii_case(&query) && next_version == version {
                        if has_direct_package_match(&candidates, &query) {
                            return Ok(package_choices(candidates, request).with_pea(pea));
                        }
                        break;
                    }
                    query = next_query;
                    version = next_version;
                }
                EngineDecision::SearchAppImage {
                    query,
                    version,
                    repository,
                } => {
                    progress(ResolveStage::SearchingAppImages);
                    return self
                        .search_appimages(&query, version.as_ref(), repository.as_deref())
                        .map(|r| r.with_pea(pea));
                }
                EngineDecision::Explain(message) => {
                    if let Some(candidate) = candidates.iter().find(|candidate| {
                        has_direct_package_match(std::slice::from_ref(candidate), &query)
                    }) {
                        *self
                            .recent_package
                            .lock()
                            .expect("recent package mutex poisoned") = Some(candidate.clone());
                    }
                    return Ok(Resolution::Explain(message));
                }
                EngineDecision::Cancel => return Ok(Resolution::Cancel),
                EngineDecision::Reject(message) => {
                    bail!("unsafe model decision rejected: {message}")
                }
                _ => bail!("the agent changed tasks while evaluating package results"),
            }
        }
        Ok(Resolution::Explain(
            "I couldn't identify a relevant installable package for that request. No change was made."
                .into(),
        ))
    }

    pub(super) fn propose_candidate(&self, candidate: PackageCandidate) -> Result<Resolution> {
        let attribute = candidate.attribute.clone();
        *self
            .recent_package
            .lock()
            .expect("recent package mutex poisoned") = Some(candidate);
        self.propose_install(&attribute)
    }

    pub(super) fn propose_install(&self, package: &str) -> Result<Resolution> {
        match self.ipc.request(&IpcRequest::ProposeInstall {
            package: package.to_owned(),
        })? {
            IpcResponse::Proposal { proposal } => Ok(Resolution::Proposal(proposal)),
            _ => bail!("unexpected response to ProposeInstall"),
        }
    }

    pub(super) fn propose_remove(&self, package: &str) -> Result<Resolution> {
        match self.ipc.request(&IpcRequest::ProposeRemove {
            package: package.to_owned(),
        })? {
            IpcResponse::Proposal { proposal } => Ok(Resolution::Proposal(proposal)),
            _ => bail!("unexpected response to ProposeRemove"),
        }
    }

    pub(super) fn check_package(&self, query: &str) -> Result<Resolution> {
        let candidates = self.search_packages(query, true)?;
        let Some(best) = candidates.first().cloned() else {
            return Ok(Resolution::Explain(format!(
                "I couldn't find a Nixpkgs package matching `{query}`."
            )));
        };
        *self
            .recent_package
            .lock()
            .expect("recent package mutex poisoned") = Some(best.clone());
        let alternatives = candidates
            .iter()
            .skip(1)
            .take(3)
            .map(|candidate| format!("• {} ({})", candidate.name, candidate.attribute))
            .collect::<Vec<_>>();
        let mut answer = format!(
            "Yes. The best Nixpkgs match is {} (`{}`).",
            best.name, best.attribute
        );
        if !best.description.is_empty() {
            answer.push_str(&format!("\n{}", best.description));
        }
        if !alternatives.is_empty() {
            answer.push_str("\n\nOther matches:\n");
            answer.push_str(&alternatives.join("\n"));
        }
        answer.push_str("\n\nSay “install it” to review that exact package.");
        Ok(Resolution::Explain(answer))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;

    fn candidate() -> PackageCandidate {
        PackageCandidate {
            attribute: "postgresql_17".into(),
            name: "postgresql".into(),
            version: "17.6".into(),
            description: "Database server and tools".into(),
        }
    }

    fn scripted_ipc(
        socket: &std::path::Path,
        exchanges: Vec<(IpcRequest, IpcResponse)>,
    ) -> std::thread::JoinHandle<()> {
        let listener = UnixListener::bind(socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        std::thread::spawn(move || {
            for (expected, response) in exchanges {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
                let (mut stream, _) = loop {
                    match listener.accept() {
                        Ok(connection) => break connection,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(std::time::Instant::now() < deadline, "missing {expected:?}");
                            std::thread::sleep(std::time::Duration::from_millis(10));
                        }
                        Err(e) => panic!("{e}"),
                    }
                };
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut line = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut line)
                    .unwrap();
                assert_eq!(serde_json::from_str::<IpcRequest>(&line).unwrap(), expected);
                serde_json::to_writer(&mut stream, &response).unwrap();
                stream.write_all(b"\n").unwrap();
            }
        })
    }

    fn fixture_client(socket: std::path::PathBuf, url: String) -> PeasyClient {
        PeasyClient::with_provider(
            socket,
            std::path::Path::new(&std::env::var_os("PEASY_TEST_ENGINE").unwrap()),
            crate::ModelProvider::Ollama {
                base_url: url,
                model: "fixture".into(),
            },
        )
        .unwrap()
    }

    #[test]
    #[ignore = "requires PEASY_TEST_ENGINE; packaged checks run this"]
    fn exact_install_skips_search_and_retains_setup_and_guidance() {
        use peasy_core::{PackageOperation, PackageState, Proposal, ProposalChange};
        let temp = tempfile::tempdir().unwrap();
        let socket = temp.path().join("ipc.sock");
        let setup: peasy_core::ManagedSetup =
            serde_json::from_str(include_str!("../system_configuration/example.json")).unwrap();
        let note = "Review the virtualisation service and group access.";
        let exchanges = vec![
            (
                IpcRequest::GetPackages,
                IpcResponse::Packages { packages: vec![] },
            ),
            (
                IpcRequest::GetTheme,
                IpcResponse::Theme {
                    theme: ThemeSettings::default(),
                },
            ),
            (
                IpcRequest::GetManagedModule,
                IpcResponse::ManagedModule {
                    module: peasy_core::render_packages_module(&PackageState::default()).unwrap(),
                },
            ),
            (
                IpcRequest::LookupPackage {
                    attribute: "virt-manager".into(),
                },
                IpcResponse::SearchResults {
                    candidates: vec![PackageCandidate {
                        attribute: "virt-manager".into(),
                        name: "Virtual Machine Manager".into(),
                        version: "1.0".into(),
                        description: String::new(),
                    }],
                },
            ),
            (
                IpcRequest::ProposeSetup {
                    package: "virt-manager".into(),
                    setup: setup.settings.clone(),
                },
                IpcResponse::Proposal {
                    proposal: Box::new(Proposal {
                        id: "a".repeat(48),
                        title: "Set up virt-manager".into(),
                        packages: vec![],
                        diff: vec![],
                        change: ProposalChange::Setup {
                            operation: PackageOperation::Install,
                            setup: setup.clone(),
                        },
                    }),
                },
            ),
        ];
        let server = scripted_ipc(&socket, exchanges);
        let (url, requests) = crate::tests::serve_json_once(
            serde_json::json!({"message":{"content":serde_json::json!({"result":{"action":"install_package", "package":"virt-manager", "setup":setup.settings, "message":note}}).to_string()}}),
        );
        let client = fixture_client(socket, url);
        let Resolution::Proposal(proposal) =
            client.resolve("install Virtual Machine Manager").unwrap()
        else {
            panic!("expected review");
        };
        assert!(proposal.diff.iter().any(|line| line.text.contains(note)));
        assert_eq!(
            client
                .recent_package
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .attribute,
            "virt-manager"
        );
        requests
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        assert!(requests.try_recv().is_err(), "one model request suffices");
        server.join().unwrap();
    }

    #[test]
    #[ignore = "requires PEASY_TEST_ENGINE; packaged checks run this"]
    fn unavailable_hint_falls_back_with_the_full_request() {
        use peasy_core::PackageState;
        let temp = tempfile::tempdir().unwrap();
        let socket = temp.path().join("ipc.sock");
        let server = scripted_ipc(
            &socket,
            vec![
                (
                    IpcRequest::GetPackages,
                    IpcResponse::Packages { packages: vec![] },
                ),
                (
                    IpcRequest::GetTheme,
                    IpcResponse::Theme {
                        theme: ThemeSettings::default(),
                    },
                ),
                (
                    IpcRequest::GetManagedModule,
                    IpcResponse::ManagedModule {
                        module: peasy_core::render_packages_module(&PackageState::default())
                            .unwrap(),
                    },
                ),
                (
                    IpcRequest::LookupPackage {
                        attribute: "postgresql".into(),
                    },
                    IpcResponse::SearchResults { candidates: vec![] },
                ),
                (
                    IpcRequest::SearchPackages {
                        query: "postgresql".into(),
                        refresh: false,
                    },
                    IpcResponse::SearchResults {
                        candidates: vec![candidate()],
                    },
                ),
            ],
        );
        let actions = [
            serde_json::json!({"result":{"action":"install_package","package":"postgresql","message":null,"setup":null}}),
            serde_json::json!({"result":{"action":"explain","message":"These are the requested client tools."}}),
        ];
        let (url, requests) = crate::tests::serve_json_responses(
            actions
                .into_iter()
                .map(|action| serde_json::json!({"message":{"content":action.to_string()}}))
                .collect(),
        );
        let client = fixture_client(socket, url);
        assert!(matches!(
            client
                .resolve("install PostgreSQL client tools only")
                .unwrap(),
            Resolution::Explain(_)
        ));
        requests
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        let (_, second) = requests
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        let boundary: serde_json::Value =
            serde_json::from_str(second["messages"][1]["content"].as_str().unwrap()).unwrap();
        assert!(
            boundary
                .to_string()
                .contains("install PostgreSQL client tools only")
        );
        assert_eq!(
            boundary["package_candidates"][0]["attribute"],
            "postgresql_17"
        );
        server.join().unwrap();
    }

    #[test]
    #[ignore = "requires PEASY_TEST_ENGINE; packaged checks run this"]
    fn old_daemon_fallback_and_exact_lookup_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        let socket = temp.path().join("ipc.sock");
        let unsupported = || IpcResponse::Error {
            message: "invalid typed IPC request".into(),
        };
        let server = scripted_ipc(
            &socket,
            vec![
                (
                    IpcRequest::LookupPackage {
                        attribute: "hello".into(),
                    },
                    unsupported(),
                ),
                (
                    IpcRequest::SearchPackages {
                        query: "hello".into(),
                        refresh: true,
                    },
                    unsupported(),
                ),
                (
                    IpcRequest::SearchPackages {
                        query: "hello".into(),
                        refresh: false,
                    },
                    IpcResponse::SearchResults { candidates: vec![] },
                ),
                (
                    IpcRequest::LookupPackage {
                        attribute: "hello".into(),
                    },
                    IpcResponse::SearchResults {
                        candidates: vec![candidate()],
                    },
                ),
                (
                    IpcRequest::LookupPackage {
                        attribute: "hello".into(),
                    },
                    IpcResponse::Error {
                        message: "host evaluation failed".into(),
                    },
                ),
            ],
        );
        let client = fixture_client(socket, "http://127.0.0.1:11434".into());
        assert!(client.lookup_package_hint("hello").unwrap().is_none());
        assert!(client.search_packages("hello", true).unwrap().is_empty());
        assert!(
            client
                .lookup_package_hint("hello")
                .unwrap_err()
                .to_string()
                .contains("different package")
        );
        assert!(
            client
                .lookup_package_hint("hello")
                .unwrap_err()
                .to_string()
                .contains("host evaluation failed")
        );
        let wire = serde_json::to_value(IpcRequest::SearchPackages {
            query: "hello".into(),
            refresh: false,
        })
        .unwrap();
        assert!(wire.get("refresh").is_none());
        assert!(matches!(
            serde_json::from_value::<IpcRequest>(wire).unwrap(),
            IpcRequest::SearchPackages { refresh: false, .. }
        ));
        server.join().unwrap();
    }

    #[test]
    fn fallback_choice_keeps_tools_only_intent() {
        let Resolution::Choose(choice) =
            package_choices(vec![candidate()], "install PostgreSQL tools only")
        else {
            panic!("expected choices");
        };
        let ChoiceSource::Nixpkgs { request, .. } = &choice.candidates[0].source else {
            panic!("expected package");
        };
        assert_eq!(request, "install PostgreSQL tools only");
    }

    #[test]
    #[ignore = "requires PEASY_TEST_ENGINE; packaged checks provide the Wasm guest"]
    fn matching_package_does_not_hide_setup_limitation_or_clarification() {
        for explanation in [
            "Peasy can install the local tools, but remote access requires manual configuration.\n1. Ask the database administrator for the server address, database name and authentication method.\n2. Configure the server listener and a narrowly scoped firewall rule on that server, following its existing NixOS module and flake workflow.\n3. Enter credentials locally in your database client; do not paste them into Peasy.\n4. Keep the server's existing data directory and version. A migration requires separate administration.",
            "Do you want a local PostgreSQL server or tools only?",
        ] {
            let temp = tempfile::tempdir().unwrap();
            let socket = temp.path().join("ipc.sock");
            let listener = UnixListener::bind(&socket).unwrap();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut line = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut line)
                    .unwrap();
                let request: IpcRequest = serde_json::from_str(&line).unwrap();
                assert!(matches!(
                    request,
                    IpcRequest::SearchPackages { refresh: true, .. }
                ));
                serde_json::to_writer(
                    &mut stream,
                    &IpcResponse::SearchResults {
                        candidates: vec![candidate()],
                    },
                )
                .unwrap();
                stream.write_all(b"\n").unwrap();
            });
            let action = serde_json::json!({"action":"explain", "message":explanation});
            let (url, request) = crate::tests::serve_json_once(
                serde_json::json!({"message":{"role":"assistant", "content":action.to_string()},"done":true}),
            );
            let engine = std::env::var_os("PEASY_TEST_ENGINE").unwrap();
            let client = PeasyClient::with_provider(
                socket,
                std::path::Path::new(&engine),
                crate::ModelProvider::Ollama {
                    base_url: url,
                    model: "test".into(),
                },
            )
            .unwrap();
            let result = client
                .resolve_package_agent(
                    "install PostgreSQL",
                    "postgresql".into(),
                    Some(RequestedVersion::Latest),
                    &[],
                    &ThemeSettings::default(),
                    "",
                    None,
                    &mut |_| {},
                )
                .unwrap();
            assert!(matches!(result, Resolution::Explain(message) if message == explanation));
            assert_eq!(
                client
                    .recent_package
                    .lock()
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .attribute,
                "postgresql_17"
            );
            request
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap();
            server.join().unwrap();
        }
    }
}

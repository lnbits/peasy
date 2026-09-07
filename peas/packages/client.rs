//! packages pea: unprivileged discovery, review and execution.
use crate::{Choice, ChoiceItem, ChoiceSource, PeasyClient, Resolution, ResolveStage};
use anyhow::{Context, Result, bail};
use peasy_core::{
    EngineDecision, EngineInput, IpcRequest, IpcResponse, PackageCandidate, RequestedVersion,
    ThemeSettings, validate_query,
};

pub(super) fn nix_choice(candidate: PackageCandidate) -> ChoiceItem {
    ChoiceItem {
        name: candidate.name.clone(),
        attribute: candidate.attribute.clone(),
        description: candidate.description.clone(),
        version: candidate.version.clone(),
        source: ChoiceSource::Nixpkgs { candidate },
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

pub(super) fn package_choices(candidates: Vec<PackageCandidate>) -> Resolution {
    Resolution::Choose(Choice {
        intro: Some("I found these installable matches. Choose the one you want:".into()),
        candidates: candidates.into_iter().map(nix_choice).collect(),
    })
}

impl PeasyClient {
    #[allow(clippy::too_many_arguments)] // Explicit, bounded agent-loop context.
    pub(super) fn resolve_package_agent<F>(
        &self,
        request: &str,
        mut query: String,
        mut version: Option<RequestedVersion>,
        installed: &[String],
        theme: &ThemeSettings,
        managed_configuration: &str,
        progress: &mut F,
    ) -> Result<Resolution>
    where
        F: FnMut(ResolveStage),
    {
        for _ in 0..3 {
            progress(ResolveStage::SearchingPackages);
            let mut candidates = match self.ipc.request(&IpcRequest::SearchPackages {
                query: query.clone(),
            })? {
                IpcResponse::SearchResults { candidates } => candidates,
                _ => bail!("unexpected response to SearchPackages"),
            };
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
                        let mut item = nix_choice(candidate.clone());
                        if let Some(setup) = setup {
                            item.source = ChoiceSource::SystemSetup { candidate, setup };
                        }
                        return Ok(Resolution::Choose(Choice {
                            intro: Some(message),
                            candidates: vec![item],
                        }));
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
                            return Ok(package_choices(candidates));
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
                    return self.search_appimages(&query, version.as_ref(), repository.as_deref());
                }
                EngineDecision::Explain(message) => {
                    if has_direct_package_match(&candidates, &query) {
                        return Ok(package_choices(candidates));
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
            IpcResponse::Proposal { proposal } => Ok(Resolution::Proposal(*proposal)),
            _ => bail!("unexpected response to ProposeInstall"),
        }
    }

    pub(super) fn propose_remove(&self, package: &str) -> Result<Resolution> {
        match self.ipc.request(&IpcRequest::ProposeRemove {
            package: package.to_owned(),
        })? {
            IpcResponse::Proposal { proposal } => Ok(Resolution::Proposal(*proposal)),
            _ => bail!("unexpected response to ProposeRemove"),
        }
    }

    pub(super) fn check_package(&self, query: &str) -> Result<Resolution> {
        let candidates = match self.ipc.request(&IpcRequest::SearchPackages {
            query: validate_query(query)?.to_owned(),
        })? {
            IpcResponse::SearchResults { candidates } => candidates,
            _ => bail!("unexpected response to SearchPackages"),
        };
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

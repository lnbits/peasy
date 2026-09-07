//! Packages pea: pure candidate/managed-membership decisions inside zero-import Wasm.
use peasy_core::{EngineDecision, PackageCandidate};

pub(super) fn install(
    package: String,
    message: Option<String>,
    setup: Option<peasy_core::SystemSetup>,
    candidates: &[PackageCandidate],
) -> EngineDecision {
    if let Some(setup) = &setup
        && let Err(error) = setup.validate()
    {
        return EngineDecision::Reject(error.to_string());
    }
    if candidates.iter().any(|item| item.attribute == package) {
        EngineDecision::Install {
            package,
            message,
            setup,
        }
    } else {
        EngineDecision::Reject("model selected a package outside the candidate set".into())
    }
}

pub(super) fn remove(package: String, installed: &[String]) -> EngineDecision {
    if installed.iter().any(|item| item == &package) {
        EngineDecision::Remove(package)
    } else {
        EngineDecision::Reject("model selected a package Peasy does not manage".into())
    }
}

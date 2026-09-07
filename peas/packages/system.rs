//! Packages pea: Nixpkgs lookup, caches, verification and proposals.
const SEARCH_CACHE_CAPACITY: usize = 64;
const SEARCH_CACHE_TTL: Duration = Duration::from_secs(15 * 60);
const PLATFORM_FILTER_LIMIT: usize = MAX_CANDIDATES * 4;
const DISPLAY_CANDIDATE_LIMIT: usize = 6;

use super::{NixBackend, Preview, human_name, useful_stderr};
use anyhow::{Context, Result, bail};
use peasy_core::{
    MAX_CANDIDATES, PackageCandidate, PackageOperation, ProposalChange, module_diff, regex_escape,
    validate_attribute, validate_query,
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

impl NixBackend {
    pub fn search(&self, query: &str) -> Result<Vec<PackageCandidate>> {
        let query = validate_query(query)?;
        let cache_key = query.to_ascii_lowercase();
        if let Some(cached) = self.cached_search(&cache_key) {
            return Ok(cached);
        }
        let _evaluation = self.evaluation_lock.try_lock().map_err(|_| {
            anyhow::anyhow!("A Nix operation is already running; try again shortly")
        })?;
        let flake = format!("path:{}", self.config.nixpkgs.display());
        let output = self.runner.run(
            &self.config.nix,
            &[
                "search".into(),
                "--json".into(),
                "--no-write-lock-file".into(),
                flake.into(),
                format!(".*{}.*", regex_escape(query)).into(),
            ],
            None,
        )?;
        if !output.status.success() {
            bail!("package search failed: {}", useful_stderr(&output));
        }
        let results: BTreeMap<String, Value> = serde_json::from_slice(&output.stdout)
            .context("Nix returned invalid package-search JSON")?;
        let legacy_prefix = format!("legacyPackages.{}.", self.config.system);
        let packages_prefix = format!("packages.{}.", self.config.system);
        let query_lower = query.to_ascii_lowercase();
        let mut candidates = results
            .into_iter()
            .filter_map(|(key, metadata)| {
                let attribute = key
                    .strip_prefix(&legacy_prefix)
                    .or_else(|| key.strip_prefix(&packages_prefix))?
                    .to_owned();
                validate_attribute(&attribute).ok()?;
                let pname = metadata
                    .get("pname")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        attribute
                            .rsplit('.')
                            .next()
                            .unwrap_or(&attribute)
                            .to_owned()
                    });
                let description = metadata
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .chars()
                    .take(240)
                    .collect();
                let version = metadata
                    .get("version")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .chars()
                    .take(64)
                    .collect();
                Some(PackageCandidate {
                    attribute,
                    name: human_name(&pname),
                    description,
                    version,
                })
            })
            .collect::<Vec<_>>();
        candidates.sort_by_key(|candidate| candidate_rank(candidate, &query_lower));
        candidates.truncate(PLATFORM_FILTER_LIMIT);
        let available = self.available_attributes(
            &candidates
                .iter()
                .map(|candidate| candidate.attribute.clone())
                .collect::<Vec<_>>(),
        )?;
        candidates.retain(|candidate| available.contains(&candidate.attribute));
        candidates.sort_by_key(|candidate| candidate_rank(candidate, &query_lower));
        candidates.truncate(DISPLAY_CANDIDATE_LIMIT);
        {
            let mut verified = self
                .verified_packages
                .lock()
                .expect("verified-packages mutex poisoned");
            for candidate in &candidates {
                if verified.len() >= 512 {
                    verified.clear();
                }
                verified.insert(candidate.attribute.clone(), candidate.name.clone());
            }
        }
        self.store_cached_search(cache_key, candidates.clone());
        Ok(candidates)
    }

    pub(super) fn cached_search(&self, key: &str) -> Option<Vec<PackageCandidate>> {
        let mut cache = self
            .search_cache
            .lock()
            .expect("search cache mutex poisoned");
        cache.retain(|_, entry| entry.created.elapsed() < SEARCH_CACHE_TTL);
        cache.get(key).map(|entry| entry.candidates.clone())
    }

    pub(super) fn store_cached_search(&self, key: String, candidates: Vec<PackageCandidate>) {
        let mut cache = self
            .search_cache
            .lock()
            .expect("search cache mutex poisoned");
        cache.retain(|_, entry| entry.created.elapsed() < SEARCH_CACHE_TTL);
        if cache.len() >= SEARCH_CACHE_CAPACITY
            && let Some(oldest) = cache
                .iter()
                .min_by_key(|(_, entry)| entry.created)
                .map(|(key, _)| key.clone())
        {
            cache.remove(&oldest);
        }
        cache.insert(
            key,
            CachedSearch {
                created: Instant::now(),
                candidates,
            },
        );
    }

    pub(super) fn available_attributes(
        &self,
        attributes: &[String],
    ) -> Result<std::collections::HashSet<String>> {
        if attributes.is_empty() {
            return Ok(Default::default());
        }
        let names = attributes
            .iter()
            .map(serde_json::to_string)
            .collect::<std::result::Result<Vec<_>, _>>()?
            .join(" ");
        let nixpkgs = peasy_core::nix_string(&self.config.nixpkgs.to_string_lossy());
        let system = serde_json::to_string(&self.config.system)?;
        let expression = format!(
            r#"let
  pkgs = import (builtins.toPath {nixpkgs}) {{ system = {system}; }};
  lib = pkgs.lib;
  names = [ {names} ];
  isAvailable = name:
    let
      checked = builtins.tryEval (
        let package = lib.attrByPath (lib.splitString "." name) null pkgs;
        in package != null
          && lib.meta.availableOn pkgs.stdenv.hostPlatform package
          && !(package.meta.broken or false)
      );
    in checked.success && checked.value;
in builtins.filter isAvailable names"#
        );
        let output = self.runner.run(
            &self.config.nix,
            &[
                "eval".into(),
                "--impure".into(),
                "--json".into(),
                "--no-write-lock-file".into(),
                "--expr".into(),
                expression.into(),
            ],
            None,
        )?;
        if !output.status.success() {
            bail!(
                "checking package compatibility failed: {}",
                useful_stderr(&output)
            );
        }
        let available: Vec<String> = serde_json::from_slice(&output.stdout)
            .context("Nix returned invalid package-compatibility JSON")?;
        Ok(available.into_iter().collect())
    }

    pub(super) fn package_set_expression(&self) -> String {
        // The administrator's source is already immutable in the store. A
        // path flake needlessly snapshots/hashes it again for each lookup.
        format!(
            "import (builtins.toPath {}) {{ system = {}; }}",
            peasy_core::nix_string(&self.config.nixpkgs.to_string_lossy()),
            peasy_core::nix_string(&self.config.system),
        )
    }

    pub fn verify(&self, attribute: &str) -> Result<String> {
        validate_attribute(attribute)?;
        if let Some(display_name) = self
            .verified_packages
            .lock()
            .expect("verified-packages mutex poisoned")
            .get(attribute)
            .cloned()
        {
            return Ok(display_name);
        }
        let _evaluation = self.evaluation_lock.try_lock().map_err(|_| {
            anyhow::anyhow!("A Nix operation is already running; try again shortly")
        })?;
        let package = format!(
            "(let pkgs = {}; in pkgs.lib.getAttrFromPath (pkgs.lib.splitString \".\" {}) pkgs)",
            self.package_set_expression(),
            peasy_core::nix_string(attribute),
        );
        let output = self.runner.run(
            &self.config.nix,
            &[
                "eval".into(),
                "--impure".into(),
                "--json".into(),
                "--no-write-lock-file".into(),
                "--expr".into(),
                format!("{package}.meta").into(),
            ],
            None,
        )?;
        if !output.status.success() {
            bail!("unknown Nixpkgs package `{attribute}`");
        }
        let _: Value = serde_json::from_slice(&output.stdout)
            .context("Nix returned invalid package metadata")?;
        let pname = self.runner.run(
            &self.config.nix,
            &[
                "eval".into(),
                "--impure".into(),
                "--raw".into(),
                "--no-write-lock-file".into(),
                "--expr".into(),
                format!("{package}.pname").into(),
            ],
            None,
        )?;
        let display_name = if pname.status.success() {
            human_name(String::from_utf8_lossy(&pname.stdout).trim())
        } else {
            human_name(attribute.rsplit('.').next().unwrap_or(attribute))
        };
        let mut verified = self
            .verified_packages
            .lock()
            .expect("verified-packages mutex poisoned");
        if verified.len() >= 512 {
            verified.clear();
        }
        verified.insert(attribute.to_owned(), display_name.clone());
        Ok(display_name)
    }

    pub fn preview_package(&self, operation: PackageOperation, package: &str) -> Result<Preview> {
        validate_attribute(package)?;
        if let Some(setup) = self
            .current_state()?
            .setups
            .into_iter()
            .find(|s| s.package == package)
        {
            if operation == PackageOperation::Remove {
                return self.preview_setup_remove(setup);
            }
            bail!("this package already has a Peasy setup; review a setup update instead");
        }
        if operation == PackageOperation::Remove
            && let Some(appimage) = self
                .current_state()?
                .appimages
                .iter()
                .find(|item| item.id == package)
                .cloned()
        {
            return self.preview_appimage_remove(appimage);
        }
        let display_name = self.verify(package)?;
        let before = self.current_state()?;
        if operation == PackageOperation::Remove
            && !before.packages.iter().any(|item| item == package)
        {
            bail!("Peasy does not manage `{package}`");
        }
        let after = before.with_change(operation, package)?;
        if before == after {
            bail!("`{package}` is already in the requested state");
        }
        let title = format!(
            "{} {display_name}",
            match operation {
                PackageOperation::Install => "Install",
                PackageOperation::Remove => "Remove",
            }
        );
        let mut diff = module_diff(&before, &after)?;
        let dependents = after.setup_dependents(package);
        if !dependents.is_empty() {
            diff.push(peasy_core::DiffLine {
                kind: peasy_core::DiffKind::Context,
                text: match operation {
                    PackageOperation::Remove => format!("Remove the independent Peasy entry only. {package} remains required by: {}.", dependents.join(", ")),
                    PackageOperation::Install => format!("Keep {package} installed independently of its existing setup dependencies: {}.", dependents.join(", ")),
                },
            });
        }
        Ok(Preview {
            diff,
            before,
            change: ProposalChange::Package {
                operation,
                package: package.to_owned(),
                display_name,
            },
            title,
        })
    }
}

fn candidate_rank(candidate: &PackageCandidate, query: &str) -> (u8, u8, u8, u8, usize, String) {
    let leaf = candidate
        .attribute
        .rsplit('.')
        .next()
        .unwrap_or(&candidate.attribute)
        .to_ascii_lowercase();
    let name = candidate.name.to_ascii_lowercase();
    let description = candidate.description.to_ascii_lowercase();
    let query_slug = query.split_whitespace().collect::<Vec<_>>().join("-");
    let exact_rank = u8::from(leaf != query_slug && name != query);
    let match_rank = if leaf == query_slug || name == query {
        0
    } else if leaf.starts_with(&query_slug) || name.starts_with(query) {
        1
    } else if leaf.contains(&query_slug) || name.contains(query) {
        2
    } else {
        3
    };
    let non_desktop_terms = [
        "api", "bridge", "emoji", "exporter", "font", "library", "module", "node", "plugin",
        "python", "server",
    ];
    let category_penalty = non_desktop_terms
        .iter()
        .filter(|term| !query.contains(**term) && leaf.split(['-', '_']).any(|part| part == **term))
        .count()
        .min(u8::MAX as usize) as u8;
    let desktop_rank = u8::from(
        !leaf.contains("desktop")
            && !leaf.contains("electron")
            && !description.contains("desktop")
            && !description.contains("graphical")
            && !description.contains(" gui "),
    );
    (
        exact_rank,
        category_penalty,
        match_rank,
        desktop_rank,
        candidate.attribute.len(),
        candidate.attribute.clone(),
    )
}

pub(super) struct CachedSearch {
    created: Instant,
    candidates: Vec<PackageCandidate>,
}

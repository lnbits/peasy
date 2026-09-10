//! Packages pea: Nixpkgs lookup, caches, verification and proposals.
const SEARCH_CACHE_CAPACITY: usize = 64;
const SEARCH_CACHE_TTL: Duration = Duration::from_secs(15 * 60);
const PLATFORM_FILTER_LIMIT: usize = MAX_CANDIDATES * 4;
const DISPLAY_CANDIDATE_LIMIT: usize = 6;

use super::{NixBackend, Preview, human_name, useful_stderr};
use anyhow::{Context, Result, bail};
use peasy_core::{
    MAX_CANDIDATES, PackageCandidate, PackageOperation, ProposalChange, module_diff,
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
        let output = self.runner.run(
            &self.config.nix,
            &[
                "search".into(),
                "--json".into(),
                "--impure".into(),
                "--no-write-lock-file".into(),
                "--expr".into(),
                self.package_set_expression()?.into(),
                // With --expr, Nix still expects an attribute selector
                // before its regex arguments. Select the whole package set.
                "".into(),
                format!(".*{}.*", peasy_core::regex_escape(query)).into(),
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
                    .or_else(|| key.strip_prefix(&packages_prefix))
                    .unwrap_or(&key)
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
        candidates.retain(|c| {
            c.attribute.to_lowercase().contains(&query_lower)
                || c.name.to_lowercase().contains(&query_lower)
                || c.description.to_lowercase().contains(&query_lower)
        });
        candidates.sort_by_key(|candidate| candidate_rank(candidate, &query_lower));
        candidates.truncate(PLATFORM_FILTER_LIMIT);
        let available = self.identities_unlocked(
            &candidates
                .iter()
                .map(|c| c.attribute.clone())
                .collect::<Vec<_>>(),
            false,
        )?;
        candidates.retain(|c| available.iter().any(|p| p.attribute == c.attribute));
        for candidate in &mut candidates {
            if let Some(p) = available
                .iter()
                .find(|p| p.attribute == candidate.attribute)
            {
                candidate.version = p.version.clone();
                candidate.name = p.name.clone();
            }
        }
        candidates.sort_by_key(|candidate| candidate_rank(candidate, &query_lower));
        candidates.truncate(DISPLAY_CANDIDATE_LIMIT);
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

    pub(super) fn host_expression(&self) -> Result<String> {
        use peasy_core::nix_string;
        match &self.config.rebuild_target {
            super::RebuildTarget::Configuration { path } => {
                let rendered = peasy_core::render_system_expression(
                    &self.config.nixpkgs,
                    path,
                    &self.config.system,
                )?;
                Ok(rendered.replace("evaluated.config.system.build.toplevel", "evaluated"))
            }
            super::RebuildTarget::Flake { reference, .. } => {
                let hostname;
                let (directory, name) = if let Some(parts) = reference.split_once('#') {
                    parts
                } else {
                    hostname = std::fs::read_to_string("/proc/sys/kernel/hostname")
                        .context("reading the host name for the flake configuration")?;
                    (reference.as_str(), hostname.trim())
                };
                if name.is_empty() {
                    bail!("host flake must name a NixOS configuration");
                }
                Ok(format!(
                    "(builtins.getFlake {}).nixosConfigurations.{}",
                    nix_string(&format!("path:{directory}")),
                    nix_string(name)
                ))
            }
        }
    }

    pub(super) fn package_set_expression(&self) -> Result<String> {
        Ok(format!("({}).pkgs", self.host_expression()?))
    }

    pub fn identities(&self, attributes: &[String]) -> Result<Vec<peasy_core::PackageIdentity>> {
        if attributes.is_empty() {
            return Ok(vec![]);
        }
        let _evaluation = self.evaluation_lock.try_lock().map_err(|_| {
            anyhow::anyhow!("A Nix operation is already running; try again shortly")
        })?;
        self.identities_unlocked(attributes, true)
    }

    pub(super) fn identities_unlocked(
        &self,
        attributes: &[String],
        required: bool,
    ) -> Result<Vec<peasy_core::PackageIdentity>> {
        if attributes.is_empty() {
            return Ok(vec![]);
        }
        for name in attributes {
            validate_attribute(name)?;
        }
        let names = attributes
            .iter()
            .map(|n| peasy_core::nix_string(n))
            .collect::<Vec<_>>()
            .join(" ");
        let expression = format!(
            r#"let pkgs = {}; lib = pkgs.lib;
          identify = attribute: let p = lib.attrByPath (lib.splitString "." attribute) null pkgs;
            checked = builtins.tryEval (builtins.deepSeq result result);
            result = if p != null && lib.isDerivation p && lib.meta.availableOn pkgs.stdenv.hostPlatform p && !(p.meta.broken or false)
              then {{ inherit attribute; name = p.pname or attribute; version = p.version or ""; drv_path = p.drvPath; }} else null;
          in if checked.success then checked.value else null;
        in builtins.filter (p: p != null) (map identify [ {} ])"#,
            self.package_set_expression()?,
            names
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
            bail!("checking host packages failed: {}", useful_stderr(&output));
        }
        let mut identities: Vec<peasy_core::PackageIdentity> =
            serde_json::from_slice(&output.stdout).context("invalid host package identities")?;
        for p in &mut identities {
            validate_attribute(&p.attribute)?;
            if !attributes.contains(&p.attribute)
                || !p.drv_path.starts_with("/nix/store/")
                || !p.drv_path.ends_with(".drv")
                || p.drv_path.len() > 512
                || p.name.len() > 240
                || p.version.len() > 64
            {
                bail!("invalid package identity");
            }
            p.name = human_name(&p.name);
        }
        if required
            && attributes
                .iter()
                .any(|name| !identities.iter().any(|p| &p.attribute == name))
        {
            bail!(
                "A requested package is unavailable in the host configuration; review the selection again"
            );
        }
        identities.sort_by(|a, b| a.attribute.cmp(&b.attribute));
        Ok(identities)
    }

    #[cfg(test)]
    pub fn verify(&self, attribute: &str) -> Result<String> {
        Ok(self.identities(&[attribute.to_owned()])?.remove(0).name)
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
        let packages = if operation == PackageOperation::Install {
            self.identities(&[package.to_owned()])?
        } else {
            vec![]
        };
        let display_name = packages
            .first()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| human_name(package));
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
        for p in &packages {
            diff.push(peasy_core::DiffLine {
                kind: peasy_core::DiffKind::Context,
                text: format!(
                    "{} {} (reviewed derivation {})",
                    p.name, p.version, p.drv_path
                ),
            });
        }
        Ok(Preview {
            packages,
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

use crate::{activation, state};
use anyhow::{Context, Result, bail};
use peasy_core::{
    AppImagePackage, ApplyResult, DiffKind, DiffLine, MAX_CANDIDATES, PackageCandidate,
    PackageOperation, PackageState, ProposalChange, ThemeSettings, module_diff, regex_escape,
    render_packages_module, render_system_expression, validate_attribute, validate_query,
};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const SEARCH_CACHE_CAPACITY: usize = 64;
const SEARCH_CACHE_TTL: Duration = Duration::from_secs(15 * 60);
const PLATFORM_FILTER_LIMIT: usize = MAX_CANDIDATES * 4;
const DISPLAY_CANDIDATE_LIMIT: usize = 6;

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
        command
            .output()
            .with_context(|| format!("running trusted executable {}", program.display()))
    }
}

pub struct NixBackend {
    config: BackendConfig,
    runner: Arc<dyn CommandRunner>,
    search_cache: Mutex<HashMap<String, CachedSearch>>,
    verified_packages: Mutex<HashMap<String, String>>,
    apply_lock: Mutex<()>,
}

#[derive(Clone)]
struct CachedSearch {
    created: Instant,
    candidates: Vec<PackageCandidate>,
}

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
            .collect())
    }

    pub fn theme(&self) -> Result<ThemeSettings> {
        Ok(self.current_state()?.theme)
    }

    pub fn managed_module(&self) -> Result<String> {
        render_packages_module(&self.current_state()?).map_err(Into::into)
    }

    pub fn search(&self, query: &str) -> Result<Vec<PackageCandidate>> {
        let query = validate_query(query)?;
        let cache_key = query.to_ascii_lowercase();
        if let Some(cached) = self.cached_search(&cache_key) {
            return Ok(cached);
        }
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
                verified.insert(candidate.attribute.clone(), candidate.name.clone());
            }
        }
        self.store_cached_search(cache_key, candidates.clone());
        Ok(candidates)
    }

    fn cached_search(&self, key: &str) -> Option<Vec<PackageCandidate>> {
        let mut cache = self
            .search_cache
            .lock()
            .expect("search cache mutex poisoned");
        cache.retain(|_, entry| entry.created.elapsed() < SEARCH_CACHE_TTL);
        cache.get(key).map(|entry| entry.candidates.clone())
    }

    fn store_cached_search(&self, key: String, candidates: Vec<PackageCandidate>) {
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

    fn available_attributes(
        &self,
        attributes: &[String],
    ) -> Result<std::collections::HashSet<String>> {
        if attributes.is_empty() {
            return Ok(Default::default());
        }
        let names = attributes
            .iter()
            .map(|attribute| serde_json::to_string(attribute))
            .collect::<std::result::Result<Vec<_>, _>>()?
            .join(" ");
        let nixpkgs = serde_json::to_string(&self.config.nixpkgs.to_string_lossy())?;
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
        let installable = format!(
            "path:{}#legacyPackages.{}.{}",
            self.config.nixpkgs.display(),
            self.config.system,
            attribute
        );
        let output = self.runner.run(
            &self.config.nix,
            &[
                "eval".into(),
                "--json".into(),
                "--no-write-lock-file".into(),
                format!("{installable}.meta").into(),
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
                "--raw".into(),
                "--no-write-lock-file".into(),
                format!("{installable}.pname").into(),
            ],
            None,
        )?;
        let display_name = if pname.status.success() {
            human_name(String::from_utf8_lossy(&pname.stdout).trim())
        } else {
            human_name(attribute.rsplit('.').next().unwrap_or(attribute))
        };
        self.verified_packages
            .lock()
            .expect("verified-packages mutex poisoned")
            .insert(attribute.to_owned(), display_name.clone());
        Ok(display_name)
    }

    pub fn preview_package(&self, operation: PackageOperation, package: &str) -> Result<Preview> {
        validate_attribute(package)?;
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
        Ok(Preview {
            diff: module_diff(&before, &after)?,
            before,
            change: ProposalChange::Package {
                operation,
                package: package.to_owned(),
                display_name,
            },
            title,
        })
    }

    pub fn preview_appimage_install(&self, package: AppImagePackage) -> Result<Preview> {
        package.validate()?;
        let before = self.current_state()?;
        let after = before.with_appimage_install(&package)?;
        if before == after {
            bail!("that exact external AppImage is already installed");
        }
        let replacing = before
            .appimages
            .iter()
            .any(|existing| existing.id == package.id);
        let mut diff = appimage_review_details(&package);
        diff.extend(module_diff(&before, &after)?);
        Ok(Preview {
            before,
            change: ProposalChange::AppImage {
                operation: PackageOperation::Install,
                package: package.clone(),
            },
            title: format!(
                "{} external {} {}",
                if replacing { "Update" } else { "Install" },
                package.display_name,
                package.version
            ),
            diff,
        })
    }

    fn preview_appimage_remove(&self, package: AppImagePackage) -> Result<Preview> {
        package.validate()?;
        let before = self.current_state()?;
        let after = before.with_appimage_remove(&package.id)?;
        let mut diff = appimage_review_details(&package);
        diff.extend(module_diff(&before, &after)?);
        Ok(Preview {
            before,
            change: ProposalChange::AppImage {
                operation: PackageOperation::Remove,
                package: package.clone(),
            },
            title: format!("Remove external {}", package.display_name),
            diff,
        })
    }

    pub fn preview_theme(&self, theme: ThemeSettings) -> Result<Preview> {
        let before = self.current_state()?;
        let after = before.with_theme(&theme)?;
        if before == after {
            bail!("that GNOME theme is already selected");
        }
        let mut details = Vec::new();
        if let Some(color) = theme.accent_color {
            details.push(format!("{color} accent"));
        }
        if let Some(scheme) = theme.color_scheme {
            details.push(format!("{scheme} mode"));
        }
        Ok(Preview {
            diff: module_diff(&before, &after)?,
            before,
            change: ProposalChange::Theme { theme },
            title: format!("Change GNOME theme to {}", details.join(" and ")),
        })
    }

    pub fn apply(
        &self,
        change: &ProposalChange,
        expected: &PackageState,
        proposal_id: &str,
    ) -> Result<ApplyResult> {
        let _guard = self.apply_lock.lock().expect("apply mutex poisoned");
        let previous = self.current_state()?;
        if &previous != expected {
            bail!("proposal is stale because Peasy state changed; review a new diff");
        }
        let (proposed, message) = match change {
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
                    PackageOperation::Remove => format!("{package} removed."),
                };
                (proposed, message)
            }
            ProposalChange::Theme { theme } => {
                let proposed = previous.with_theme(theme)?;
                (
                    proposed,
                    "GNOME theme saved in the active NixOS generation.".to_owned(),
                )
            }
            ProposalChange::AppImage { operation, package } => {
                package.validate()?;
                let proposed = match operation {
                    PackageOperation::Install => previous.with_appimage_install(package)?,
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
        let stage = self
            .config
            .runtime_dir
            .join("transactions")
            .join(proposal_id);
        if stage.exists() {
            bail!("proposal staging directory already exists");
        }
        fs::create_dir_all(&stage)?;
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

fn appimage_review_details(package: &AppImagePackage) -> Vec<DiffLine> {
    vec![
        DiffLine {
            kind: DiffKind::Context,
            text: "External native application — verify the publisher before applying".into(),
        },
        DiffLine {
            kind: DiffKind::Context,
            text: format!("Repository: https://github.com/{}", package.repository),
        },
        DiffLine {
            kind: DiffKind::Context,
            text: format!(
                "Release: {} ({})",
                package.release_tag, package.architecture
            ),
        },
        DiffLine {
            kind: DiffKind::Context,
            text: format!("Asset: {} ({} bytes)", package.asset_name, package.size),
        },
        DiffLine {
            kind: DiffKind::Context,
            text: format!("SHA-256: {}", package.hash),
        },
    ]
}

fn useful_stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr)
        .lines()
        .take(12)
        .collect::<Vec<_>>()
        .join("\n")
        .chars()
        .take(1600)
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;
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

    fn config(runtime_dir: PathBuf) -> BackendConfig {
        let managed_module = runtime_dir.join("source/peasy-managed.nix");
        BackendConfig {
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

        let mut unpinned = package;
        unpinned.hash = "not-a-hash".into();
        assert!(backend.preview_appimage_install(unpinned).is_err());
        assert!(runner.calls.lock().unwrap().is_empty());
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
            "Change GNOME theme to blue accent and dark mode"
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

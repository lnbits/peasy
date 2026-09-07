//! appimages pea: unprivileged discovery, review and execution.
use crate::{Choice, ChoiceItem, ChoiceSource, PeasyClient, Resolution, human_name, safe_stderr};
use anyhow::{Context, Result, bail};
use peasy_core::validate_query;
use peasy_core::{
    AppImageArchitecture, AppImagePackage, IpcRequest, IpcResponse, MAX_APPIMAGE_BYTES,
    RequestedVersion,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Read;
use std::{path::Path, process::Command, time::Duration};

const GITHUB_API: &str = "https://api.github.com";

pub(super) struct GitHubDiscovery {
    client: reqwest::blocking::Client,
}

#[derive(Debug, Deserialize)]
struct RepositorySearch {
    items: Vec<GitHubRepository>,
}

#[derive(Clone, Debug, Deserialize)]
struct GitHubRepository {
    full_name: String,
    name: String,
    description: Option<String>,
    html_url: String,
    stargazers_count: u64,
    fork: bool,
    archived: bool,
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<GitHubAsset>,
}

#[derive(Clone, Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct AppImageCandidate {
    pub display_name: String,
    pub description: String,
    pub repository: String,
    pub repository_url: String,
    pub stars: u64,
    pub version: String,
    pub release_tag: String,
    pub release_url: String,
    pub asset_name: String,
    pub download_url: String,
    pub size: u64,
    pub architecture: AppImageArchitecture,
    pub signature_available: bool,
    pub checksum_available: bool,
}

impl AppImageCandidate {
    pub(super) fn into_package(self, hash: String) -> Result<AppImagePackage> {
        let id = format!(
            "appimage.{}",
            self.repository.to_ascii_lowercase().replace('/', ".")
        );
        let package = AppImagePackage {
            id,
            display_name: self.display_name,
            repository: self.repository,
            version: self.version,
            release_tag: self.release_tag,
            asset_name: self.asset_name,
            url: self.download_url,
            hash,
            architecture: self.architecture,
            size: self.size,
        };
        package.validate()?;
        Ok(package)
    }
}

impl GitHubDiscovery {
    pub(super) fn new() -> Result<Self> {
        Ok(Self {
            client: reqwest::blocking::Client::builder()
                .https_only(true)
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(25))
                .user_agent(concat!("Peasy/", env!("CARGO_PKG_VERSION")))
                .build()?,
        })
    }

    fn search(
        &self,
        query: &str,
        requested_version: Option<&RequestedVersion>,
        repository: Option<&str>,
    ) -> Result<Vec<AppImageCandidate>> {
        let query = validate_query(query)?;
        if let Some(repository) = repository {
            return self.search_repository(repository, requested_version);
        }
        let repositories: RepositorySearch = self.get_json(self.client.get(format!(
            "{GITHUB_API}/search/repositories?q={}&sort=stars&order=desc&per_page=6",
            percent_encode_query(&format!("{query} in:name,description"))
        )))?;
        let architecture = current_appimage_architecture()?;
        let mut repositories = repositories
            .items
            .into_iter()
            .filter(|repository| !repository.fork && !repository.archived)
            .collect::<Vec<_>>();
        repositories.sort_by_key(|repository| repository_rank(repository, query));
        repositories.truncate(5);
        let mut candidates = Vec::new();
        for repository in repositories {
            let releases: Vec<GitHubRelease> = self.get_json(self.client.get(format!(
                "{GITHUB_API}/repos/{}/releases?per_page=30",
                repository.full_name
            )))?;
            if let Some(candidate) =
                release_candidate(repository, releases, requested_version, architecture)
            {
                candidates.push(candidate);
            }
        }
        candidates.sort_by_key(|candidate| {
            (
                candidate.repository.to_ascii_lowercase(),
                std::cmp::Reverse(candidate.stars),
            )
        });
        candidates.sort_by_key(|candidate| {
            let leaf = candidate
                .repository
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase();
            let slug = query_slug(query);
            (u8::from(leaf != slug), std::cmp::Reverse(candidate.stars))
        });
        candidates.truncate(5);
        Ok(candidates)
    }

    fn search_repository(
        &self,
        repository: &str,
        requested_version: Option<&RequestedVersion>,
    ) -> Result<Vec<AppImageCandidate>> {
        let repository = parse_github_repository(repository)
            .context("the requested GitHub repository is invalid")?;
        // GitHub follows renamed repositories here and returns the canonical
        // full_name, so release URLs and the resulting pinned package record
        // remain internally consistent.
        let repository: GitHubRepository =
            self.get_json(self.client.get(format!("{GITHUB_API}/repos/{repository}")))?;
        if repository.fork || repository.archived {
            bail!(
                "github.com/{} is not an active upstream repository",
                repository.full_name
            );
        }
        let releases: Vec<GitHubRelease> = self.get_json(self.client.get(format!(
            "{GITHUB_API}/repos/{}/releases?per_page=30",
            repository.full_name
        )))?;
        Ok(release_candidate(
            repository,
            releases,
            requested_version,
            current_appimage_architecture()?,
        )
        .into_iter()
        .collect())
    }

    fn get_json<T: for<'de> Deserialize<'de>>(
        &self,
        request: reqwest::blocking::RequestBuilder,
    ) -> Result<T> {
        let response = request
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .context("searching GitHub releases")?;
        let status = response.status();
        let mut bytes = Vec::new();
        response.take(2 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > 2 * 1024 * 1024 {
            bail!("GitHub returned an oversized response");
        }
        if !status.is_success() {
            let message = serde_json::from_slice::<Value>(&bytes)
                .ok()
                .and_then(|value| {
                    value
                        .get("message")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| status.to_string());
            bail!("GitHub release search failed: {message}");
        }
        serde_json::from_slice(&bytes).context("GitHub returned invalid release metadata")
    }
}

fn repository_rank(repository: &GitHubRepository, query: &str) -> (u8, u8, std::cmp::Reverse<u64>) {
    let name = repository.name.to_ascii_lowercase();
    let slug = query_slug(query);
    (
        u8::from(name != slug),
        u8::from(!name.contains(&slug)),
        std::cmp::Reverse(repository.stargazers_count),
    )
}

fn query_slug(query: &str) -> String {
    query
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
}

fn parse_github_repository(value: &str) -> Option<String> {
    let value = value
        .trim()
        .trim_end_matches(['/', '.', ',', ':', ';', ')', ']', '}']);
    let mut parts = value.split('/');
    let owner = parts.next()?;
    let repository = parts.next()?.trim_end_matches(".git");
    if parts.next().is_some() || !valid_github_slug(owner) || !valid_github_slug(repository) {
        return None;
    }
    Some(format!(
        "{}/{}",
        owner.to_ascii_lowercase(),
        repository.to_ascii_lowercase()
    ))
}

fn valid_github_slug(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn percent_encode_query(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push(HEX[(byte >> 4) as usize] as char);
            encoded.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    encoded
}

fn nix_store_component(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'+' | b'-' | b'_') {
                byte as char
            } else {
                '-'
            }
        })
        .collect()
}

fn release_candidate(
    repository: GitHubRepository,
    releases: Vec<GitHubRelease>,
    requested_version: Option<&RequestedVersion>,
    architecture: AppImageArchitecture,
) -> Option<AppImageCandidate> {
    for release in releases
        .into_iter()
        .filter(|release| !release.draft && !release.prerelease)
    {
        if let Some(requested) = requested_version
            && !requested.matches(&release.tag_name)
        {
            continue;
        }
        let mut assets = release
            .assets
            .iter()
            .filter(|asset| {
                asset.size > 0
                    && asset.size <= MAX_APPIMAGE_BYTES
                    && asset.name.to_ascii_lowercase().ends_with(".appimage")
            })
            .filter_map(|asset| {
                appimage_asset_rank(&asset.name, architecture).map(|rank| (rank, asset.clone()))
            })
            .collect::<Vec<_>>();
        assets.sort_by_key(|(rank, asset)| (*rank, asset.name.len()));
        let Some((_, asset)) = assets.into_iter().next() else {
            continue;
        };
        let lower_asset = asset.name.to_ascii_lowercase();
        let signature_available = release.assets.iter().any(|other| {
            let name = other.name.to_ascii_lowercase();
            name == format!("{lower_asset}.sig") || name == format!("{lower_asset}.asc")
        });
        let checksum_available = release.assets.iter().any(|other| {
            let name = other.name.to_ascii_lowercase();
            name.contains("sha256") || name.contains("checksums")
        });
        let version = release.tag_name.trim_start_matches(['v', 'V']).to_owned();
        return Some(AppImageCandidate {
            display_name: human_name(&repository.name),
            description: repository
                .description
                .unwrap_or_else(|| "GitHub release providing a Linux AppImage".into())
                .chars()
                .take(240)
                .collect(),
            repository: repository.full_name,
            repository_url: repository.html_url,
            stars: repository.stargazers_count,
            version,
            release_tag: release.tag_name,
            release_url: release.html_url,
            asset_name: asset.name,
            download_url: asset.browser_download_url,
            size: asset.size,
            architecture,
            signature_available,
            checksum_available,
        });
    }
    None
}

fn appimage_asset_rank(name: &str, architecture: AppImageArchitecture) -> Option<u8> {
    let name = name.to_ascii_lowercase();
    let is_x86 = ["x86_64", "x86-64", "amd64", "x64"]
        .iter()
        .any(|token| name.contains(token));
    let is_arm = ["aarch64", "arm64"]
        .iter()
        .any(|token| name.contains(token));
    match architecture {
        AppImageArchitecture::X86_64 if is_arm => None,
        AppImageArchitecture::X86_64 if is_x86 => Some(0),
        AppImageArchitecture::X86_64 => Some(1),
        AppImageArchitecture::Aarch64 if is_x86 => None,
        AppImageArchitecture::Aarch64 if is_arm => Some(0),
        AppImageArchitecture::Aarch64 => None,
    }
}

fn current_appimage_architecture() -> Result<AppImageArchitecture> {
    match std::env::consts::ARCH {
        "x86_64" => Ok(AppImageArchitecture::X86_64),
        "aarch64" => Ok(AppImageArchitecture::Aarch64),
        architecture => bail!("AppImage discovery is not supported on {architecture}"),
    }
}

pub(super) fn appimage_choice(candidate: AppImageCandidate) -> ChoiceItem {
    let verification = match (candidate.checksum_available, candidate.signature_available) {
        (true, true) => "upstream checksum and signature assets available",
        (true, false) => "upstream checksum asset available; no signature asset found",
        (false, true) => "upstream signature asset available; no checksum asset found",
        (false, false) => "no upstream checksum or signature asset found",
    };
    ChoiceItem {
        name: candidate.display_name.clone(),
        attribute: format!(
            "github.com/{} · {} · {}",
            candidate.repository, candidate.release_tag, candidate.asset_name
        ),
        description: format!(
            "{} GitHub stars; {verification}. {}",
            candidate.stars, candidate.description
        ),
        version: candidate.version.clone(),
        source: ChoiceSource::AppImage { candidate },
    }
}

impl PeasyClient {
    pub(super) fn search_appimages(
        &self,
        query: &str,
        version: Option<&RequestedVersion>,
        repository: Option<&str>,
    ) -> Result<Resolution> {
        let policy = peasy_core::AppImagePolicy::load(Path::new(peasy_core::APPIMAGE_POLICY_PATH))?;
        if policy.is_disabled() {
            bail!(
                "External AppImages require an administrator-approved release. Use a Nixpkgs package, or configure services.peasy.appImages.trustedHashes for the publisher you trust."
            );
        }
        let candidates = self
            .github
            .search(query, version, repository)?
            .into_iter()
            .filter(|candidate| policy.allows_repository(&candidate.repository))
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            let version = version
                .map(|value| format!(" at version {value}"))
                .unwrap_or_default();
            let location = repository
                .map(|repository| format!(" in github.com/{repository}"))
                .unwrap_or_else(|| " in the likely GitHub repositories".into());
            bail!(
                "I found no compatible {architecture} AppImage{version}{location}",
                architecture = current_appimage_architecture()?
            );
        }
        Ok(Resolution::Choose(Choice {
            intro: Some("External AppImages are third-party software. Check the GitHub repository and release before choosing a download.".into()),
            candidates: candidates.into_iter().map(appimage_choice).collect(),
        }))
    }

    pub(super) fn prefetch_appimage(&self, candidate: &AppImageCandidate) -> Result<String> {
        let record = candidate
            .clone()
            .into_package("sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".into())?;
        let policy = peasy_core::AppImagePolicy::load(Path::new(peasy_core::APPIMAGE_POLICY_PATH))?;
        if !policy.allows_repository(&record.repository) {
            bail!("This AppImage publisher has not been approved by the administrator");
        }
        if candidate.size == 0 || candidate.size > MAX_APPIMAGE_BYTES {
            bail!("the selected AppImage is outside Peasy's size limit");
        }
        let name = format!(
            "peasy-{}-{}-{}.AppImage",
            nix_store_component(
                candidate
                    .repository
                    .rsplit('/')
                    .next()
                    .unwrap_or("external")
            ),
            nix_store_component(&candidate.version),
            candidate.architecture
        );
        if name.len() > 180 {
            bail!("the selected release produced an unsafe Nix store name");
        }
        let output = Command::new(&self.tools.nix)
            .args([
                "store",
                "prefetch-file",
                "--json",
                "--name",
                &name,
                &candidate.download_url,
            ])
            .env(
                "NIX_CONFIG",
                "extra-experimental-features = nix-command flakes",
            )
            .output()
            .context("downloading the selected AppImage into the Nix store")?;
        if !output.status.success() {
            bail!(
                "Nix could not fetch the AppImage: {}",
                safe_stderr(&output.stderr)
            );
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct PrefetchResult {
            hash: String,
            store_path: String,
        }
        let result: PrefetchResult = serde_json::from_slice(&output.stdout)
            .context("Nix returned invalid AppImage prefetch metadata")?;
        if !result.store_path.starts_with("/nix/store/") {
            bail!("Nix returned an invalid AppImage store path");
        }
        Ok(result.hash)
    }

    pub(super) fn propose_appimage(&self, package: AppImagePackage) -> Result<Resolution> {
        match self
            .ipc
            .request(&IpcRequest::ProposeAppImageInstall { package })?
        {
            IpcResponse::Proposal { proposal } => Ok(Resolution::Proposal(*proposal)),
            _ => bail!("unexpected response to ProposeAppImageInstall"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn repository() -> GitHubRepository {
        GitHubRepository {
            full_name: "example/nostr-chat".into(),
            name: "nostr-chat".into(),
            description: Some("A Nostr chat desktop app".into()),
            html_url: "https://github.com/example/nostr-chat".into(),
            stargazers_count: 1200,
            fork: false,
            archived: false,
        }
    }

    fn release(tag: &str, asset: &str, prerelease: bool) -> GitHubRelease {
        GitHubRelease {
            tag_name: tag.into(),
            html_url: format!("https://github.com/example/nostr-chat/releases/tag/{tag}"),
            draft: false,
            prerelease,
            assets: vec![GitHubAsset {
                name: asset.into(),
                browser_download_url: format!(
                    "https://github.com/example/nostr-chat/releases/download/{tag}/{asset}"
                ),
                size: 42_000_000,
            }],
        }
    }

    #[test]
    fn github_release_selection_is_stable_exact_and_arch_specific() {
        let candidate = release_candidate(
            repository(),
            vec![
                release("v2.0.0-beta", "nostr-chat-x86_64.AppImage", true),
                release("v1.3.0", "nostr-chat-aarch64.AppImage", false),
                release("v1.2.0", "nostr-chat-x86_64.AppImage", false),
            ],
            None,
            AppImageArchitecture::X86_64,
        )
        .unwrap();
        assert_eq!(candidate.release_tag, "v1.2.0");
        assert_eq!(candidate.asset_name, "nostr-chat-x86_64.AppImage");

        let exact = release_candidate(
            repository(),
            vec![
                release("v1.3", "nostr-chat-x86_64.AppImage", false),
                release("v1.2", "nostr-chat-x86_64.AppImage", false),
            ],
            Some(&RequestedVersion::Exact("1.2".into())),
            AppImageArchitecture::X86_64,
        )
        .unwrap();
        assert_eq!(exact.release_tag, "v1.2");
        assert!(
            release_candidate(
                repository(),
                vec![release("v1.2.1", "nostr-chat-x86_64.AppImage", false)],
                Some(&RequestedVersion::Exact("1.2".into())),
                AppImageArchitecture::X86_64,
            )
            .is_none()
        );
        assert_eq!(percent_encode_query("nostr chat"), "nostr%20chat");
    }

    #[test]
    fn discovered_appimage_still_passes_the_closed_system_record() {
        let candidate = release_candidate(
            repository(),
            vec![release("v1.2", "nostr-chat-x86_64.AppImage", false)],
            None,
            AppImageArchitecture::X86_64,
        )
        .unwrap();
        let package = candidate
            .clone()
            .into_package("sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".into())
            .unwrap();
        package.validate().unwrap();

        let mut hostile = candidate;
        hostile.download_url = "https://attacker.invalid/payload.AppImage".into();
        assert!(
            hostile
                .into_package("sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".into())
                .is_err()
        );
    }
}

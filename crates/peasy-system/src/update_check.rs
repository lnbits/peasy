//! Network-only, unprivileged helper. No Nix access, AI or user-selected URLs.
use anyhow::{Context, Result, bail};
use peasy_core::{PeasyRelease, UPDATE_ASSET};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{io::Read, time::Duration};

const API: &str = "https://api.github.com/repos/lnbits/peasy";
const MAX_RESPONSE: usize = 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublishedRelease {
    pub latest_version: Option<String>,
    pub release: Option<PeasyRelease>,
    pub message: String,
}

fn lookup(
    mut fetch: impl FnMut(&str, usize) -> Result<Option<Vec<u8>>>,
) -> Result<PublishedRelease> {
    let Some(bytes) = fetch(&format!("{API}/releases/latest"), MAX_RESPONSE)? else {
        return Ok(PublishedRelease {
            latest_version: None,
            release: None,
            message: "No stable Peasy release has been published yet.".into(),
        });
    };
    let latest: Value = serde_json::from_slice(&bytes)?;
    if latest["draft"] != false
        || latest["prerelease"] != false
        || !latest["published_at"].is_string()
    {
        bail!("GitHub did not return a published stable release");
    }
    let tag = latest["tag_name"].as_str().context("release has no tag")?;
    // Validate before ever placing the tag into a URL.
    let mut pin = PeasyRelease {
        format: 1,
        version: tag.strip_prefix('v').unwrap_or("").into(),
        tag: tag.into(),
        revision: "0".repeat(40),
        sha256: "0".repeat(64),
    };
    pin.validate()?;
    let asset_url =
        format!("https://github.com/lnbits/peasy/releases/download/{tag}/{UPDATE_ASSET}");
    let present = latest["assets"].as_array().is_some_and(|assets| {
        assets.iter().any(|a| {
            a["name"] == UPDATE_ASSET
                && a["browser_download_url"] == asset_url
                && a["state"] == "uploaded"
                && a["size"]
                    .as_u64()
                    .is_some_and(|size| size > 0 && size <= 4096)
        })
    });
    if !present {
        return Ok(PublishedRelease {
            latest_version: Some(pin.version.clone()),
            release: None,
            message: format!(
                "GitHub release {tag} has no verified updater metadata. Update through your host configuration; see https://github.com/lnbits/peasy/releases/tag/{tag}."
            ),
        });
    }
    pin = serde_json::from_slice(
        &fetch(&asset_url, 4096)?.context("release updater metadata disappeared")?,
    )?;
    pin.validate()?;
    if pin.tag != tag {
        bail!("release metadata does not match its published tag");
    }
    let mut reference: Value = serde_json::from_slice(
        &fetch(&format!("{API}/git/ref/tags/{tag}"), 32768)?.context("release tag disappeared")?,
    )?;
    for _ in 0..4 {
        let object = &reference["object"];
        let sha = object["sha"].as_str().context("invalid tag revision")?;
        if sha.len() != 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("invalid tag revision");
        }
        match object["type"].as_str() {
            Some("commit") if sha == pin.revision => {
                return Ok(PublishedRelease {
                    latest_version: Some(pin.version.clone()),
                    release: Some(pin),
                    message: "Published release verified.".into(),
                });
            }
            Some("tag") => {
                reference = serde_json::from_slice(
                    &fetch(&format!("{API}/git/tags/{sha}"), 32768)?
                        .context("annotated tag disappeared")?,
                )?
            }
            _ => bail!("published release revision does not match updater metadata"),
        }
    }
    bail!("release tag nesting exceeds its limit")
}

pub fn run() -> Result<()> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .connect_timeout(Duration::from_secs(5))
        .user_agent("Peasy-release-check/1")
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() < 3
                && attempt.url().scheme() == "https"
                && matches!(
                    attempt.url().host_str(),
                    Some("github.com" | "release-assets.githubusercontent.com" | "api.github.com")
                )
            {
                attempt.follow()
            } else {
                attempt.error("untrusted release redirect")
            }
        }))
        .build()?;
    let result = lookup(|url, limit| {
        let mut response = client
            .get(url)
            .header("Accept", "application/vnd.github+json")
            .send()?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if response.status() == reqwest::StatusCode::FORBIDDEN
            || response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
        {
            bail!("GitHub rate limit or access restriction; try checking later");
        }
        if !response.status().is_success() {
            bail!("GitHub release check returned {}", response.status());
        }
        if response.content_length().is_some_and(|n| n > limit as u64) {
            bail!("release response too large");
        }
        let mut bytes = Vec::new();
        (&mut response)
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > limit {
            bail!("release response too large");
        }
        Ok(Some(bytes))
    });
    // A failed check never leaves a previous successful report to be mistaken for fresh data.
    let report = match result {
        Ok(published) => serde_json::json!({"published": published}),
        Err(error) => serde_json::json!({"error": format!("{error:#}")}),
    };
    std::fs::write(
        "/run/peasy-update-check/result.json",
        serde_json::to_vec(&report)?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture() -> (PeasyRelease, Value) {
        let pin = PeasyRelease {
            format: 1,
            version: "0.2.0".into(),
            tag: "v0.2.0".into(),
            revision: "a".repeat(40),
            sha256: "b".repeat(64),
        };
        let latest = json!({"tag_name":pin.tag,"draft":false,"prerelease":false,"published_at":"2026-09-25", "assets":[{"name":UPDATE_ASSET,"state":"uploaded","size":256,"browser_download_url":format!("https://github.com/lnbits/peasy/releases/download/{}/{}",pin.tag,UPDATE_ASSET)}]});
        (pin, latest)
    }
    #[test]
    fn verifies_published_asset_and_annotated_tag_independently() {
        let (pin, latest) = fixture();
        let found = lookup(|url, _| {
            Ok(Some(serde_json::to_vec(
                &if url.ends_with("/releases/latest") {
                    latest.clone()
                } else if url.ends_with(UPDATE_ASSET) {
                    serde_json::to_value(&pin).unwrap()
                } else if url.contains("/git/ref/tags/") {
                    json!({"object":{"type":"tag","sha":"c".repeat(40)}})
                } else {
                    assert!(url.ends_with(&"c".repeat(40)));
                    json!({"object":{"type":"commit","sha":pin.revision}})
                },
            )?))
        })
        .unwrap();
        assert_eq!(found.release, Some(pin));
    }
    #[test]
    fn rejects_moved_tags_drafts_and_missing_metadata_without_guessing_a_download() {
        let (pin, latest) = fixture();
        assert!(
            lookup(|url, _| Ok(Some(serde_json::to_vec(
                &if url.ends_with("/releases/latest") {
                    latest.clone()
                } else if url.ends_with(UPDATE_ASSET) {
                    serde_json::to_value(&pin).unwrap()
                } else {
                    json!({"object":{"type":"commit","sha":"d".repeat(40)}})
                }
            )?)))
            .is_err()
        );
        let mut draft = latest.clone();
        draft["draft"] = json!(true);
        assert!(lookup(|_, _| Ok(Some(serde_json::to_vec(&draft)?))).is_err());
        let mut no_asset = latest;
        no_asset["assets"] = json!([]);
        assert!(
            lookup(|url, _| {
                assert!(url.ends_with("/releases/latest"));
                Ok(Some(serde_json::to_vec(&no_asset)?))
            })
            .unwrap()
            .release
            .is_none()
        );
        assert!(lookup(|_, _| Ok(None)).unwrap().release.is_none());
    }
}

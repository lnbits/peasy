//! Fixed official-source verification in an unprivileged, network-enabled service.
//! The privileged configuration daemon keeps its AF_UNIX-only sandbox.
use anyhow::{Context, Result, bail};
use peasy_core::pea::{MAX_PACK_BYTES, POLICY_PATH, PeaCatalogue, PeaManifest, PeaPin, PeaPolicy};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::Path,
    time::Duration,
};

const OFFICIAL_REF: &str = "https://api.github.com/repos/lnbits/peasy/git/ref/heads/main";
const MAX_CATALOGUE_BYTES: usize = 128 * 1024;

fn read_response(mut response: reqwest::blocking::Response, limit: usize) -> Result<Vec<u8>> {
    if !response.status().is_success() {
        bail!("official pea source returned {}", response.status());
    }
    if response.content_length().is_some_and(|n| n > limit as u64) {
        bail!("official pea response exceeds its size limit");
    }
    let mut bytes = Vec::new();
    (&mut response)
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        bail!("official pea response exceeds its size limit");
    }
    Ok(bytes)
}

pub(crate) fn validate_artifact(pin: &PeaPin, bytes: &[u8]) -> Result<PeaManifest> {
    pin.validate()?;
    if bytes.len() > MAX_PACK_BYTES
        || !hex::encode(Sha256::digest(bytes)).eq_ignore_ascii_case(&pin.hash)
    {
        bail!("pea artifact exceeds its size limit or does not match its reviewed hash");
    }
    let manifest: PeaManifest = serde_json::from_slice(bytes)?;
    manifest.validate()?;
    if !pin.matches(&manifest) {
        bail!("pea metadata does not match its reviewed pin");
    }
    Ok(manifest)
}

fn verify_source(
    pin: &PeaPin,
    restore: bool,
    mut fetch: impl FnMut(&str, usize) -> Result<Vec<u8>>,
) -> Result<Vec<u8>> {
    pin.validate()?;
    // Discovery requires the current main revision. Restore may use an older
    // revision only after proving it is an ancestor of independently read main.
    // Merely existing in this repository also includes unpublished PR commits.
    let reference: serde_json::Value = serde_json::from_slice(&fetch(OFFICIAL_REF, 32 * 1024)?)?;
    let head = reference["object"]["sha"]
        .as_str()
        .context("missing official revision")?;
    if reference["object"]["type"] != "commit"
        || head.len() != 40
        || !head.bytes().all(|b| b.is_ascii_hexdigit())
    {
        bail!("invalid official main revision");
    }
    if head != pin.revision {
        if !restore {
            bail!(
                "official catalogue revision changed or was not published on main; discover and review again"
            );
        }
        // Page two avoids the unbounded changed-file patches on page one.
        // https://docs.github.com/en/rest/commits/commits#compare-two-commits
        let comparison = format!(
            "https://api.github.com/repos/lnbits/peasy/compare/{}...{head}?per_page=1&page=2",
            pin.revision
        );
        let result: serde_json::Value = serde_json::from_slice(&fetch(&comparison, 128 * 1024)?)?;
        if result["status"] != "ahead"
            || result["behind_by"] != 0
            || result["base_commit"]["sha"] != pin.revision
            || result["merge_base_commit"]["sha"] != pin.revision
        {
            bail!("backup pea revision is not in the published main history");
        }
    }
    let url = format!(
        "https://raw.githubusercontent.com/lnbits/peasy/{}/peas/catalogue.json",
        pin.revision
    );
    let catalogue: PeaCatalogue = serde_json::from_slice(&fetch(&url, MAX_CATALOGUE_BYTES)?)?;
    catalogue.validate(&pin.revision)?;
    let entry = catalogue
        .peas
        .iter()
        .find(|e| {
            let m = &e.package;
            m.id == pin.id
                && m.version == pin.version
                && m.host_api == pin.host_api
                && m.hash == pin.hash
                && m.permissions == pin.permissions
        })
        .context("pea pin is not an exact member of the official catalogue")?;
    let bytes = fetch(&pin.url(), MAX_PACK_BYTES)?;
    let manifest = validate_artifact(pin, &bytes)?;
    if entry.capabilities != manifest.capabilities {
        bail!("pea capabilities do not match the official catalogue");
    }
    Ok(bytes)
}

pub fn run() -> Result<()> {
    let request = std::fs::File::open("/run/peasy-pea-request.json")?;
    let mut bytes = vec![];
    request.take(8193).read_to_end(&mut bytes)?;
    if bytes.len() > 8192 {
        bail!("pea request too large");
    }
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct RestoreRequest {
        pin: PeaPin,
        restore: bool,
    }
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Request {
        Current(PeaPin),
        Backup(RestoreRequest),
    }
    let (pin, restore) = match serde_json::from_slice(&bytes)? {
        Request::Current(pin) => (pin, false),
        Request::Backup(request) => (request.pin, request.restore),
    };
    if !PeaPolicy::load(Path::new(POLICY_PATH))?.allows(&pin) {
        bail!("pea is disallowed by administrator policy");
    }
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .user_agent("Peasy-pea-verifier/1")
        .build()?;
    let bytes = verify_source(&pin, restore, |url, limit| {
        read_response(client.get(url).send()?, limit)
    })?;
    // RuntimeDirectory is private to this one-shot service and cleared on stop.
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open("/run/peasy-pea-fetch/pea.json")?;
    output.write_all(&bytes)?;
    output.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::{net::TcpListener, thread, time::Instant};

    fn fixture() -> (PeaPin, Value, Vec<u8>) {
        let bytes = include_bytes!("../../../peas/networking/pea.json").to_vec();
        let m: PeaManifest = serde_json::from_slice(&bytes).unwrap();
        let pin = PeaPin {
            id: m.id,
            version: m.version,
            revision: "a".repeat(40),
            hash: hex::encode(Sha256::digest(&bytes)),
            host_api: m.host_api,
            permissions: m.permissions,
        };
        let catalogue = json!({"format":1,"peas":[{"package":{
            "id":pin.id,"version":pin.version,"hash":pin.hash,"host_api":pin.host_api,
            "permissions":pin.permissions},"capabilities":m.capabilities}]});
        (pin, catalogue, bytes)
    }

    #[test]
    fn restore_requires_published_ancestry_before_fetching_historical_artifacts() {
        let (pin, catalogue, bytes) = fixture();
        for accepted in [false, true] {
            let mut artifact_fetched = false;
            let result = verify_source(&pin, true, |url, _limit| {
                if url == OFFICIAL_REF {
                    return Ok(serde_json::to_vec(
                        &json!({"object":{"type":"commit","sha":"b".repeat(40)}}),
                    )?);
                }
                if url.contains("/compare/") {
                    assert!(url.ends_with("?per_page=1&page=2"));
                    return Ok(serde_json::to_vec(&json!({
                        "status": if accepted { "ahead" } else { "diverged" },
                        "behind_by": if accepted { 0 } else { 1 },
                        "base_commit":{"sha":pin.revision},
                        "merge_base_commit":{"sha": if accepted { pin.revision.clone() } else { "c".repeat(40) }}
                    }))?);
                }
                assert!(accepted, "unpublished artifact must not be fetched");
                if url.ends_with("catalogue.json") {
                    return Ok(serde_json::to_vec(&catalogue)?);
                }
                assert_eq!(url, pin.url());
                artifact_fetched = true;
                Ok(bytes.clone())
            });
            assert_eq!(result.is_ok(), accepted);
            assert_eq!(artifact_fetched, accepted);
        }
        // An 'ahead' label alone is insufficient without the exact merge base.
        assert!(verify_source(&pin, true, |url, _| {
            if url == OFFICIAL_REF {
                Ok(serde_json::to_vec(&json!({"object":{"type":"commit","sha":"b".repeat(40)}}))?)
            } else {
                assert!(url.contains("/compare/"));
                Ok(serde_json::to_vec(&json!({"status":"ahead", "behind_by":0, "base_commit":{"sha":pin.revision}, "merge_base_commit":{"sha":"c".repeat(40)}}))?)
            }
        }).is_err());
    }

    #[test]
    fn unpublished_revisions_and_forged_catalogue_metadata_never_fetch_a_package() {
        let (pin, catalogue, bytes) = fixture();
        for field in [
            "revision",
            "missing",
            "hash",
            "version",
            "host_api",
            "permissions",
        ] {
            let mut catalogue = catalogue.clone();
            match field {
                "missing" => catalogue["peas"] = json!([]),
                "hash" => catalogue["peas"][0]["package"][field] = json!("b".repeat(64)),
                "version" => catalogue["peas"][0]["package"][field] = json!("different"),
                "host_api" => catalogue["peas"][0]["package"][field] = json!(99),
                "permissions" => catalogue["peas"][0]["package"][field] = json!(["network.read"]),
                _ => {}
            }
            let mut calls = vec![];
            let result = verify_source(&pin, false, |url, limit| {
                calls.push((url.to_owned(), limit));
                if url == OFFICIAL_REF {
                    return Ok(serde_json::to_vec(
                        &json!({"object":{"type":"commit","sha":
                        if field == "revision" {"b".repeat(40)} else {pin.revision.clone()}}}),
                    )?);
                }
                assert!(
                    url.ends_with("/peas/catalogue.json"),
                    "unapproved artifact must not be requested"
                );
                Ok(serde_json::to_vec(&catalogue)?)
            });
            assert!(result.is_err(), "{field}");
            assert_eq!(calls.len(), if field == "revision" { 1 } else { 2 });
        }
        let mut calls = vec![];
        let result = verify_source(&pin, false, |url, limit| {
            calls.push((url.to_owned(), limit));
            if url == OFFICIAL_REF {
                Ok(serde_json::to_vec(
                    &json!({"object":{"type":"commit","sha":pin.revision}}),
                )?)
            } else if url.ends_with("catalogue.json") {
                Ok(serde_json::to_vec(&catalogue)?)
            } else {
                assert_eq!(url, pin.url());
                Ok(bytes.clone())
            }
        })
        .unwrap();
        assert_eq!(result, bytes);
        assert_eq!(
            calls.iter().map(|c| c.1).collect::<Vec<_>>(),
            vec![32768, MAX_CATALOGUE_BYTES, MAX_PACK_BYTES]
        );
        let mut altered = bytes;
        altered.push(b' ');
        assert!(validate_artifact(&pin, &altered).is_err());
    }

    fn server(response: &'static [u8], hold_open: bool) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = vec![];
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                if stream.read(&mut byte).unwrap() == 0 {
                    return;
                }
                request.push(byte[0]);
            }
            stream.write_all(response).unwrap();
            if hold_open {
                // The bounded reader must close instead of consuming an unbounded body.
                assert_eq!(stream.read(&mut [0]).unwrap(), 0);
            }
        });
        (url, worker)
    }

    #[test]
    fn downloads_bound_declared_and_chunked_bodies_and_reject_redirects() {
        for response in [
            b"HTTP/1.1 200 OK\r\nContent-Length: 999999999\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n6\r\n123456\r\n".as_slice(),
            b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/forbidden\r\nContent-Length: 0\r\n\r\n".as_slice(),
        ] {
            let (url, worker) = server(response, true);
            let client = reqwest::blocking::Client::builder().no_proxy()
                .redirect(reqwest::redirect::Policy::none()).build().unwrap();
            assert!(read_response(client.get(url).send().unwrap(), 4).is_err());
            drop(client);
            worker.join().unwrap();
        }
    }

    #[test]
    fn timeout_covers_a_stalled_body() {
        let (url, worker) = server(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nx", true);
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_millis(150))
            .build()
            .unwrap();
        let start = Instant::now();
        assert!(read_response(client.get(url).send().unwrap(), 10).is_err());
        assert!(start.elapsed() < Duration::from_secs(2));
        drop(client);
        worker.join().unwrap();
    }
}

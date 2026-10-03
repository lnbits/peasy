//! Curated local model downloads. Model identifiers never come from the LLM.
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::time::Duration;

use crate::{http, safe_provider_error, validate_model_name, validate_ollama_url};

/// A versioned, data-only recommendation list published by Peasy.
/// The private fields ensure downloads use a catalogue that passed validation.
#[derive(Clone, Debug)]
pub struct ModelCatalogue {
    models: Vec<SuggestedModel>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogueDocument {
    version: u32,
    models: Vec<SuggestedModel>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuggestedModel {
    pub name: String,
    /// Approximate registry download bytes, not runtime RAM requirements.
    pub size: u64,
}

pub const CATALOGUE_URL: &str = "https://raw.githubusercontent.com/lnbits/peasy/main/crates/peasy-client/src/ollama-catalogue.json";
const BUNDLED_MODEL: &str = "qwen3:0.6b";

impl ModelCatalogue {
    pub fn bundled() -> Self {
        Self::parse(include_bytes!("ollama-catalogue.json"))
            .expect("bundled model catalogue must validate")
    }

    pub fn models(&self) -> &[SuggestedModel] {
        &self.models
    }

    fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > 16 * 1024 {
            bail!("Model catalogue is too large");
        }
        let catalogue: CatalogueDocument = serde_json::from_slice(bytes)?;
        if catalogue.version != 1 || catalogue.models.len() != 6 {
            bail!("Unsupported model catalogue version or size");
        }
        if catalogue.models[0].name != BUNDLED_MODEL {
            bail!("Model catalogue must preserve the bundled model");
        }
        let mut names = std::collections::BTreeSet::new();
        for model in &catalogue.models {
            // Only explicit local tags in Ollama's official library. No remote
            // registries, namespaces, paths, cloud tags or implicit latest tag.
            let (name, tag) = model
                .name
                .split_once(':')
                .context("Model tag is required")?;
            let valid = |part: &str| {
                !part.is_empty()
                    && part.as_bytes()[0].is_ascii_alphanumeric()
                    && part.bytes().all(|b| {
                        b.is_ascii_lowercase() || b.is_ascii_digit() || b".-_".contains(&b)
                    })
                    && !part.contains("..")
            };
            if model.name.len() > 100
                || !valid(name)
                || !valid(tag)
                || tag == "latest"
                || tag.contains("cloud")
                || name.contains("cloud")
                || model.size == 0
                || model.size > 20_000_000_000
                || !names.insert(&model.name)
            {
                bail!("Invalid model catalogue entry");
            }
        }
        Ok(Self {
            models: catalogue.models,
        })
    }
}

/// Refresh metadata only, never model weights. Successful catalogues are cached
/// for six hours in this process. Offline/invalid responses retain the last
/// valid catalogue or the compiled fallback, without failing local settings.
pub fn recommended_models(force_refresh: bool) -> ModelCatalogue {
    use std::sync::{Mutex, OnceLock};
    use std::time::Instant;
    static CACHE: OnceLock<Mutex<Option<(Instant, ModelCatalogue)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    let previous = cache.lock().ok().and_then(|c| c.clone());
    if let Some((when, catalogue)) = &previous
        && !force_refresh
        && when.elapsed() < Duration::from_secs(6 * 60 * 60)
    {
        return catalogue.clone();
    }
    let fetched = (|| -> Result<ModelCatalogue> {
        let client = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(4))
            .user_agent(concat!("Peasy/", env!("CARGO_PKG_VERSION")))
            .build()?;
        fetch_catalogue(client.get(CATALOGUE_URL))
    })();
    match fetched {
        Ok(catalogue) => {
            if let Ok(mut cache) = cache.lock() {
                *cache = Some((Instant::now(), catalogue.clone()));
            }
            catalogue
        }
        Err(_) => previous
            .map(|(_, catalogue)| catalogue)
            .unwrap_or_else(ModelCatalogue::bundled),
    }
}

fn fetch_catalogue(request: reqwest::RequestBuilder) -> Result<ModelCatalogue> {
    let (status, bytes) = http::read(request, 16 * 1024)?;
    if !status.is_success() {
        bail!("Model catalogue HTTP {status}");
    }
    ModelCatalogue::parse(&bytes)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstalledModel {
    pub name: String,
    pub size: Option<u64>,
}

pub fn list_ollama_models(base_url: &str) -> Result<Vec<String>> {
    Ok(list_ollama_model_details(base_url)?
        .into_iter()
        .map(|m| m.name)
        .collect())
}

pub fn list_ollama_model_details(base_url: &str) -> Result<Vec<InstalledModel>> {
    validate_ollama_url(base_url)?;
    let (status, bytes) = http::read(
        client(Duration::from_secs(15))?
            .get(format!("{}/api/tags", base_url.trim_end_matches('/'))),
        256 * 1024,
    )
    .context("connecting to local Ollama")?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).context("Ollama returned invalid JSON")?;
    if !status.is_success() {
        bail!(
            "Ollama: {}",
            safe_provider_error(
                value["error"]
                    .as_str()
                    .unwrap_or("could not list Ollama models")
            )
        );
    }
    let mut models = value["models"]
        .as_array()
        .context("Ollama model list did not contain models")?
        .iter()
        .filter_map(|item| {
            let name = item.get("name").or_else(|| item.get("model"))?.as_str()?;
            validate_model_name(name).ok()?;
            Some(InstalledModel {
                name: name.into(),
                size: item["size"].as_u64(),
            })
        })
        .collect::<Vec<_>>();
    models.sort_by(|a, b| a.name.cmp(&b.name));
    models.dedup_by(|a, b| a.name == b.name);
    Ok(models)
}

fn client(timeout: Duration) -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .tls_certs_only(std::iter::empty::<reqwest::Certificate>())
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(3))
        .read_timeout(Duration::from_secs(120))
        .timeout(timeout)
        .user_agent(concat!("Peasy/", env!("CARGO_PKG_VERSION")))
        .build()?)
}

#[derive(Debug, Deserialize)]
pub struct PullProgress {
    pub status: Option<String>,
    pub total: Option<u64>,
    pub completed: Option<u64>,
    error: Option<String>,
}

fn progress_record(bytes: &[u8], progress: &mut impl FnMut(PullProgress)) -> Result<bool> {
    let event: PullProgress =
        serde_json::from_slice(bytes).context("Ollama returned invalid download progress")?;
    if let Some(error) = &event.error {
        bail!("Ollama: {}", safe_provider_error(error));
    }
    let success = event.status.as_deref() == Some("success");
    if event.status.is_none() {
        bail!("Ollama download progress is missing status");
    }
    progress(event);
    Ok(success)
}

/// Pull through the existing local daemon, never a shell, remote server or LLM.
/// A successful stream is followed by a fresh inventory to confirm installation.
pub fn pull_ollama_model(
    base_url: &str,
    model: &str,
    catalogue: &ModelCatalogue,
    mut progress: impl FnMut(PullProgress),
) -> Result<Vec<InstalledModel>> {
    validate_ollama_url(base_url)?;
    if !catalogue.models().iter().any(|entry| entry.name == model) {
        bail!("Model is not in Peasy's download catalogue");
    }
    http::read_lines(
        client(Duration::from_secs(2 * 60 * 60))?
            .post(format!("{}/api/pull", base_url.trim_end_matches('/')))
            .json(&serde_json::json!({"model": model, "stream": true})),
        |bytes| progress_record(bytes, &mut progress),
    )?;
    let models = list_ollama_model_details(base_url)?;
    if !models.iter().any(|installed| installed.name == model) {
        bail!("Ollama did not list the downloaded model as installed; refresh and try again");
    }
    Ok(models)
}

/// Remove only a model present in the local daemon's current inventory. Check
/// again after DELETE; an HTTP success alone is not proof of removal.
pub fn remove_ollama_model(base_url: &str, model: &str) -> Result<Vec<InstalledModel>> {
    validate_ollama_url(base_url)?;
    validate_model_name(model)?;
    let models = list_ollama_model_details(base_url)?;
    if !models.iter().any(|entry| entry.name == model) {
        bail!("Model is no longer installed; refresh the model list");
    }
    // Once a deletion starts, let its bounded request and inventory check finish
    // even if settings closes. Cancellation cannot undo a daemon-side deletion.
    peasy_core::cancellation::Cancellation::current().protect()?;
    let (status, bytes) = http::read(
        client(Duration::from_secs(30))?
            .delete(format!("{}/api/delete", base_url.trim_end_matches('/')))
            .json(&serde_json::json!({"model": model})),
        16 * 1024,
    )?;
    if !status.is_success() {
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
        bail!(
            "Ollama HTTP {status}: {}",
            safe_provider_error(value["error"].as_str().unwrap_or("model removal failed"))
        );
    }
    let models = list_ollama_model_details(base_url)?;
    if models.iter().any(|entry| entry.name == model) {
        bail!("Ollama still lists the model as installed; refresh and try again");
    }
    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn progress_requires_explicit_success_and_preserves_failures() {
        assert!(
            !progress_record(
                br#"{"status":"pulling a layer","completed":2,"total":10}"#,
                &mut |_| {}
            )
            .unwrap()
        );
        assert!(progress_record(br#"{"status":"success"}"#, &mut |_| {}).unwrap());
        assert!(
            progress_record(br#"{"error":"disk full"}"#, &mut |_| {})
                .unwrap_err()
                .to_string()
                .contains("disk full")
        );
        assert!(progress_record(b"{}", &mut |_| {}).is_err());
        assert!(progress_record(b"not json", &mut |_| {}).is_err());
    }
    #[test]
    fn catalogue_accepts_new_choices_but_rejects_unsafe_or_incompatible_data() {
        let original: serde_json::Value =
            serde_json::from_slice(include_bytes!("ollama-catalogue.json")).unwrap();
        let mut updated = original.clone();
        updated["models"][1]["name"] = serde_json::json!("future-model:2b");
        let catalogue = ModelCatalogue::parse(&serde_json::to_vec(&updated).unwrap()).unwrap();
        assert_eq!(catalogue.models()[1].name, "future-model:2b");
        for invalid in [
            "example.org/model:2b",
            "user/model:2b",
            "../model:2b",
            "model:latest",
            "model:2b-cloud",
            "model-cloud:2b",
            "model",
            "model:tag:extra",
            "model: tag",
            "model:\n2b",
            "model:",
            "model:..",
        ] {
            let mut document = original.clone();
            document["models"][1]["name"] = serde_json::json!(invalid);
            assert!(
                ModelCatalogue::parse(&serde_json::to_vec(&document).unwrap()).is_err(),
                "{invalid}"
            );
        }
        for (pointer, value) in [
            ("/version", serde_json::json!(2)),
            ("/models/0/name", serde_json::json!("other:1b")),
            ("/models/1/name", serde_json::json!("qwen3:0.6b")),
            ("/models/1/size", serde_json::json!(0)),
            ("/models/1/size", serde_json::json!(20_000_000_001_u64)),
        ] {
            let mut document = original.clone();
            *document.pointer_mut(pointer).unwrap() = value;
            assert!(ModelCatalogue::parse(&serde_json::to_vec(&document).unwrap()).is_err());
        }
        let mut document = original.clone();
        document["models"].as_array_mut().unwrap().pop();
        assert!(ModelCatalogue::parse(&serde_json::to_vec(&document).unwrap()).is_err());
        let mut document = original;
        document["models"][1]["command"] = serde_json::json!("anything");
        assert!(ModelCatalogue::parse(&serde_json::to_vec(&document).unwrap()).is_err());
        assert!(ModelCatalogue::parse(&vec![b' '; 16385]).is_err());
        assert!(ModelCatalogue::parse(b"invalid json").is_err());
    }

    #[test]
    fn download_catalogue_is_bounded_and_untrusted_targets_are_rejected() {
        let catalogue = ModelCatalogue::bundled();
        let suggested = catalogue.models();
        assert_eq!(suggested.len(), 6);
        assert_eq!(suggested[0].name, "qwen3:0.6b");
        let names: std::collections::BTreeSet<_> = suggested.iter().map(|m| &m.name).collect();
        assert_eq!(names.len(), suggested.len());
        for model in suggested {
            validate_model_name(&model.name).unwrap();
            assert!(model.size > 0);
        }
        assert!(
            pull_ollama_model(
                "https://example.com",
                "qwen3:4b",
                &ModelCatalogue::bundled(),
                |_| {}
            )
            .is_err()
        );
        assert!(
            pull_ollama_model(
                crate::DEFAULT_OLLAMA_URL,
                "user/unlisted:latest",
                &ModelCatalogue::bundled(),
                |_| {}
            )
            .is_err()
        );
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    fn request(stream: &mut std::net::TcpStream) -> (String, serde_json::Value) {
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut headers = Vec::new();
        while !headers.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            assert_eq!(stream.read(&mut byte).unwrap(), 1);
            headers.push(byte[0]);
        }
        let headers = String::from_utf8(headers).unwrap();
        let length: usize = headers
            .lines()
            .find_map(|line| {
                let (key, value) = line.split_once(':')?;
                key.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().unwrap())
            })
            .unwrap_or(0);
        let mut body = vec![0; length];
        stream.read_exact(&mut body).unwrap();
        (headers, serde_json::from_slice(&body).unwrap_or_default())
    }

    #[test]
    fn pull_stream_confirms_inventory_before_reporting_installation() {
        for listed in [true, false] {
            let mut document: serde_json::Value =
                serde_json::from_slice(include_bytes!("ollama-catalogue.json")).unwrap();
            document["models"][1]["name"] = serde_json::json!("future-model:2b");
            let catalogue = ModelCatalogue::parse(&serde_json::to_vec(&document).unwrap()).unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let server = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let (headers, body) = request(&mut stream);
                assert!(headers.starts_with("POST /api/pull "));
                assert_eq!(
                    body,
                    serde_json::json!({"model":"future-model:2b","stream":true})
                );
                let body = b"{\"status\":\"pulling manifest\"}\n{\"status\":\"pulling layer\",\"total\":100,\"completed\":25}\n{\"status\":\"success\"}\n";
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
                    body.len()
                )
                .unwrap();
                for piece in body.chunks(7) {
                    stream.write_all(piece).unwrap();
                }
                drop(stream);
                let (mut stream, _) = listener.accept().unwrap();
                let (headers, _) = request(&mut stream);
                assert!(headers.starts_with("GET /api/tags "));
                let body = if listed {
                    r#"{"models":[{"name":"future-model:2b","size":2500000000}]}"#
                } else {
                    r#"{"models":[]}"#
                };
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            });
            let mut seen = Vec::new();
            let models = pull_ollama_model(&url, "future-model:2b", &catalogue, |p| seen.push(p));
            if listed {
                assert_eq!(
                    models.unwrap(),
                    vec![InstalledModel {
                        name: "future-model:2b".into(),
                        size: Some(2_500_000_000)
                    }]
                );
            } else {
                assert!(models.unwrap_err().to_string().contains("did not list"));
            }
            assert!(seen.iter().any(|p| p.completed == Some(25)));
            server.join().unwrap();
        }
    }

    #[test]
    fn truncated_failed_and_oversized_streams_never_succeed() {
        for (status, body) in [
            ("200 OK", "{\"status\":\"pulling manifest\"}\n".to_string()),
            ("200 OK", "{\"error\":\"disk full\"}\n".to_string()),
            (
                "500 Internal Server Error",
                "{\"error\":\"registry unavailable\"}".to_string(),
            ),
            ("200 OK", "x".repeat(65537)),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let server = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                request(&mut stream);
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                );
            });
            assert!(
                pull_ollama_model(&url, "qwen3:4b", &ModelCatalogue::bundled(), |_| {}).is_err()
            );
            server.join().unwrap();
        }
    }

    fn respond(stream: &mut std::net::TcpStream, status: &str, body: &str) {
        write!(
            stream,
            "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
    }

    #[test]
    fn catalogue_http_rejects_missing_invalid_and_oversized_responses() {
        for (status, body, success) in [
            (
                "200 OK",
                include_str!("ollama-catalogue.json").to_string(),
                true,
            ),
            (
                "404 Not Found",
                include_str!("ollama-catalogue.json").to_string(),
                false,
            ),
            ("200 OK", "<html>unavailable</html>".to_string(), false),
            ("200 OK", " ".repeat(16385), false),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/catalogue.json", listener.local_addr().unwrap());
            let server = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                assert!(request(&mut stream).0.starts_with("GET /catalogue.json "));
                respond(&mut stream, status, &body);
            });
            assert_eq!(
                fetch_catalogue(client(Duration::from_secs(3)).unwrap().get(url)).is_ok(),
                success
            );
            server.join().unwrap();
        }
    }

    #[test]
    fn removal_requires_inventory_before_and_after_delete() {
        for (present, delete_status, remains) in [
            (true, "200 OK", false),
            (true, "200 OK", true),
            (true, "500 Internal Server Error", true),
            (false, "200 OK", false),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let server = thread::spawn(move || {
                let inventory = r#"{"models":[{"name":"custom:latest","size":123}]}"#;
                let empty = r#"{"models":[]}"#;
                let (mut stream, _) = listener.accept().unwrap();
                assert!(request(&mut stream).0.starts_with("GET /api/tags "));
                respond(
                    &mut stream,
                    "200 OK",
                    if present { inventory } else { empty },
                );
                drop(stream);
                if !present {
                    return;
                }
                let (mut stream, _) = listener.accept().unwrap();
                let (headers, body) = request(&mut stream);
                assert!(headers.starts_with("DELETE /api/delete "));
                assert_eq!(body, serde_json::json!({"model":"custom:latest"}));
                respond(
                    &mut stream,
                    delete_status,
                    if delete_status == "200 OK" {
                        ""
                    } else {
                        r#"{"error":"disk error"}"#
                    },
                );
                drop(stream);
                if delete_status != "200 OK" {
                    return;
                }
                let (mut stream, _) = listener.accept().unwrap();
                assert!(request(&mut stream).0.starts_with("GET /api/tags "));
                respond(
                    &mut stream,
                    "200 OK",
                    if remains { inventory } else { empty },
                );
            });
            let result = remove_ollama_model(&url, "custom:latest");
            assert_eq!(
                result.is_ok(),
                present && delete_status == "200 OK" && !remains
            );
            if let Ok(models) = result {
                assert!(models.is_empty());
            }
            server.join().unwrap();
        }
        assert!(remove_ollama_model("http://example.com", "qwen3:4b").is_err());
    }

    #[test]
    fn cancellation_closes_the_active_pull_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let token = peasy_core::cancellation::Cancellation::default();
        let worker_token = token.clone();
        let worker = thread::spawn(move || {
            worker_token
                .scope(|| pull_ollama_model(&url, "qwen3:4b", &ModelCatalogue::bundled(), |_| {}))
        });
        let (mut stream, _) = listener.accept().unwrap();
        request(&mut stream);
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10000\r\n\r\n{\"status\":\"pulling manifest\"}\n").unwrap();
        token.cancel();
        let error = worker.join().unwrap().unwrap_err();
        assert!(
            error
                .downcast_ref::<peasy_core::cancellation::Cancelled>()
                .is_some()
        );
        match stream.read(&mut [0]) {
            Ok(0) => {}
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
            other => panic!("cancelled pull connection remained open: {other:?}"),
        }
    }
}

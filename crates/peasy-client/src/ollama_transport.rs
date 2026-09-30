//! Keep complete instructions at the local model boundary. Never recover from a
//! context error by dropping messages, disabling the schema, or allowing shifts.
use crate::{http, safe_provider_error};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::time::Duration;

const CONTEXT_SIZES: [u32; 2] = [8192, 16384];

fn supports_context_controls(version: &str) -> bool {
    let parts: Vec<_> = version.split('.').collect();
    if parts.len() != 3 {
        return false;
    }
    match (
        parts[0].parse::<u32>(),
        parts[1].parse::<u32>(),
        parts[2].parse::<u32>(),
    ) {
        (Ok(major), Ok(minor), Ok(patch)) => (major, minor, patch) >= (0, 30, 6),
        _ => false,
    }
}

fn context_overflow(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    (message.contains("context length") || message.contains("context size"))
        && ["exceed", "longer", "too long", "too large", "full"]
            .iter()
            .any(|phrase| message.contains(phrase))
}

pub(super) fn chat(client: &reqwest::Client, base_url: &str, mut body: Value) -> Result<Value> {
    // Older servers silently ignore unknown request fields. Check on each turn
    // (cheap and no inference), including after a local server restart.
    let (status, bytes) = http::read(
        client
            .get(format!("{base_url}/api/version"))
            .timeout(Duration::from_secs(5)),
        4096,
    )
    .context("checking local Ollama version")?;
    let version: Value =
        serde_json::from_slice(&bytes).context("Ollama returned invalid version data")?;
    if !status.is_success()
        || !version["version"]
            .as_str()
            .is_some_and(supports_context_controls)
    {
        bail!(
            "Peasy requires Ollama 0.30.6 or newer to prevent silent prompt truncation. Update Ollama, restart its server, and retry."
        );
    }

    body["truncate"] = json!(false);
    body["shift"] = json!(false);
    body["think"] = json!(false);
    for context in CONTEXT_SIZES {
        body["options"]["num_ctx"] = json!(context);
        let (status, bytes) = http::read(
            client.post(format!("{base_url}/api/chat")).json(&body),
            256 * 1024,
        )
        .context("contacting local Ollama")?;
        let value: Value =
            serde_json::from_slice(&bytes).context("Ollama returned invalid JSON")?;
        // A non-streaming generation can fail after HTTP headers were sent.
        if !status.is_success() || value.get("error").is_some() {
            let message = value["error"].as_str().unwrap_or("Ollama request failed");
            if context_overflow(message) {
                if context != CONTEXT_SIZES[CONTEXT_SIZES.len() - 1] {
                    continue;
                }
                bail!(
                    "The complete Peasy request does not fit this Ollama model at {context} context tokens. No shortened prompt was accepted. Try a shorter request or another provider. Ollama: {}",
                    safe_provider_error(message)
                );
            }
            bail!("Ollama: {}", safe_provider_error(message));
        }
        if value["done"] != true {
            bail!(
                "Ollama returned an incomplete response. No action was accepted. Retry the request."
            );
        }
        if value["done_reason"] == "length" {
            bail!(
                "Ollama reached its response or context limit ({context} requested context tokens). No partial action was accepted. Try a shorter request or another model."
            );
        }
        if std::env::var_os("PEASY_OLLAMA_DIAGNOSTICS").as_deref()
            == Some(std::ffi::OsStr::new("1"))
        {
            // Counts only: never log prompts, configuration, or model output.
            eprintln!(
                "Ollama: model={} requested_context={context} prompt_tokens={} output_tokens={} load_ns={} prompt_ns={} generation_ns={}",
                body["model"],
                value["prompt_eval_count"],
                value["eval_count"],
                value["load_duration"],
                value["prompt_eval_duration"],
                value["eval_duration"]
            );
        }
        return Ok(value);
    }
    unreachable!("bounded context attempts always return")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::serve_http_responses;

    fn request() -> Value {
        json!({
            "model":"fixture", "stream":false,
            "messages":[
                {"role":"system", "content":"All permission constraints must remain intact."},
                {"role":"user", "content":"Install an application"}
            ],
            "format":{"type":"object", "additionalProperties":false},
            "options":{"temperature":0}
        })
    }

    fn complete() -> Value {
        json!({"done":true, "done_reason":"stop", "message":{"content":"{}"}})
    }

    fn version() -> (u16, Value) {
        (200, json!({"version":"0.33.1"}))
    }

    #[test]
    fn context_retry_preserves_the_complete_prompt_and_schema() {
        // Ollama can report a runtime error in an HTTP 200 response too.
        for status in [200, 400] {
            let (url, requests) = serve_http_responses(vec![
                version(),
                (
                    status,
                    json!({"error":"the prompt is longer than the context length currently available to the model"}),
                ),
                (200, complete()),
            ]);
            assert!(chat(&reqwest::Client::new(), &url, request()).is_ok());
            let (path, _) = requests.recv_timeout(Duration::from_secs(2)).unwrap();
            assert_eq!(path, "GET /api/version HTTP/1.1");
            for context in CONTEXT_SIZES {
                let (path, body) = requests.recv_timeout(Duration::from_secs(2)).unwrap();
                assert_eq!(path, "POST /api/chat HTTP/1.1");
                assert_eq!(body["messages"], request()["messages"]);
                assert_eq!(body["format"], request()["format"]);
                assert_eq!(body["options"]["num_ctx"], context);
                assert_eq!(body["options"]["temperature"], 0);
                for flag in ["truncate", "shift", "think", "stream"] {
                    assert_eq!(body[flag], false);
                }
            }
        }
    }

    #[test]
    fn context_growth_is_bounded_and_never_falls_back_to_truncation() {
        let (url, requests) = serve_http_responses(vec![
            version(),
            (400, json!({"error":"input exceeds the context size"})),
            (400, json!({"error":"input exceeds the context size"})),
        ]);
        let error = chat(&reqwest::Client::new(), &url, request())
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("16384") && error.contains("No shortened prompt"),
            "{error}"
        );
        assert_eq!(requests.try_iter().count(), 3);
    }

    #[test]
    fn old_or_unverifiable_servers_are_rejected_before_inference() {
        for value in [
            json!({"version":"0.30.5"}),
            json!({"version":"unknown"}),
            json!({}),
        ] {
            let (url, requests) = serve_http_responses(vec![(200, value)]);
            let error = chat(&reqwest::Client::new(), &url, request())
                .unwrap_err()
                .to_string();
            assert!(error.contains("0.30.6"), "{error}");
            let (path, _) = requests.recv_timeout(Duration::from_secs(2)).unwrap();
            assert_eq!(path, "GET /api/version HTTP/1.1");
            assert_eq!(requests.try_iter().count(), 0);
        }
        assert!(supports_context_controls("0.30.6"));
        assert!(supports_context_controls("0.33.2"));
        assert!(supports_context_controls("1.0.0"));
        assert!(!supports_context_controls("0.33.1-rc1"));
    }

    #[test]
    fn partial_output_and_runtime_errors_never_become_actions() {
        for (status, value, expected) in [
            (
                200,
                json!({"done":true, "done_reason":"length", "message":{"content":"{\"result\":{\"action\":\"cancel\"}}"}}),
                "No partial action",
            ),
            (
                200,
                json!({"done":false, "message":{"content":"{}"}}),
                "incomplete",
            ),
            (200, json!({"message":{"content":"{}"}}), "incomplete"),
            (
                200,
                json!({"error":"model failed to load"}),
                "model failed to load",
            ),
            (404, json!({"error":"model not found"}), "model not found"),
        ] {
            let (url, requests) = serve_http_responses(vec![version(), (status, value)]);
            let error = chat(&reqwest::Client::new(), &url, request())
                .unwrap_err()
                .to_string();
            assert!(error.contains(expected), "{error}");
            assert_eq!(requests.try_iter().count(), 2);
        }
    }
}

use super::*;
use crate::{http, ollama_transport, redacted_provider_error};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::time::Duration;

fn content(prompt: &str, attachments: &[Attachment]) -> Vec<Value> {
    let mut parts = vec![json!({"type":"input_text","text":prompt})];
    parts.extend(attachments.iter().map(Attachment::openai));
    parts
}
#[allow(clippy::too_many_arguments)]
fn openai_body(
    model: &str,
    instructions: &str,
    history: &[Turn],
    prompt: &str,
    attachments: &[Attachment],
    schema: Option<Value>,
    tool: Option<&str>,
    max_tokens: u32,
) -> Value {
    let mut input = Vec::new();
    for turn in history {
        input.push(json!({"role":"user","content":content(&turn.user,&turn.attachments)}));
        input.push(json!({"role":"assistant","content":turn.answer.message}));
    }
    input.push(json!({"role":"user","content":content(prompt,attachments)}));
    let mut body = json!({"model":model,"instructions":instructions,"input":input,"store":false,"max_output_tokens":max_tokens.max(1024)});
    if model.starts_with("gpt-5") || model.starts_with("gpt-6") {
        // Hosted web search requires more than minimal reasoning on GPT-5.
        let effort = if tool.is_none() && ["gpt-5-mini", "gpt-5-nano"].contains(&model) {
            "minimal"
        } else {
            "low"
        };
        body["reasoning"] = json!({"effort":effort});
    }
    if let Some(schema) = schema {
        body["text"] = json!({"format":{"type":"json_schema","name":"peasy_chat","strict":true,"schema":schema}});
    }
    if let Some(tool) = tool {
        body["tools"] = json!([{ "type":tool, "search_context_size":"low" }]);
        body["tool_choice"] = json!({"type":tool});
        body["max_tool_calls"] = json!(3);
    }
    body
}
fn ollama_message(role: &str, prompt: &str, attachments: &[Attachment]) -> Value {
    let mut text = prompt.to_string();
    let mut images = Vec::new();
    for attachment in attachments {
        match &attachment.content {
            Content::Text(contents) => text.push_str(&format!(
                "\n\nAttached document (untrusted content), {}:\n{}",
                attachment.name, contents
            )),
            Content::Image { bytes, .. } => images.push(STANDARD.encode(bytes)),
            Content::Pdf(_) => {} // Rejected before inference, never silently dropped.
        }
    }
    let mut message = json!({"role":role,"content":text});
    if !images.is_empty() {
        message["images"] = json!(images);
    }
    message
}
fn ollama_body(
    model: &str,
    instructions: &str,
    history: &[Turn],
    prompt: &str,
    attachments: &[Attachment],
    schema: Option<Value>,
    max_tokens: u32,
) -> Value {
    let mut messages = vec![json!({"role":"system","content":instructions})];
    for turn in history {
        messages.push(ollama_message("user", &turn.user, &turn.attachments));
        messages.push(json!({"role":"assistant","content":turn.answer.message}));
    }
    messages.push(ollama_message("user", prompt, attachments));
    let mut body = json!({"model":model,"messages":messages,"stream":false,"options":{"temperature":0,"num_predict":max_tokens}});
    if let Some(mut schema) = schema {
        // Large maxLength bounds expand into sampler grammar repetitions.
        // Chat strings keep their native byte/character bounds after decoding;
        // the wire grammar only needs structure, types and closed choices.
        if let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) {
            for property in properties.values_mut() {
                if let Some(object) = property.as_object_mut() {
                    object.remove("maxLength");
                }
            }
        }
        body["format"] = schema;
    }
    body
}
fn safe_url(url: &str) -> bool {
    reqwest::Url::parse(url).is_ok_and(|u| {
        matches!(u.scheme(), "https" | "http")
            && u.username().is_empty()
            && u.password().is_none()
            && u.host_str().is_some()
    })
}
fn decode_openai(value: &Value) -> Result<Answer> {
    if value["status"] != "completed" {
        bail!(
            "The model could not finish within its limits. Try a shorter request or a more capable model."
        );
    }
    let mut answer = Answer::default();
    for item in value["output"]
        .as_array()
        .context("OpenAI returned no output")?
    {
        if let Some(parts) = item["content"].as_array() {
            for part in parts {
                if part["type"] == "refusal" {
                    answer.message.push_str(
                        part["refusal"]
                            .as_str()
                            .unwrap_or("The provider declined this request."),
                    );
                }
                if part["type"] != "output_text" {
                    continue;
                }
                let mut text = part["text"].as_str().unwrap_or_default().to_string();
                if text.len() > 48000 {
                    bail!("The model returned too much text. Ask for a shorter answer.");
                }
                // Add visible inline source links using annotation offsets. Never
                // treat a source URL as a fetch instruction or an executable URI.
                let mut citations = Vec::new();
                for annotation in part["annotations"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .take(32)
                {
                    if annotation["type"] != "url_citation" {
                        continue;
                    }
                    let url = annotation["url"].as_str().unwrap_or_default();
                    if !safe_url(url) {
                        continue;
                    }
                    let title = annotation["title"]
                        .as_str()
                        .unwrap_or("Source")
                        .chars()
                        .filter(|c| !c.is_control())
                        .take(200)
                        .collect::<String>();
                    answer.sources.push((title, url.to_string()));
                    let end = annotation["end_index"].as_u64().unwrap_or(u64::MAX) as usize;
                    citations.push((end, answer.sources.len(), url.to_string()));
                }
                citations.sort_by_key(|c| std::cmp::Reverse(c.0));
                for (end, index, url) in citations {
                    let byte = text
                        .char_indices()
                        .nth(end)
                        .map(|(i, _)| i)
                        .unwrap_or(text.len());
                    let url = url.replace('(', "%28").replace(')', "%29");
                    text.insert_str(byte, &format!(" [{index}]({url})"));
                }
                if !answer.message.is_empty() {
                    answer.message.push_str("\n\n");
                }
                answer.message.push_str(&text);
            }
        }
    }
    if answer.message.len() > 48000 {
        bail!("The model returned too much text. Ask for a shorter answer.");
    }
    if answer.message.is_empty() {
        bail!("The model returned no answer. Try another model.");
    }
    Ok(answer)
}
impl ModelBackend {
    pub(super) fn check_attachments(&self, attachments: &[Attachment]) -> Result<()> {
        if let Self::Ollama(model) = self {
            if attachments
                .iter()
                .any(|a| matches!(a.content, Content::Pdf(_)))
            {
                bail!(peasy_core::i18n::tr(
                    "This local model cannot read PDF files here. Attach a text excerpt or choose OpenAI."
                ));
            }
            if attachments
                .iter()
                .any(|a| matches!(a.content, Content::Image { .. }))
            {
                let (status, bytes) = http::read(
                    model
                        .client
                        .post(format!("{}/api/show", model.base_url))
                        .timeout(Duration::from_secs(10))
                        .json(&json!({"model":model.model})),
                    1024 * 1024,
                )?;
                let value: Value = serde_json::from_slice(&bytes)?;
                if !status.is_success() {
                    bail!(
                        "Ollama could not check this model's image support. Refresh installed models in Settings."
                    );
                }
                if !value["capabilities"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|v| v == "vision"))
                {
                    bail!(peasy_core::i18n::tr(
                        "This model cannot read images. Choose a vision-capable model or OpenAI."
                    ));
                }
            }
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn chat_text(
        &self,
        instructions: &str,
        history: &[Turn],
        prompt: &str,
        attachments: &[Attachment],
        schema: Option<Value>,
        tool: Option<&str>,
        max_tokens: u32,
    ) -> Result<Answer> {
        match self {
            Self::OpenAi(model) => {
                let body = openai_body(
                    &model.model,
                    instructions,
                    history,
                    prompt,
                    attachments,
                    schema,
                    tool,
                    max_tokens,
                );
                let (status, bytes) = http::read(
                    model
                        .client
                        .post(crate::OPENAI_URL)
                        .bearer_auth(model.key.as_str())
                        .timeout(Duration::from_secs(120))
                        .json(&body),
                    256 * 1024,
                )?;
                let value: Value =
                    serde_json::from_slice(&bytes).context("OpenAI returned invalid JSON")?;
                if !status.is_success() {
                    let message = redacted_provider_error(
                        value
                            .pointer("/error/message")
                            .and_then(Value::as_str)
                            .unwrap_or("Request failed"),
                        model.key.as_str(),
                    );
                    let lower = message.to_ascii_lowercase();
                    if status.as_u16() == 400
                        && ["context", "not support", "unsupported"]
                            .iter()
                            .any(|s| lower.contains(s))
                    {
                        bail!(
                            "This model cannot handle this request or attachment. Try a shorter request or choose a model that supports this feature. OpenAI: {message}"
                        );
                    }
                    bail!("OpenAI ({status}): {message}");
                }
                decode_openai(&value)
            }
            Self::Ollama(model) => {
                if tool.is_some() {
                    bail!("This feature requires OpenAI. Choose it in Settings.");
                }
                let body = ollama_body(
                    &model.model,
                    instructions,
                    history,
                    prompt,
                    attachments,
                    schema,
                    max_tokens,
                );
                let value=ollama_transport::chat(&model.client,&model.base_url,body).map_err(|e|{
                    let text=format!("{e:#}");
                    if text.contains("context") || text.contains("partial action") {anyhow::anyhow!(peasy_core::i18n::tr("This request exceeds the model's limits. Try a shorter request, start a new conversation or choose a more capable model."))}else{e}
                })?;
                let message = value
                    .pointer("/message/content")
                    .and_then(Value::as_str)
                    .context("Ollama returned no answer")?;
                if message.is_empty() || message.len() > 48000 {
                    bail!("The model returned an empty or oversized answer. Try another model.");
                }
                Ok(Answer {
                    message: message.into(),
                    ..Default::default()
                })
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transports_preserve_roles_and_attachments_without_tools_for_ollama() {
        let attachment =
            Attachment::from_bytes("notes.txt".into(), b"test instructions".to_vec()).unwrap();
        let turn = Turn {
            user: "first".into(),
            attachments: vec![attachment],
            answer: Answer {
                message: "reply".into(),
                ..Default::default()
            },
        };
        let body = ollama_body(
            "fixture",
            "system",
            std::slice::from_ref(&turn),
            "followup",
            &[],
            Some(answer_schema()),
            100,
        );
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][2]["role"], "assistant");
        assert!(
            body["messages"][1]["content"]
                .as_str()
                .unwrap()
                .contains("test instructions")
        );
        assert!(body.get("tools").is_none());
        assert_eq!(body["stream"], false);
        assert!(
            body["format"]["properties"]["message"]
                .get("maxLength")
                .is_none()
        );
        let body = openai_body(
            "gpt-5-mini",
            "system",
            &[turn],
            "followup",
            &[],
            None,
            Some("web_search"),
            100,
        );
        assert_eq!(body["store"], false);
        assert_eq!(body["tool_choice"]["type"], "web_search");
        assert_eq!(body["reasoning"]["effort"], "low");
        assert_eq!(body["model"], "gpt-5-mini");
        assert_eq!(body["input"][1]["role"], "assistant");
    }
    #[test]
    fn image_analysis_uses_the_selected_chat_model_without_an_image_tool() {
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jD1sAAAAASUVORK5CYII=";
        let image =
            Attachment::from_bytes("pixel.png".into(), STANDARD.decode(png).unwrap()).unwrap();
        let body = openai_body(
            "selected-model",
            "instructions",
            &[],
            "describe this",
            &[image],
            Some(answer_schema()),
            None,
            2048,
        );
        assert_eq!(body["model"], "selected-model");
        assert!(body.get("tools").is_none());
        assert!(
            body["input"][0]["content"][1]["image_url"]
                .as_str()
                .unwrap()
                .starts_with("data:image/png;base64,")
        );
    }
    #[test]
    fn partial_answers_and_unsafe_citations_are_not_accepted() {
        assert!(decode_openai(&json!({"status":"incomplete","output":[]})).is_err());
        let answer=decode_openai(&json!({"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"A fact.","annotations":[{"type":"url_citation","url":"https://example.com","title":"Source","end_index":7},{"type":"url_citation","url":"file:///etc/passwd","end_index":7}]}]}]})).unwrap();
        assert_eq!(answer.sources.len(), 1);
        assert!(answer.message.contains("[1](https://example.com)"));
        assert!(!answer.message.contains("file:"));
    }
}
